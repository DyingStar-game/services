/**
 * Player profiles: creation on first contact, updates, search and game-server stats.
 */
import { and, eq, ilike, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { playerProfiles, type EntityType, type PlayerProfile, type RpSheet } from '../db/schema/index.js';
import { HttpError, conflict, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';

/** Fields a player may edit on their own profile. */
export interface ProfilePatch {
  displayName?: string;
  avatarUrl?: string | null;
  faction?: string | null;
  biography?: string | null;
  rpSheet?: RpSheet | null;
}

/** Fields accepted when creating/updating an NPC profile via the internal API. */
export interface NpcProfileInput {
  displayName: string;
  avatarUrl?: string | null;
  faction?: string | null;
  biography?: string | null;
  role?: string | null;
  level?: number;
}

/** Stat deltas / values reported by the game server (reputation goes through `reputation.service`). */
export interface StatsUpdate {
  playtimeSecondsDelta?: number;
  level?: number;
  role?: string | null;
}

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  // drizzle wraps driver errors in DrizzleQueryError; the pg error sits in `cause`.
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/**
 * Fetches a profile by player id.
 * @param playerId - Player id.
 * @returns Profile, or null if none.
 */
export async function getProfile(playerId: string): Promise<PlayerProfile | null> {
  const rows = await db.select().from(playerProfiles).where(eq(playerProfiles.playerId, playerId)).limit(1);
  return rows[0] ?? null;
}

/**
 * Fetches a profile or throws 404.
 * @param playerId - Player id.
 * @returns Profile.
 */
export async function requireProfile(playerId: string): Promise<PlayerProfile> {
  const profile = await getProfile(playerId);
  if (!profile) throw notFound(`Player ${playerId} not found`);
  return profile;
}

/**
 * Whether the profile is a server-managed NPC.
 * @param playerId - Profile id.
 * @returns True when the profile exists and is an NPC.
 */
export async function isNpc(playerId: string): Promise<boolean> {
  const profile = await getProfile(playerId);
  return profile !== null && profile.entityType === 'npc';
}

/**
 * Rejects the call when the target profile is an NPC.
 * @param playerId - Profile id.
 */
export async function requireNotNpc(playerId: string): Promise<void> {
  if (await isNpc(playerId)) {
    throw new HttpError(400, 'NPC_NOT_APPLICABLE', 'This action does not apply to NPC profiles');
  }
}

/**
 * Returns the existing profile or creates one from the token identity.
 * On display-name collision a short suffix is appended.
 * @param playerId - Player id.
 * @param username - Preferred display name.
 * @returns Existing or freshly created profile.
 */
export async function ensureProfile(playerId: string, username: string): Promise<PlayerProfile> {
  const existing = await getProfile(playerId);
  if (existing) return existing;

  for (let attempt = 0; attempt < 5; attempt++) {
    const displayName = attempt === 0 ? username : `${username}#${playerId.slice(0, 4 + attempt)}`;
    try {
      const [created] = await db.insert(playerProfiles).values({ playerId, displayName }).returning();
      await recordActivity(playerId, 'profile_created');
      return created;
    } catch (err) {
      if (!isUniqueViolation(err)) throw err;
      // Concurrent creation of the same player wins; name collision retries with a suffix.
      const raced = await getProfile(playerId);
      if (raced) return raced;
    }
  }
  throw conflict(`Could not allocate a unique display name for ${username}`);
}

/**
 * Creates an NPC profile (idempotent). Unlike players, a taken NPC name is an error:
 * a collision must not be silently renamed, since the shared unique `display_name`
 * protects real players from impersonation.
 * @param playerId - NPC id (UUID, assigned by the game server).
 * @param input - NPC profile fields.
 * @returns Existing or freshly created NPC profile.
 */
export async function ensureNpcProfile(playerId: string, input: NpcProfileInput): Promise<PlayerProfile> {
  const existing = await getProfile(playerId);
  if (existing) {
    if (existing.entityType === 'player') throw conflict('This id already belongs to a player profile');
    if (existing.displayName !== input.displayName) {
      throw conflict(`NPC display name "${input.displayName}" conflicts with existing profile`);
    }
    const patch: Partial<Omit<NpcProfileInput, 'displayName'>> = {
      ...(input.avatarUrl !== undefined ? { avatarUrl: input.avatarUrl } : {}),
      ...(input.faction !== undefined ? { faction: input.faction } : {}),
      ...(input.biography !== undefined ? { biography: input.biography } : {}),
      ...(input.role !== undefined ? { role: input.role } : {}),
      ...(input.level !== undefined ? { level: input.level } : {}),
    };
    if (Object.keys(patch).length === 0) return existing;
    const [updated] = await db
      .update(playerProfiles)
      .set({ ...patch, updatedAt: new Date() })
      .where(eq(playerProfiles.playerId, playerId))
      .returning();
    return updated;
  }
  try {
    const [created] = await db
      .insert(playerProfiles)
      .values({ playerId, entityType: 'npc', ...input })
      .returning();
    await recordActivity(playerId, 'npc_profile_created');
    return created;
  } catch (err) {
    if (!isUniqueViolation(err)) throw err;
    const raced = await getProfile(playerId);
    if (raced && raced.entityType === 'npc') return raced;
    throw conflict(`Display name "${input.displayName}" is already taken`);
  }
}

/**
 * Applies a player-initiated patch to their profile.
 * @param playerId - Player id.
 * @param patch - Editable fields.
 * @returns Updated profile.
 */
export async function updateProfile(playerId: string, patch: ProfilePatch): Promise<PlayerProfile> {
  await requireProfile(playerId);
  try {
    const [updated] = await db
      .update(playerProfiles)
      .set({ ...patch, updatedAt: new Date() })
      .where(eq(playerProfiles.playerId, playerId))
      .returning();
    await recordActivity(playerId, 'profile_updated', { fields: Object.keys(patch) });
    return updated;
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(`Display name "${patch.displayName}" is already taken`);
    throw err;
  }
}

/**
 * Searches profiles by display name (case-insensitive substring).
 * @param search - Substring to match; empty lists the most recent profiles.
 * @param limit - Max results.
 * @param entityType - Optional profile-kind filter (default: all).
 * @returns Matching profiles.
 */
export async function searchProfiles(search: string, limit: number, entityType?: EntityType): Promise<PlayerProfile[]> {
  const conditions = [];
  if (search) conditions.push(ilike(playerProfiles.displayName, `%${search}%`));
  if (entityType) conditions.push(eq(playerProfiles.entityType, entityType));
  const query = db.select().from(playerProfiles);
  const filtered = conditions.length ? query.where(and(...conditions)) : query;
  return filtered.orderBy(playerProfiles.displayName).limit(limit);
}

/**
 * Applies game-server reported stats (deltas for counters, absolute for level/role).
 * @param playerId - Player id.
 * @param stats - Stats update.
 * @returns Updated profile.
 */
export async function applyStats(playerId: string, stats: StatsUpdate): Promise<PlayerProfile> {
  await requireProfile(playerId);
  const [updated] = await db
    .update(playerProfiles)
    .set({
      playtimeSeconds: stats.playtimeSecondsDelta
        ? sql`${playerProfiles.playtimeSeconds} + ${stats.playtimeSecondsDelta}`
        : undefined,
      level: stats.level,
      role: stats.role,
      updatedAt: new Date(),
    })
    .where(eq(playerProfiles.playerId, playerId))
    .returning();
  if (stats.level !== undefined) {
    await recordActivity(playerId, 'level_changed', { level: stats.level });
  }
  return updated;
}
