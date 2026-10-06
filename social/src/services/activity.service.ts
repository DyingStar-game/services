/**
 * Player activity history (persisted in `player_activity`).
 */
import { count, desc, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { page, type Page } from '../lib/pagination.js';
import { playerActivity, type PlayerActivityEntry } from '../db/schema/index.js';

/**
 * Appends an activity entry for a player.
 * @param playerId - Player id.
 * @param type - Event type (e.g. `friend_request_sent`, `profile_updated`).
 * @param details - Optional structured payload.
 */
export async function recordActivity(
  playerId: string,
  type: string,
  details?: Record<string, unknown>,
): Promise<void> {
  await db.insert(playerActivity).values({ playerId, type, details });
}

/**
 * Returns the most recent activity entries for a player (newest first).
 * @param playerId - Player id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of activity entries.
 */
export async function listActivity(playerId: string, limit: number, offset: number): Promise<Page<PlayerActivityEntry>> {
  const condition = eq(playerActivity.playerId, playerId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(playerActivity).where(condition),
    db
      .select()
      .from(playerActivity)
      .where(condition)
      .orderBy(desc(playerActivity.createdAt), desc(playerActivity.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(rows, totalRow[0].total, limit, offset);
}
