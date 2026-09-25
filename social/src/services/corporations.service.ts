/**
 * Corporation lifecycle and membership: creation, public page, members, ranks assignment, ceo.
 */
import { and, count, desc, eq, ilike, inArray, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  CORPORATION_PERMISSIONS,
  corporationJoinRequests,
  corporationMembers,
  corporationRanks,
  corporations,
  playerProfiles,
  type Corporation,
  type CorporationMember,
  type CorporationPermission,
  type CorporationRank,
  type CorporationRecruitmentMode,
  type PlayerLocation,
  type PlayerProfile,
  type PresenceStatus,
} from '../db/schema/index.js';
import { HttpError, conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { recordCorporationActivity } from './corporationActivity.service.js';
import { getPresenceMap } from './presence.service.js';
import { isNpc, requireProfile } from './profiles.service.js';

/** Corporation fields editable by members with `manage_corporation`. */
export interface CorporationPatch {
  name?: string;
  ticker?: string;
  logoUrl?: string | null;
  description?: string | null;
  recruitment?: CorporationRecruitmentMode;
}

/** Corporation as listed publicly. */
export interface CorporationSummary extends Corporation {
  memberCount: number;
}

/** Minimal corporation reference embedded in profiles. */
export interface CorporationRef {
  id: string;
  name: string;
  ticker: string;
}

/** A member with profile, rank and presence. */
export interface CorporationMemberView extends PlayerProfile {
  joinedAt: Date;
  rank: CorporationRank;
  status: PresenceStatus;
  location: PlayerLocation | null;
}

/** The caller's membership context. */
export interface CorporationMembership {
  corporation: Corporation;
  member: CorporationMember;
  rank: CorporationRank;
}

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

function rethrowUnique(err: unknown, patch: { name?: string; ticker?: string }): never {
  if (isUniqueViolation(err)) {
    throw conflict(`Corporation name "${patch.name ?? ''}" or ticker "${patch.ticker ?? ''}" is already taken`);
  }
  throw err;
}

/**
 * Whether a rank grants a permission (the CEO rank grants all).
 * @param rank - Rank.
 * @param permission - Permission to check.
 * @returns True if granted.
 */
export function hasCorporationPermission(rank: CorporationRank, permission: CorporationPermission): boolean {
  return rank.isCeo || rank.permissions.includes(permission);
}

/**
 * Fetches a corporation or throws 404.
 * @param corporationId - Corporation id.
 * @returns Corporation.
 */
export async function requireCorporation(corporationId: string): Promise<Corporation> {
  const rows = await db.select().from(corporations).where(eq(corporations.id, corporationId)).limit(1);
  if (!rows[0]) throw notFound(`Corporation ${corporationId} not found`);
  return rows[0];
}

/**
 * Membership context of a player, or null if corporationless.
 * @param playerId - Player id.
 * @returns Corporation, member row and rank.
 */
export async function getCorporationMembership(playerId: string): Promise<CorporationMembership | null> {
  const rows = await db
    .select({ corporation: corporations, member: corporationMembers, rank: corporationRanks })
    .from(corporationMembers)
    .innerJoin(corporations, eq(corporations.id, corporationMembers.corporationId))
    .innerJoin(corporationRanks, eq(corporationRanks.id, corporationMembers.rankId))
    .where(eq(corporationMembers.playerId, playerId))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * Requires the player to be a member of the given corporation.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 * @returns Membership context.
 */
export async function requireCorporationMember(corporationId: string, playerId: string): Promise<CorporationMembership> {
  await requireCorporation(corporationId);
  const membership = await getCorporationMembership(playerId);
  if (!membership || membership.corporation.id !== corporationId) throw forbidden('Not a member of this corporation');
  return membership;
}

/**
 * Requires membership plus a permission.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 * @param permission - Required permission.
 * @returns Membership context.
 */
export async function requireCorporationPermission(
  corporationId: string,
  playerId: string,
  permission: CorporationPermission,
): Promise<CorporationMembership> {
  const membership = await requireCorporationMember(corporationId, playerId);
  if (!hasCorporationPermission(membership.rank, permission)) {
    throw forbidden(`Missing corporation permission: ${permission}`);
  }
  return membership;
}

/**
 * Corporation references for several players, keyed by player id.
 * @param playerIds - Player ids.
 * @returns Map of corporation refs.
 */
export async function getCorporationRefMap(playerIds: string[]): Promise<Map<string, CorporationRef>> {
  if (playerIds.length === 0) return new Map();
  const rows = await db
    .select({ playerId: corporationMembers.playerId, id: corporations.id, name: corporations.name, ticker: corporations.ticker })
    .from(corporationMembers)
    .innerJoin(corporations, eq(corporations.id, corporationMembers.corporationId))
    .where(inArray(corporationMembers.playerId, playerIds));
  return new Map(rows.map((r) => [r.playerId, { id: r.id, name: r.name, ticker: r.ticker }]));
}

/**
 * Lists corporations by name/ticker substring with member counts.
 * @param search - Substring (empty = all).
 * @param limit - Max results.
 * @returns Corporation summaries.
 */
export async function listCorporations(search: string, limit: number): Promise<CorporationSummary[]> {
  const memberCount = count(corporationMembers.playerId);
  const query = db
    .select({ corporation: corporations, memberCount })
    .from(corporations)
    .leftJoin(corporationMembers, eq(corporationMembers.corporationId, corporations.id))
    .groupBy(corporations.id);
  const filtered = search
    ? query.where(or(ilike(corporations.name, `%${search}%`), ilike(corporations.ticker, `%${search}%`)))
    : query;
  const rows = await filtered.orderBy(desc(memberCount), corporations.name).limit(limit);
  return rows.map((r) => ({ ...r.corporation, memberCount: r.memberCount }));
}

/**
 * Creates a corporation with default ranks (CEO / Director / Member) and the creator as CEO.
 * @param ceoId - Creating player (must be corporationless).
 * @param data - Corporation fields.
 * @returns Created corporation.
 */
export async function createCorporation(
  ceoId: string,
  data: { name: string; ticker: string; description?: string | null; logoUrl?: string | null; recruitment?: CorporationRecruitmentMode },
): Promise<Corporation> {
  await requireProfile(ceoId);
  if (await getCorporationMembership(ceoId)) throw conflict('Already a member of a corporation');

  try {
    const corporation = await db.transaction(async (tx) => {
      const [corporation] = await tx
        .insert(corporations)
        .values({ ...data, ceoId })
        .returning();
      const ranks = await tx
        .insert(corporationRanks)
        .values([
          { corporationId: corporation.id, name: 'CEO', priority: 100, permissions: [...CORPORATION_PERMISSIONS], isCeo: true },
          { corporationId: corporation.id, name: 'Director', priority: 50, permissions: ['invite', 'recruit', 'manage_members'] },
          { corporationId: corporation.id, name: 'Member', priority: 0, permissions: [], isDefault: true },
        ])
        .returning();
      const ceoRank = ranks.find((r) => r.isCeo)!;
      await tx.insert(corporationMembers).values({ corporationId: corporation.id, playerId: ceoId, rankId: ceoRank.id });
      await recordCorporationActivity(corporation.id, ceoId, 'corporation_created', undefined, tx);
      return corporation;
    });
    await recordActivity(ceoId, 'corporation_created', { corporationId: corporation.id, name: corporation.name });
    return corporation;
  } catch (err) {
    rethrowUnique(err, data);
  }
}

/**
 * Updates corporation fields (requires `manage_corporation`).
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 * @param patch - Fields to change.
 * @returns Updated corporation.
 */
export async function updateCorporation(corporationId: string, actorId: string, patch: CorporationPatch): Promise<Corporation> {
  await requireCorporationPermission(corporationId, actorId, 'manage_corporation');
  try {
    const [updated] = await db
      .update(corporations)
      .set({ ...patch, updatedAt: new Date() })
      .where(eq(corporations.id, corporationId))
      .returning();
    await recordCorporationActivity(corporationId, actorId, 'corporation_updated', { fields: Object.keys(patch) });
    return updated;
  } catch (err) {
    rethrowUnique(err, patch);
  }
}

/**
 * Deletes a corporation and everything attached (CEO only).
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 */
export async function disbandCorporation(corporationId: string, actorId: string): Promise<void> {
  const corporation = await requireCorporation(corporationId);
  if (corporation.ceoId !== actorId) throw forbidden('Only the CEO can disband the corporation');
  const members = await db
    .select({ playerId: corporationMembers.playerId })
    .from(corporationMembers)
    .where(eq(corporationMembers.corporationId, corporationId));
  await db.delete(corporations).where(eq(corporations.id, corporationId));
  await Promise.all(
    members.map((m) => recordActivity(m.playerId, 'corporation_disbanded', { corporationId, name: corporation.name })),
  );
}

/**
 * Public corporation page: corporation, member count, ranks and members.
 * @param corporationId - Corporation id.
 * @returns Corporation page payload.
 */
export async function getCorporationPage(
  corporationId: string,
): Promise<CorporationSummary & { ranks: CorporationRank[]; members: CorporationMemberView[] }> {
  const corporation = await requireCorporation(corporationId);
  const [ranks, members] = await Promise.all([listCorporationRanks(corporationId), listCorporationMembers(corporationId)]);
  return { ...corporation, memberCount: members.length, ranks, members };
}

/**
 * Ranks of a corporation, highest priority first.
 * @param corporationId - Corporation id.
 * @returns Ranks.
 */
export async function listCorporationRanks(corporationId: string): Promise<CorporationRank[]> {
  return db
    .select()
    .from(corporationRanks)
    .where(eq(corporationRanks.corporationId, corporationId))
    .orderBy(desc(corporationRanks.priority), corporationRanks.id);
}

/**
 * Members of a corporation with profile, rank and presence (highest rank first).
 * @param corporationId - Corporation id.
 * @returns Members.
 */
export async function listCorporationMembers(corporationId: string): Promise<CorporationMemberView[]> {
  const rows = await db
    .select({ profile: playerProfiles, member: corporationMembers, rank: corporationRanks })
    .from(corporationMembers)
    .innerJoin(playerProfiles, eq(playerProfiles.playerId, corporationMembers.playerId))
    .innerJoin(corporationRanks, eq(corporationRanks.id, corporationMembers.rankId))
    .where(eq(corporationMembers.corporationId, corporationId))
    .orderBy(desc(corporationRanks.priority), corporationMembers.joinedAt);
  const presence = await getPresenceMap(rows.map((r) => r.profile.playerId));
  return rows.map((r) => {
    const p = presence.get(r.profile.playerId);
    return { ...r.profile, joinedAt: r.member.joinedAt, rank: r.rank, status: p?.status ?? 'offline', location: p?.location ?? null };
  });
}

async function requireMemberRow(
  corporationId: string,
  playerId: string,
): Promise<{ member: CorporationMember; rank: CorporationRank }> {
  const rows = await db
    .select({ member: corporationMembers, rank: corporationRanks })
    .from(corporationMembers)
    .innerJoin(corporationRanks, eq(corporationRanks.id, corporationMembers.rankId))
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)))
    .limit(1);
  if (!rows[0]) throw notFound(`Player ${playerId} is not a member of this corporation`);
  return rows[0];
}

/**
 * Adds a player to a corporation with the default rank (used by join/invitation flows).
 * Clears any pending join requests of that player.
 * @param corporationId - Corporation id.
 * @param playerId - Joining player.
 * @param actorId - Player responsible for the join (self, recruiter or inviter).
 */
export async function addCorporationMember(corporationId: string, playerId: string, actorId: string): Promise<void> {
  if (await getCorporationMembership(playerId)) throw conflict('Player is already a member of a corporation');
  const [defaultRank] = await db
    .select()
    .from(corporationRanks)
    .where(and(eq(corporationRanks.corporationId, corporationId), eq(corporationRanks.isDefault, true)))
    .limit(1);
  if (!defaultRank) throw new HttpError(500, 'INTERNAL_ERROR', 'Corporation has no default rank');
  await db.transaction(async (tx) => {
    await tx.insert(corporationMembers).values({ corporationId, playerId, rankId: defaultRank.id });
    await tx.delete(corporationJoinRequests).where(eq(corporationJoinRequests.playerId, playerId));
    await recordCorporationActivity(corporationId, actorId, 'member_joined', { playerId }, tx);
  });
  await recordActivity(playerId, 'corporation_joined', { corporationId });
}

/**
 * Adds an NPC to a corporation (game server via the internal API). The NPC must already exist
 * and can never be the CEO or hold the CEO rank. Uses the given rank, or the default rank.
 * @param corporationId - Corporation id.
 * @param playerId - NPC profile id.
 * @param rankId - Optional rank to assign.
 * @returns The created membership row.
 */
export async function addNpcCorporationMember(corporationId: string, playerId: string, rankId?: number): Promise<CorporationMember> {
  const corporation = await requireCorporation(corporationId);
  await requireProfile(playerId);
  if (!(await isNpc(playerId))) throw new HttpError(400, 'NOT_AN_NPC', 'Only NPC profiles can be added this way');
  if (playerId === corporation.ceoId) throw forbidden('An NPC cannot be the corporation CEO');
  if (await getCorporationMembership(playerId)) throw conflict('NPC is already a member of a corporation');

  let chosenRank: CorporationRank;
  if (rankId !== undefined) {
    const [rank] = await db
      .select()
      .from(corporationRanks)
      .where(and(eq(corporationRanks.id, rankId), eq(corporationRanks.corporationId, corporationId)))
      .limit(1);
    if (!rank) throw notFound(`Rank ${rankId} not found`);
    if (rank.isCeo) throw forbidden('Cannot assign the CEO rank to an NPC');
    chosenRank = rank;
  } else {
    const [defaultRank] = await db
      .select()
      .from(corporationRanks)
      .where(and(eq(corporationRanks.corporationId, corporationId), eq(corporationRanks.isDefault, true)))
      .limit(1);
    if (!defaultRank) throw new HttpError(500, 'INTERNAL_ERROR', 'Corporation has no default rank');
    chosenRank = defaultRank;
  }

  const member = await db.transaction(async (tx) => {
    const [m] = await tx
      .insert(corporationMembers)
      .values({ corporationId, playerId, rankId: chosenRank.id })
      .returning();
    await recordCorporationActivity(corporationId, null, 'npc_added', { playerId, rank: chosenRank.name }, tx);
    return m;
  });
  await recordActivity(playerId, 'corporation_joined', { corporationId });
  return member;
}

/**
 * Removes an NPC from a corporation (game server via the internal API).
 * @param corporationId - Corporation id.
 * @param playerId - NPC profile id.
 */
export async function removeNpcCorporationMember(corporationId: string, playerId: string): Promise<void> {
  await requireCorporation(corporationId);
  await requireProfile(playerId);
  if (!(await isNpc(playerId))) throw new HttpError(400, 'NOT_AN_NPC', 'Only NPC profiles can be removed this way');
  await requireMemberRow(corporationId, playerId);
  const deleted = await db
    .delete(corporationMembers)
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)))
    .returning({ playerId: corporationMembers.playerId });
  if (deleted.length === 0) return;
  await recordCorporationActivity(corporationId, null, 'npc_removed', { playerId });
  await recordActivity(playerId, 'corporation_left', { corporationId });
}

/**
 * Changes a member's rank. Actor needs `manage_members`, must outrank both the target and the new rank.
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 * @param playerId - Target member.
 * @param rankId - New rank id.
 * @returns Updated member row.
 */
export async function setCorporationMemberRank(
  corporationId: string,
  actorId: string,
  playerId: string,
  rankId: number,
): Promise<CorporationMember> {
  const actor = await requireCorporationPermission(corporationId, actorId, 'manage_members');
  const target = await requireMemberRow(corporationId, playerId);
  if (playerId === actorId) throw forbidden('Cannot change your own rank');
  if (target.rank.priority >= actor.rank.priority) throw forbidden('Cannot manage a member of equal or higher rank');
  const [rank] = await db
    .select()
    .from(corporationRanks)
    .where(and(eq(corporationRanks.id, rankId), eq(corporationRanks.corporationId, corporationId)))
    .limit(1);
  if (!rank) throw notFound(`Rank ${rankId} not found`);
  if (rank.isCeo) throw forbidden('Use the CEO transfer to assign the CEO rank');
  if (rank.priority >= actor.rank.priority) throw forbidden('Cannot assign a rank equal or higher than your own');
  const [updated] = await db
    .update(corporationMembers)
    .set({ rankId })
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)))
    .returning();
  await recordCorporationActivity(corporationId, actorId, 'rank_changed', { playerId, from: target.rank.name, to: rank.name });
  await recordActivity(playerId, 'corporation_rank_changed', { corporationId, rank: rank.name });
  return updated;
}

/**
 * Removes a member: self-leave, or kick with `manage_members` on a lower-ranked member.
 * The CEO cannot leave without transferring leadership.
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 * @param playerId - Member to remove.
 */
export async function removeCorporationMember(corporationId: string, actorId: string, playerId: string): Promise<void> {
  const corporation = await requireCorporation(corporationId);
  const target = await requireMemberRow(corporationId, playerId);
  if (playerId === corporation.ceoId) throw forbidden('The CEO must transfer leadership before leaving');
  if (playerId !== actorId) {
    const actor = await requireCorporationPermission(corporationId, actorId, 'manage_members');
    if (target.rank.priority >= actor.rank.priority) throw forbidden('Cannot kick a member of equal or higher rank');
  }
  await db
    .delete(corporationMembers)
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)));
  const kicked = playerId !== actorId;
  await recordCorporationActivity(corporationId, actorId, kicked ? 'member_kicked' : 'member_left', { playerId });
  await recordActivity(playerId, kicked ? 'corporation_kicked' : 'corporation_left', { corporationId });
}

/**
 * Transfers the CEO: new CEO gets the CEO rank, former CEO gets the highest non-CEO rank.
 * @param corporationId - Corporation id.
 * @param actorId - Current CEO.
 * @param playerId - New CEO (must be a member).
 * @returns Updated corporation.
 */
export async function transferCorporationCeo(corporationId: string, actorId: string, playerId: string): Promise<Corporation> {
  const corporation = await requireCorporation(corporationId);
  if (corporation.ceoId !== actorId) throw forbidden('Only the CEO can transfer leadership');
  if (playerId === actorId) throw new HttpError(400, 'INVALID_TARGET', 'Already the CEO');
  await requireMemberRow(corporationId, playerId);
  const ranks = await listCorporationRanks(corporationId);
  const ceoRank = ranks.find((r) => r.isCeo)!;
  const fallback = ranks.find((r) => !r.isCeo) ?? ranks.find((r) => r.isDefault)!;
  const updated = await db.transaction(async (tx) => {
    await tx
      .update(corporationMembers)
      .set({ rankId: ceoRank.id })
      .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)));
    await tx
      .update(corporationMembers)
      .set({ rankId: fallback.id })
      .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, actorId)));
    const [c] = await tx
      .update(corporations)
      .set({ ceoId: playerId, updatedAt: new Date() })
      .where(eq(corporations.id, corporationId))
      .returning();
    await recordCorporationActivity(corporationId, actorId, 'ceo_transferred', { to: playerId }, tx);
    return c;
  });
  await recordActivity(playerId, 'corporation_ceo_received', { corporationId });
  return updated;
}