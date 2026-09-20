/**
 * Guild lifecycle and membership: creation, public page, members, ranks assignment, ownership.
 */
import { and, count, desc, eq, ilike, inArray, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  GUILD_PERMISSIONS,
  guildJoinRequests,
  guildMembers,
  guildRanks,
  guilds,
  playerProfiles,
  type Guild,
  type GuildMember,
  type GuildPermission,
  type GuildRank,
  type GuildRecruitmentMode,
  type PlayerLocation,
  type PlayerProfile,
  type PresenceStatus,
} from '../db/schema/index.js';
import { HttpError, conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { recordGuildActivity } from './guildActivity.service.js';
import { getPresenceMap } from './presence.service.js';
import { requireProfile } from './profiles.service.js';

/** Guild fields editable by members with `manage_guild`. */
export interface GuildPatch {
  name?: string;
  tag?: string;
  logoUrl?: string | null;
  description?: string | null;
  recruitment?: GuildRecruitmentMode;
}

/** Guild as listed publicly. */
export interface GuildSummary extends Guild {
  memberCount: number;
}

/** Minimal guild reference embedded in profiles. */
export interface GuildRef {
  id: string;
  name: string;
  tag: string;
}

/** A member with profile, rank and presence. */
export interface GuildMemberView extends PlayerProfile {
  joinedAt: Date;
  rank: GuildRank;
  status: PresenceStatus;
  location: PlayerLocation | null;
}

/** The caller's membership context. */
export interface Membership {
  guild: Guild;
  member: GuildMember;
  rank: GuildRank;
}

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

function rethrowUnique(err: unknown, patch: { name?: string; tag?: string }): never {
  if (isUniqueViolation(err)) {
    throw conflict(`Guild name "${patch.name ?? ''}" or tag "${patch.tag ?? ''}" is already taken`);
  }
  throw err;
}

/**
 * Whether a rank grants a permission (the leader rank grants all).
 * @param rank - Rank.
 * @param permission - Permission to check.
 * @returns True if granted.
 */
export function hasPermission(rank: GuildRank, permission: GuildPermission): boolean {
  return rank.isLeader || rank.permissions.includes(permission);
}

/**
 * Fetches a guild or throws 404.
 * @param guildId - Guild id.
 * @returns Guild.
 */
export async function requireGuild(guildId: string): Promise<Guild> {
  const rows = await db.select().from(guilds).where(eq(guilds.id, guildId)).limit(1);
  if (!rows[0]) throw notFound(`Guild ${guildId} not found`);
  return rows[0];
}

/**
 * Membership context of a player, or null if guildless.
 * @param playerId - Player id.
 * @returns Guild, member row and rank.
 */
export async function getMembership(playerId: string): Promise<Membership | null> {
  const rows = await db
    .select({ guild: guilds, member: guildMembers, rank: guildRanks })
    .from(guildMembers)
    .innerJoin(guilds, eq(guilds.id, guildMembers.guildId))
    .innerJoin(guildRanks, eq(guildRanks.id, guildMembers.rankId))
    .where(eq(guildMembers.playerId, playerId))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * Requires the player to be a member of the given guild.
 * @param guildId - Guild id.
 * @param playerId - Player id.
 * @returns Membership context.
 */
export async function requireMember(guildId: string, playerId: string): Promise<Membership> {
  await requireGuild(guildId);
  const membership = await getMembership(playerId);
  if (!membership || membership.guild.id !== guildId) throw forbidden('Not a member of this guild');
  return membership;
}

/**
 * Requires membership plus a permission.
 * @param guildId - Guild id.
 * @param playerId - Player id.
 * @param permission - Required permission.
 * @returns Membership context.
 */
export async function requirePermission(
  guildId: string,
  playerId: string,
  permission: GuildPermission,
): Promise<Membership> {
  const membership = await requireMember(guildId, playerId);
  if (!hasPermission(membership.rank, permission)) {
    throw forbidden(`Missing guild permission: ${permission}`);
  }
  return membership;
}

/**
 * Guild references for several players, keyed by player id.
 * @param playerIds - Player ids.
 * @returns Map of guild refs.
 */
export async function getGuildRefMap(playerIds: string[]): Promise<Map<string, GuildRef>> {
  if (playerIds.length === 0) return new Map();
  const rows = await db
    .select({ playerId: guildMembers.playerId, id: guilds.id, name: guilds.name, tag: guilds.tag })
    .from(guildMembers)
    .innerJoin(guilds, eq(guilds.id, guildMembers.guildId))
    .where(inArray(guildMembers.playerId, playerIds));
  return new Map(rows.map((r) => [r.playerId, { id: r.id, name: r.name, tag: r.tag }]));
}

/**
 * Lists guilds by name/tag substring with member counts.
 * @param search - Substring (empty = all).
 * @param limit - Max results.
 * @returns Guild summaries.
 */
export async function listGuilds(search: string, limit: number): Promise<GuildSummary[]> {
  const memberCount = count(guildMembers.playerId);
  const query = db
    .select({ guild: guilds, memberCount })
    .from(guilds)
    .leftJoin(guildMembers, eq(guildMembers.guildId, guilds.id))
    .groupBy(guilds.id);
  const filtered = search
    ? query.where(or(ilike(guilds.name, `%${search}%`), ilike(guilds.tag, `%${search}%`)))
    : query;
  const rows = await filtered.orderBy(desc(memberCount), guilds.name).limit(limit);
  return rows.map((r) => ({ ...r.guild, memberCount: r.memberCount }));
}

/**
 * Creates a guild with default ranks (Leader / Officer / Member) and the creator as owner.
 * @param ownerId - Creating player (must be guildless).
 * @param data - Guild fields.
 * @returns Created guild.
 */
export async function createGuild(
  ownerId: string,
  data: { name: string; tag: string; description?: string | null; logoUrl?: string | null; recruitment?: GuildRecruitmentMode },
): Promise<Guild> {
  await requireProfile(ownerId);
  if (await getMembership(ownerId)) throw conflict('Already a member of a guild');

  try {
    const guild = await db.transaction(async (tx) => {
      const [guild] = await tx.insert(guilds).values({ ...data, ownerId }).returning();
      const ranks = await tx
        .insert(guildRanks)
        .values([
          { guildId: guild.id, name: 'Leader', priority: 100, permissions: [...GUILD_PERMISSIONS], isLeader: true },
          { guildId: guild.id, name: 'Officer', priority: 50, permissions: ['invite', 'recruit', 'manage_members'] },
          { guildId: guild.id, name: 'Member', priority: 0, permissions: [], isDefault: true },
        ])
        .returning();
      const leader = ranks.find((r) => r.isLeader)!;
      await tx.insert(guildMembers).values({ guildId: guild.id, playerId: ownerId, rankId: leader.id });
      await recordGuildActivity(guild.id, ownerId, 'guild_created', undefined, tx);
      return guild;
    });
    await recordActivity(ownerId, 'guild_created', { guildId: guild.id, name: guild.name });
    return guild;
  } catch (err) {
    rethrowUnique(err, data);
  }
}

/**
 * Updates guild fields (requires `manage_guild`).
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 * @param patch - Fields to change.
 * @returns Updated guild.
 */
export async function updateGuild(guildId: string, actorId: string, patch: GuildPatch): Promise<Guild> {
  await requirePermission(guildId, actorId, 'manage_guild');
  try {
    const [updated] = await db
      .update(guilds)
      .set({ ...patch, updatedAt: new Date() })
      .where(eq(guilds.id, guildId))
      .returning();
    await recordGuildActivity(guildId, actorId, 'guild_updated', { fields: Object.keys(patch) });
    return updated;
  } catch (err) {
    rethrowUnique(err, patch);
  }
}

/**
 * Deletes a guild and everything attached (owner only).
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 */
export async function disbandGuild(guildId: string, actorId: string): Promise<void> {
  const guild = await requireGuild(guildId);
  if (guild.ownerId !== actorId) throw forbidden('Only the owner can disband the guild');
  const members = await db
    .select({ playerId: guildMembers.playerId })
    .from(guildMembers)
    .where(eq(guildMembers.guildId, guildId));
  await db.delete(guilds).where(eq(guilds.id, guildId));
  await Promise.all(
    members.map((m) => recordActivity(m.playerId, 'guild_disbanded', { guildId, name: guild.name })),
  );
}

/**
 * Public guild page: guild, member count, ranks and members.
 * @param guildId - Guild id.
 * @returns Guild page payload.
 */
export async function getGuildPage(guildId: string): Promise<GuildSummary & { ranks: GuildRank[]; members: GuildMemberView[] }> {
  const guild = await requireGuild(guildId);
  const [ranks, members] = await Promise.all([listRanks(guildId), listMembers(guildId)]);
  return { ...guild, memberCount: members.length, ranks, members };
}

/**
 * Ranks of a guild, highest priority first.
 * @param guildId - Guild id.
 * @returns Ranks.
 */
export async function listRanks(guildId: string): Promise<GuildRank[]> {
  return db.select().from(guildRanks).where(eq(guildRanks.guildId, guildId)).orderBy(desc(guildRanks.priority), guildRanks.id);
}

/**
 * Members of a guild with profile, rank and presence (highest rank first).
 * @param guildId - Guild id.
 * @returns Members.
 */
export async function listMembers(guildId: string): Promise<GuildMemberView[]> {
  const rows = await db
    .select({ profile: playerProfiles, member: guildMembers, rank: guildRanks })
    .from(guildMembers)
    .innerJoin(playerProfiles, eq(playerProfiles.playerId, guildMembers.playerId))
    .innerJoin(guildRanks, eq(guildRanks.id, guildMembers.rankId))
    .where(eq(guildMembers.guildId, guildId))
    .orderBy(desc(guildRanks.priority), guildMembers.joinedAt);
  const presence = await getPresenceMap(rows.map((r) => r.profile.playerId));
  return rows.map((r) => {
    const p = presence.get(r.profile.playerId);
    return { ...r.profile, joinedAt: r.member.joinedAt, rank: r.rank, status: p?.status ?? 'offline', location: p?.location ?? null };
  });
}

async function requireMemberRow(guildId: string, playerId: string): Promise<{ member: GuildMember; rank: GuildRank }> {
  const rows = await db
    .select({ member: guildMembers, rank: guildRanks })
    .from(guildMembers)
    .innerJoin(guildRanks, eq(guildRanks.id, guildMembers.rankId))
    .where(and(eq(guildMembers.guildId, guildId), eq(guildMembers.playerId, playerId)))
    .limit(1);
  if (!rows[0]) throw notFound(`Player ${playerId} is not a member of this guild`);
  return rows[0];
}

/**
 * Adds a player to a guild with the default rank (used by join/invitation flows).
 * Clears any pending join requests of that player.
 * @param guildId - Guild id.
 * @param playerId - Joining player.
 * @param actorId - Player responsible for the join (self, recruiter or inviter).
 */
export async function addMember(guildId: string, playerId: string, actorId: string): Promise<void> {
  if (await getMembership(playerId)) throw conflict('Player is already a member of a guild');
  const [defaultRank] = await db
    .select()
    .from(guildRanks)
    .where(and(eq(guildRanks.guildId, guildId), eq(guildRanks.isDefault, true)))
    .limit(1);
  if (!defaultRank) throw new HttpError(500, 'INTERNAL_ERROR', 'Guild has no default rank');
  await db.transaction(async (tx) => {
    await tx.insert(guildMembers).values({ guildId, playerId, rankId: defaultRank.id });
    await tx.delete(guildJoinRequests).where(eq(guildJoinRequests.playerId, playerId));
    await recordGuildActivity(guildId, actorId, 'member_joined', { playerId }, tx);
  });
  await recordActivity(playerId, 'guild_joined', { guildId });
}

/**
 * Changes a member's rank. Actor needs `manage_members`, must outrank both the target and the new rank.
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 * @param playerId - Target member.
 * @param rankId - New rank id.
 * @returns Updated member row.
 */
export async function setMemberRank(guildId: string, actorId: string, playerId: string, rankId: number): Promise<GuildMember> {
  const actor = await requirePermission(guildId, actorId, 'manage_members');
  const target = await requireMemberRow(guildId, playerId);
  if (playerId === actorId) throw forbidden('Cannot change your own rank');
  if (target.rank.priority >= actor.rank.priority) throw forbidden('Cannot manage a member of equal or higher rank');
  const [rank] = await db.select().from(guildRanks).where(and(eq(guildRanks.id, rankId), eq(guildRanks.guildId, guildId))).limit(1);
  if (!rank) throw notFound(`Rank ${rankId} not found`);
  if (rank.isLeader) throw forbidden('Use ownership transfer to assign the leader rank');
  if (rank.priority >= actor.rank.priority) throw forbidden('Cannot assign a rank equal or higher than your own');
  const [updated] = await db
    .update(guildMembers)
    .set({ rankId })
    .where(and(eq(guildMembers.guildId, guildId), eq(guildMembers.playerId, playerId)))
    .returning();
  await recordGuildActivity(guildId, actorId, 'rank_changed', { playerId, from: target.rank.name, to: rank.name });
  await recordActivity(playerId, 'guild_rank_changed', { guildId, rank: rank.name });
  return updated;
}

/**
 * Removes a member: self-leave, or kick with `manage_members` on a lower-ranked member.
 * The owner cannot leave without transferring ownership.
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 * @param playerId - Member to remove.
 */
export async function removeMember(guildId: string, actorId: string, playerId: string): Promise<void> {
  const guild = await requireGuild(guildId);
  const target = await requireMemberRow(guildId, playerId);
  if (playerId === guild.ownerId) throw forbidden('The owner must transfer ownership before leaving');
  if (playerId !== actorId) {
    const actor = await requirePermission(guildId, actorId, 'manage_members');
    if (target.rank.priority >= actor.rank.priority) throw forbidden('Cannot kick a member of equal or higher rank');
  }
  await db.delete(guildMembers).where(and(eq(guildMembers.guildId, guildId), eq(guildMembers.playerId, playerId)));
  const kicked = playerId !== actorId;
  await recordGuildActivity(guildId, actorId, kicked ? 'member_kicked' : 'member_left', { playerId });
  await recordActivity(playerId, kicked ? 'guild_kicked' : 'guild_left', { guildId });
}

/**
 * Transfers ownership: new owner gets the leader rank, former owner gets the highest non-leader rank.
 * @param guildId - Guild id.
 * @param actorId - Current owner.
 * @param playerId - New owner (must be a member).
 * @returns Updated guild.
 */
export async function transferOwnership(guildId: string, actorId: string, playerId: string): Promise<Guild> {
  const guild = await requireGuild(guildId);
  if (guild.ownerId !== actorId) throw forbidden('Only the owner can transfer ownership');
  if (playerId === actorId) throw new HttpError(400, 'INVALID_TARGET', 'Already the owner');
  await requireMemberRow(guildId, playerId);
  const ranks = await listRanks(guildId);
  const leader = ranks.find((r) => r.isLeader)!;
  const fallback = ranks.find((r) => !r.isLeader) ?? ranks.find((r) => r.isDefault)!;
  const updated = await db.transaction(async (tx) => {
    await tx.update(guildMembers).set({ rankId: leader.id }).where(and(eq(guildMembers.guildId, guildId), eq(guildMembers.playerId, playerId)));
    await tx.update(guildMembers).set({ rankId: fallback.id }).where(and(eq(guildMembers.guildId, guildId), eq(guildMembers.playerId, actorId)));
    const [g] = await tx.update(guilds).set({ ownerId: playerId, updatedAt: new Date() }).where(eq(guilds.id, guildId)).returning();
    await recordGuildActivity(guildId, actorId, 'ownership_transferred', { to: playerId }, tx);
    return g;
  });
  await recordActivity(playerId, 'guild_ownership_received', { guildId });
  return updated;
}
