/**
 * "Recently met" counters, recorded symmetrically for both players.
 */
import { desc, eq, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { playerEncounters, type PlayerEncounter } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { requireProfile } from './profiles.service.js';

/**
 * Records that two players met (increments both directions).
 * @param playerId - First player.
 * @param otherId - Second player.
 */
export async function recordEncounter(playerId: string, otherId: string): Promise<void> {
  if (playerId === otherId) throw new HttpError(400, 'INVALID_TARGET', 'A player cannot meet themselves');
  await Promise.all([requireProfile(playerId), requireProfile(otherId)]);
  const now = new Date();
  await db
    .insert(playerEncounters)
    .values([
      { playerId, otherId, lastMetAt: now },
      { playerId: otherId, otherId: playerId, lastMetAt: now },
    ])
    .onConflictDoUpdate({
      target: [playerEncounters.playerId, playerEncounters.otherId],
      set: { count: sql`${playerEncounters.count} + 1`, lastMetAt: now },
    });
}

/**
 * Most recent encounters of a player.
 * @param playerId - Player id.
 * @param limit - Max rows.
 * @returns Encounters, most recent first.
 */
export async function listRecentEncounters(playerId: string, limit: number): Promise<PlayerEncounter[]> {
  return db
    .select()
    .from(playerEncounters)
    .where(eq(playerEncounters.playerId, playerId))
    .orderBy(desc(playerEncounters.lastMetAt))
    .limit(limit);
}
