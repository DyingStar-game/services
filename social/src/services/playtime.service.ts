/**
 * Playtime accumulation in the cache: game-server deltas are `INCRBY`'d on every stats
 * call and flushed to PostgreSQL in one pass every few minutes, so a busy server costs
 * no database write per tick.
 *
 * Valkey runs without persistence: a restart loses at most the deltas of the current
 * flush interval (one interval by design). The flush uses `GETDEL` per key — atomic
 * take-and-delete — so a player active during the flush is not double-counted; a failed
 * database write puts the delta back for the next pass.
 */
import { eq, sql } from 'drizzle-orm';

import { cacheConfigured, PLAYTIME_KEY_PREFIX, withCache } from '../cache/valkey.js';
import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import { playerProfiles, type PlayerProfile } from '../db/schema/index.js';

function playtimeKey(playerId: string): string {
  return `${PLAYTIME_KEY_PREFIX}${playerId}`;
}

/** Applies a delta directly (cache unreachable): the legacy write path. */
async function addPlaytimeToDatabase(playerId: string, delta: number): Promise<void> {
  await db
    .update(playerProfiles)
    .set({ playtimeSeconds: sql`${playerProfiles.playtimeSeconds} + ${delta}` })
    .where(eq(playerProfiles.playerId, playerId));
}

/**
 * Adds a playtime delta: cached when possible, written straight to the database when
 * the cache is unreachable (never blocks the caller, never loses the delta silently).
 * @param playerId - Player id (profile must exist).
 * @param delta - Seconds to add (positive integers only).
 */
export async function addPlaytimeDelta(playerId: string, delta: number): Promise<void> {
  if (!Number.isInteger(delta) || delta <= 0) return;
  await withCache(
    (redis) => redis.incrby(playtimeKey(playerId), delta),
    async () => {
      await addPlaytimeToDatabase(playerId, delta);
      return null;
    },
  );
}

/**
 * Profile with the not-yet-flushed cached delta added to `playtimeSeconds`.
 * No-op (returned as-is) when the cache is disabled or holds nothing for the player.
 * @param profile - Profile (or any `{ playerId, playtimeSeconds }` shape).
 * @returns The profile with the live playtime.
 */
export async function withLivePlaytime<T extends Pick<PlayerProfile, 'playerId' | 'playtimeSeconds'>>(
  profile: T,
): Promise<T> {
  const raw = await withCache((redis) => redis.get(playtimeKey(profile.playerId)), () => null);
  const pending = raw ? Number(raw) : 0;
  if (!Number.isFinite(pending) || pending === 0) return profile;
  return { ...profile, playtimeSeconds: profile.playtimeSeconds + pending };
}

/**
 * Flushes every cached playtime delta to the database.
 * `GETDEL` removes the delta from the cache before writing it: a crash after the delete
 * loses at most that player's delta (same bound as a Valkey restart), a failure writes
 * the delta back so the next pass retries it.
 * @returns Total seconds flushed.
 */
export async function flushPlaytimeDeltas(): Promise<number> {
  return withCache(async (redis) => {
    const keys: string[] = [];
    let cursor = '0';
    do {
      const [next, batch] = await redis.scan(cursor, 'MATCH', `${PLAYTIME_KEY_PREFIX}*`, 'COUNT', 500);
      cursor = next;
      keys.push(...batch);
    } while (cursor !== '0');
    if (keys.length === 0) return 0;

    const pipeline = redis.pipeline();
    for (const key of keys) pipeline.getdel(key);
    const results = await pipeline.exec();

    let total = 0;
    const updates: Promise<void>[] = [];
    results?.forEach(([err, value], index) => {
      if (err) {
        console.error(`Playtime flush: GETDEL ${keys[index]} failed:`, err);
        return;
      }
      const delta = Number(value ?? 0);
      if (!Number.isFinite(delta) || delta === 0) return;
      total += delta;
      updates.push(flushOneDelta(keys[index].slice(PLAYTIME_KEY_PREFIX.length), delta));
    });
    await Promise.all(updates);
    return total;
  }, () => 0);
}

/** Writes one delta to the database; on failure, puts it back in the cache for a retry. */
async function flushOneDelta(playerId: string, delta: number): Promise<void> {
  try {
    await addPlaytimeToDatabase(playerId, delta);
  } catch (err) {
    console.error(`Playtime flush failed for ${playerId} (${delta}s), kept for next pass:`, err);
    await withCache((redis) => redis.incrby(playtimeKey(playerId), delta), () => null);
  }
}

/**
 * Starts the periodic flush when a cache is configured and the interval is > 0.
 * @returns Timer handle or null when disabled.
 */
export function startPlaytimeFlushScheduler(): NodeJS.Timeout | null {
  const minutes = env.valkey.playtimeFlushIntervalMinutes;
  if (minutes <= 0 || !cacheConfigured()) return null;
  const timer = setInterval(() => {
    flushPlaytimeDeltas()
      .then((total) => {
        if (total > 0) console.log(`Flushed ${total}s of cached playtime to the database`);
      })
      .catch((err) => console.error('Playtime flush pass failed:', err));
  }, minutes * 60_000);
  timer.unref();
  return timer;
}
