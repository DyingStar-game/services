/**
 * Corporation rank management (requires `manage_ranks`). The CEO rank is immutable except for its name.
 */
import { and, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { corporationMembers, corporationRanks, type CorporationPermission, type CorporationRank } from '../db/schema/index.js';
import { conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordCorporationActivity } from './corporationActivity.service.js';
import { requireCorporationPermission } from './corporations.service.js';

/** Rank fields. */
export interface CorporationRankInput {
  name: string;
  priority: number;
  permissions: CorporationPermission[];
  isDefault?: boolean;
}

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === '23505';
}

async function requireRank(corporationId: string, rankId: number): Promise<CorporationRank> {
  const [rank] = await db
    .select()
    .from(corporationRanks)
    .where(and(eq(corporationRanks.id, rankId), eq(corporationRanks.corporationId, corporationId)))
    .limit(1);
  if (!rank) throw notFound(`Rank ${rankId} not found`);
  return rank;
}

/**
 * Creates a rank below the actor's own priority.
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 * @param input - Rank fields.
 * @returns Created rank.
 */
export async function createCorporationRank(corporationId: string, actorId: string, input: CorporationRankInput): Promise<CorporationRank> {
  const actor = await requireCorporationPermission(corporationId, actorId, 'manage_ranks');
  if (input.priority >= actor.rank.priority) throw forbidden('Rank priority must be below your own');
  try {
    return await db.transaction(async (tx) => {
      if (input.isDefault) {
        await tx.update(corporationRanks).set({ isDefault: false }).where(eq(corporationRanks.corporationId, corporationId));
      }
      const [rank] = await tx
        .insert(corporationRanks)
        .values({ corporationId, ...input, isDefault: input.isDefault ?? false })
        .returning();
      await recordCorporationActivity(corporationId, actorId, 'rank_created', { rank: rank.name }, tx);
      return rank;
    });
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(`Rank "${input.name}" already exists`);
    throw err;
  }
}

/**
 * Updates a rank. The CEO rank only accepts a name change; the actor must outrank the rank.
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 * @param rankId - Rank id.
 * @param patch - Fields to change.
 * @returns Updated rank.
 */
export async function updateCorporationRank(
  corporationId: string,
  actorId: string,
  rankId: number,
  patch: Partial<CorporationRankInput>,
): Promise<CorporationRank> {
  const actor = await requireCorporationPermission(corporationId, actorId, 'manage_ranks');
  const rank = await requireRank(corporationId, rankId);
  if (rank.isCeo) {
    if (patch.priority !== undefined || patch.permissions !== undefined || patch.isDefault) {
      throw forbidden('Only the name of the CEO rank can be changed');
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
        await tx.update(corporationRanks).set({ isDefault: false }).where(eq(corporationRanks.corporationId, corporationId));
      }
      const [updated] = await tx.update(corporationRanks).set(patch).where(eq(corporationRanks.id, rankId)).returning();
      await recordCorporationActivity(corporationId, actorId, 'rank_updated', { rank: updated.name, fields: Object.keys(patch) }, tx);
      return updated;
    });
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(`Rank "${patch.name}" already exists`);
    throw err;
  }
}

/**
 * Deletes a rank; its members are moved to the default rank. CEO and default ranks cannot be deleted.
 * @param corporationId - Corporation id.
 * @param actorId - Acting player.
 * @param rankId - Rank id.
 */
export async function deleteCorporationRank(corporationId: string, actorId: string, rankId: number): Promise<void> {
  const actor = await requireCorporationPermission(corporationId, actorId, 'manage_ranks');
  const rank = await requireRank(corporationId, rankId);
  if (rank.isCeo) throw forbidden('The CEO rank cannot be deleted');
  if (rank.isDefault) throw forbidden('The default rank cannot be deleted; set another default first');
  if (rank.priority >= actor.rank.priority) throw forbidden('Cannot delete a rank equal or higher than your own');
  const [defaultRank] = await db
    .select()
    .from(corporationRanks)
    .where(and(eq(corporationRanks.corporationId, corporationId), eq(corporationRanks.isDefault, true)))
    .limit(1);
  await db.transaction(async (tx) => {
    await tx.update(corporationMembers).set({ rankId: defaultRank.id }).where(eq(corporationMembers.rankId, rankId));
    await tx.delete(corporationRanks).where(eq(corporationRanks.id, rankId));
    await recordCorporationActivity(corporationId, actorId, 'rank_deleted', { rank: rank.name }, tx);
  });
}