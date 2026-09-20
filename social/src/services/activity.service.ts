/**
 * Player activity history (persisted in `player_activity`).
 */
import { desc, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
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
 * @param limit - Max entries.
 * @returns Activity entries.
 */
export async function listActivity(playerId: string, limit: number): Promise<PlayerActivityEntry[]> {
  return db
    .select()
    .from(playerActivity)
    .where(eq(playerActivity.playerId, playerId))
    .orderBy(desc(playerActivity.createdAt), desc(playerActivity.id))
    .limit(limit);
}
