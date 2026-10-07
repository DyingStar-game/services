/**
 * Player presence (online status + location) reported by the game server.
 */
import { inArray } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  playerPresence,
  type PlayerLocation,
  type PlayerPresence,
  type PresenceStatus,
} from '../db/schema/index.js';
import { requireProfile } from './profiles.service.js';

/**
 * Upserts the presence of a player.
 * @param playerId - Player id (profile must exist).
 * @param status - New status.
 * @param location - New location; `null` clears it, `undefined` keeps the previous one.
 * @returns Stored presence.
 */
export async function setPresence(
  playerId: string,
  status: PresenceStatus,
  location?: PlayerLocation | null,
): Promise<PlayerPresence> {
  await requireProfile(playerId);
  const now = new Date();
  const [row] = await db
    .insert(playerPresence)
    .values({ playerId, status, location: location ?? null, updatedAt: now })
    .onConflictDoUpdate({
      target: playerPresence.playerId,
      set: { status, updatedAt: now, ...(location !== undefined ? { location } : {}) },
    })
    .returning();
  return row;
}

/**
 * Fetches presence rows for several players, keyed by player id (missing = offline).
 * @param playerIds - Player ids.
 * @returns Map of presence by player id.
 */
export async function getPresenceMap(playerIds: string[]): Promise<Map<string, PlayerPresence>> {
  if (playerIds.length === 0) return new Map();
  const rows = await db.select().from(playerPresence).where(inArray(playerPresence.playerId, playerIds));
  return new Map(rows.map((r) => [r.playerId, r]));
}

/**
 * Returns the presence of one player, defaulting to offline.
 * @param playerId - Player id.
 * @returns Presence status and location.
 */
export async function getPresence(playerId: string): Promise<Pick<PlayerPresence, 'status' | 'location' | 'updatedAt'>> {
  const map = await getPresenceMap([playerId]);
  return map.get(playerId) ?? { status: 'offline', location: null, updatedAt: new Date(0) };
}
