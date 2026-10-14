/**
 * Player state store: batch-upserted by the game server via
 * `PUT /api/internal/missions/player-states`. Used for zone matching and physical
 * objective evaluation (visit, delivery, transport, passenger).
 *
 * TTL: evaluations use the latest state; a state older than 30 s is treated as
 * "outside every zone" (fail-closed).
 */
import { sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { missionPlayerStates } from '../db/schema/index.js';

/** One player state entry pushed by the game server. */
export interface PlayerStateInput {
  playerId: string;
  place?: { system?: string; scene?: string; x?: number; y?: number; z?: number } | null;
  onFoot?: boolean;
  passenger?: boolean;
  carried?: Record<string, number>;
}

/**
 * Upserts a batch of player states. Each state overwrites the previous one entirely
 * (last-write-wins).
 * @param players - Player states to upsert.
 * @returns The number of rows upserted.
 */
export async function upsertPlayerStates(players: PlayerStateInput[]): Promise<{ count: number }> {
  if (players.length === 0) return { count: 0 };
  const now = new Date();
  const rows = players.map((p) => ({
    playerId: p.playerId,
    place: p.place ?? null,
    onFoot: p.onFoot ?? true,
    passenger: p.passenger ?? false,
    carried: p.carried ?? {},
    receivedAt: now,
  }));
  await db
    .insert(missionPlayerStates)
    .values(rows)
    .onConflictDoUpdate({
      target: missionPlayerStates.playerId,
      set: {
        place: sql`excluded.place`,
        onFoot: sql`excluded.on_foot`,
        passenger: sql`excluded.passenger`,
        carried: sql`excluded.carried`,
        receivedAt: sql`excluded.received_at`,
      },
    });
  return { count: rows.length };
}
