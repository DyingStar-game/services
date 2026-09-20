/**
 * Guild rank management (requires `manage_ranks`). The leader rank is immutable except for its name.
 */
import { and, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { guildMembers, guildRanks, type GuildPermission, type GuildRank } from '../db/schema/index.js';
import { conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordGuildActivity } from './guildActivity.service.js';
import { requirePermission } from './guilds.service.js';

/** Rank fields. */
export interface RankInput {
  name: string;
  priority: number;
  permissions: GuildPermission[];
  isDefault?: boolean;
}

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === '23505';
}

async function requireRank(guildId: string, rankId: number): Promise<GuildRank> {
  const [rank] = await db
    .select()
    .from(guildRanks)
    .where(and(eq(guildRanks.id, rankId), eq(guildRanks.guildId, guildId)))
    .limit(1);
  if (!rank) throw notFound(`Rank ${rankId} not found`);
  return rank;
}

/**
 * Creates a rank below the actor's own priority.
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 * @param input - Rank fields.
 * @returns Created rank.
 */
export async function createRank(guildId: string, actorId: string, input: RankInput): Promise<GuildRank> {
  const actor = await requirePermission(guildId, actorId, 'manage_ranks');
  if (input.priority >= actor.rank.priority) throw forbidden('Rank priority must be below your own');
  try {
    return await db.transaction(async (tx) => {
      if (input.isDefault) {
        await tx.update(guildRanks).set({ isDefault: false }).where(eq(guildRanks.guildId, guildId));
      }
      const [rank] = await tx.insert(guildRanks).values({ guildId, ...input, isDefault: input.isDefault ?? false }).returning();
      await recordGuildActivity(guildId, actorId, 'rank_created', { rank: rank.name }, tx);
      return rank;
    });
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(`Rank "${input.name}" already exists`);
    throw err;
  }
}

/**
 * Updates a rank. The leader rank only accepts a name change; the actor must outrank the rank.
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 * @param rankId - Rank id.
 * @param patch - Fields to change.
 * @returns Updated rank.
 */
export async function updateRank(
  guildId: string,
  actorId: string,
  rankId: number,
  patch: Partial<RankInput>,
): Promise<GuildRank> {
  const actor = await requirePermission(guildId, actorId, 'manage_ranks');
  const rank = await requireRank(guildId, rankId);
  if (rank.isLeader) {
    if (patch.priority !== undefined || patch.permissions !== undefined || patch.isDefault) {
      throw forbidden('Only the name of the leader rank can be changed');
    }
  } else {
    if (rank.priority >= actor.rank.priority) throw forbidden('Cannot edit a rank equal or higher than your own');
    if (patch.priority !== undefined && patch.priority >= actor.rank.priority) {
      throw forbidden('Rank priority must be below your own');
    }
  }
  if (patch.isDefault === false && rank.isDefault) {
    throw conflict('Set another rank as default instead of unsetting this one');
  }
  try {
    return await db.transaction(async (tx) => {
      if (patch.isDefault) {
        await tx.update(guildRanks).set({ isDefault: false }).where(eq(guildRanks.guildId, guildId));
      }
      const [updated] = await tx.update(guildRanks).set(patch).where(eq(guildRanks.id, rankId)).returning();
      await recordGuildActivity(guildId, actorId, 'rank_updated', { rank: updated.name, fields: Object.keys(patch) }, tx);
      return updated;
    });
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(`Rank "${patch.name}" already exists`);
    throw err;
  }
}

/**
 * Deletes a rank; its members are moved to the default rank. Leader and default ranks cannot be deleted.
 * @param guildId - Guild id.
 * @param actorId - Acting player.
 * @param rankId - Rank id.
 */
export async function deleteRank(guildId: string, actorId: string, rankId: number): Promise<void> {
  const actor = await requirePermission(guildId, actorId, 'manage_ranks');
  const rank = await requireRank(guildId, rankId);
  if (rank.isLeader) throw forbidden('The leader rank cannot be deleted');
  if (rank.isDefault) throw forbidden('The default rank cannot be deleted; set another default first');
  if (rank.priority >= actor.rank.priority) throw forbidden('Cannot delete a rank equal or higher than your own');
  const [defaultRank] = await db
    .select()
    .from(guildRanks)
    .where(and(eq(guildRanks.guildId, guildId), eq(guildRanks.isDefault, true)))
    .limit(1);
  await db.transaction(async (tx) => {
    await tx.update(guildMembers).set({ rankId: defaultRank.id }).where(eq(guildMembers.rankId, rankId));
    await tx.delete(guildRanks).where(eq(guildRanks.id, rankId));
    await recordGuildActivity(guildId, actorId, 'rank_deleted', { rank: rank.name }, tx);
  });
}
