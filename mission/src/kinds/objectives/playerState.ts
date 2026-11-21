/**
 * Shared player-state lookup for the physical objective kinds.
 *
 * Batch measurement (the auto-measure triggered by player-state pushes) preloads every
 * state of the batch in **one** SELECT and hands the map through `MeasureContext`.
 * `loadPlayerState()` reads it and falls back to the database on a miss — a passenger's
 * state is usually not in the batch — so a single-objective measurement behaves exactly
 * as it did before.
 */
import { eq } from 'drizzle-orm';

import { db } from '../../db/connection.js';
import { missionPlayerStates } from '../../db/schema/index.js';
import type { MeasureContext, PrefetchedPlayerState } from '../types.js';

/** Player-state TTL in seconds: a state older than this counts as "outside every zone". */
export const STATE_TTL_MS = 30_000;

/** Whether a state is fresh enough to be trusted (never older than {@link STATE_TTL_MS}). */
export function isStateFresh(state: { receivedAt: Date }): boolean {
  return Date.now() - state.receivedAt.getTime() <= STATE_TTL_MS;
}

/**
 * Reads the pushed state of a holder, preferring the batch cache.
 * @param ctx - Measure context, carrying the optional batch cache.
 * @param playerId - Holder whose state is read.
 * @returns The state row, or null when the holder never pushed one.
 */
export async function loadPlayerState(
  ctx: MeasureContext,
  playerId: string,
): Promise<PrefetchedPlayerState | null> {
  if (ctx.stateCache) {
    const cached = ctx.stateCache.get(playerId);
    // `undefined` = miss (read the database); `null` = known absent.
    if (cached !== undefined) return cached;
  }
  const [state] = await db
    .select()
    .from(missionPlayerStates)
    .where(eq(missionPlayerStates.playerId, playerId))
    .limit(1);
  return state ?? null;
}

/** Converts a player-state `place` into the shape `zoneMatches` understands. */
export function stateLocation(
  state: PrefetchedPlayerState,
): { system?: string; scene?: string; position: { x: number; y: number; z: number } | null } {
  const place = state.place;
  return {
    system: place?.system,
    scene: place?.scene,
    position:
      place?.x !== undefined && place?.y !== undefined && place?.z !== undefined
        ? { x: place.x, y: place.y, z: place.z }
        : null,
  };
}
