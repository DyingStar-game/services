/**
 * Player block list. Blocking drops any friendship/request between the two players.
 */
import { and, eq, inArray, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { friendships, playerBlocks, playerProfiles, type PlayerProfile } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { env } from '../config/env.js';
import { recordActivity } from './activity.service.js';
import { requireProfile } from './profiles.service.js';
import { adjustReputation } from './reputation.service.js';

/**
 * Whether either player has blocked the other.
 * @param a - Player id.
 * @param b - Player id.
 * @returns True if a block exists in any direction.
 */
export async function isBlockedEitherWay(a: string, b: string): Promise<boolean> {
  const rows = await db
    .select({ blockerId: playerBlocks.blockerId })
    .from(playerBlocks)
    .where(
      or(
        and(eq(playerBlocks.blockerId, a), eq(playerBlocks.blockedId, b)),
        and(eq(playerBlocks.blockerId, b), eq(playerBlocks.blockedId, a)),
      ),
    )
    .limit(1);
  return rows.length > 0;
}

/**
 * Ids of every player involved in a block with `playerId` (either direction).
 * @param playerId - Player id.
 * @returns Set of other player ids.
 */
export async function blockedIdsFor(playerId: string): Promise<Set<string>> {
  const rows = await db
    .select({ blockerId: playerBlocks.blockerId, blockedId: playerBlocks.blockedId })
    .from(playerBlocks)
    .where(or(eq(playerBlocks.blockerId, playerId), eq(playerBlocks.blockedId, playerId)));
  return new Set(rows.map((r) => (r.blockerId === playerId ? r.blockedId : r.blockerId)));
}

/**
 * Lists profiles blocked by a player.
 * @param playerId - Blocker id.
 * @returns Blocked profiles with block date.
 */
export async function listBlocks(playerId: string): Promise<Array<PlayerProfile & { blockedAt: Date }>> {
  const rows = await db
    .select({ profile: playerProfiles, blockedAt: playerBlocks.createdAt })
    .from(playerBlocks)
    .innerJoin(playerProfiles, eq(playerProfiles.playerId, playerBlocks.blockedId))
    .where(eq(playerBlocks.blockerId, playerId))
    .orderBy(playerBlocks.createdAt);
  return rows.map((r) => ({ ...r.profile, blockedAt: r.blockedAt }));
}

/**
 * Blocks a player and removes any friendship or pending request between them.
 * @param blockerId - Player doing the blocking.
 * @param blockedId - Player being blocked.
 */
export async function blockPlayer(blockerId: string, blockedId: string): Promise<void> {
  if (blockerId === blockedId) throw new HttpError(400, 'INVALID_TARGET', 'Cannot block yourself');
  await requireProfile(blockedId);
  const inserted = await db.transaction(async (tx) => {
    const rows = await tx.insert(playerBlocks).values({ blockerId, blockedId }).onConflictDoNothing().returning();
    await tx
      .delete(friendships)
      .where(
        and(
          inArray(friendships.requesterId, [blockerId, blockedId]),
          inArray(friendships.addresseeId, [blockerId, blockedId]),
        ),
      );
    return rows.length > 0;
  });
  if (!inserted) return;
  await recordActivity(blockerId, 'player_blocked', { playerId: blockedId });
  await adjustReputation(blockedId, -env.reputation.blockPenalty, 'block', 'Blocked by a player', { actorId: blockerId });
}

/**
 * Removes a block.
 * @param blockerId - Player who blocked.
 * @param blockedId - Player to unblock.
 * @returns True if a block was removed.
 */
export async function unblockPlayer(blockerId: string, blockedId: string): Promise<boolean> {
  const deleted = await db
    .delete(playerBlocks)
    .where(and(eq(playerBlocks.blockerId, blockerId), eq(playerBlocks.blockedId, blockedId)))
    .returning({ blockedId: playerBlocks.blockedId });
  if (deleted.length > 0) {
    await recordActivity(blockerId, 'player_unblocked', { playerId: blockedId });
    await adjustReputation(blockedId, env.reputation.blockPenalty, 'block', 'Unblocked by a player', { actorId: blockerId });
  }
  return deleted.length > 0;
}
