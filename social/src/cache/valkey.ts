/**
 * Valkey cache client: TTL store for live presence, accumulator for playtime deltas.
 *
 * The cache is optional — an empty `VALKEY_URL` disables it, and any connection failure
 * degrades to the database through `withCache`'s fallback (the API never fails because
 * the cache is unreachable). Valkey runs without persistence: losing it costs at most
 * one playtime flush interval and a presence re-warm from the next heartbeat.
 */
import { Redis } from 'ioredis';

import { env } from '../config/env.js';

/** Key prefixes shared by the services reading/writing the cache. */
export const PRESENCE_KEY_PREFIX = 'presence:';
export const PLAYTIME_KEY_PREFIX = 'playtime:';

let client: Redis | null = null;
let loggedErrorAt = 0;

/** Whether the cache is configured (may still be unreachable). */
export function cacheConfigured(): boolean {
  return env.valkey.url !== '';
}

function logCacheError(err: unknown): void {
  const now = Date.now();
  if (now - loggedErrorAt < 30_000) return; // the client retries in a loop: don't flood the logs
  loggedErrorAt = now;
  console.error('Valkey unavailable, using database:', err instanceof Error ? err.message : err);
}

/**
 * Creates the client when configured. Never throws: a missing Valkey must not stop startup.
 * @returns True when a cache client exists.
 */
export function initCache(): boolean {
  if (!cacheConfigured()) return false;
  client = new Redis(env.valkey.url, {
    lazyConnect: true,
    connectTimeout: 2_000,
    maxRetriesPerRequest: 1,
    enableOfflineQueue: false,
  });
  client.on('error', logCacheError);
  return true;
}

/** Connects the client (best effort) and logs the outcome. @returns True when ready. */
export async function connectCache(): Promise<boolean> {
  if (!client) return false;
  try {
    await client.connect();
    return true;
  } catch (err) {
    logCacheError(err); // reconnection keeps running in the background
    return false;
  }
}

/** @returns Pong when the cache answers, false otherwise (also false when disabled). */
export async function pingCache(): Promise<boolean> {
  if (!client || client.status !== 'ready') return false;
  try {
    return (await client.ping()) === 'PONG';
  } catch {
    return false;
  }
}

/** Closes the client (used by the graceful shutdown before exiting). */
export async function closeCache(): Promise<void> {
  if (!client) return;
  try {
    await client.quit();
  } catch {
    client.disconnect();
  }
}

/**
 * Runs a cache operation, falling back when the cache is disabled or unreachable.
 * `fallback` is also where writes go: a failed cache write becomes a database write.
 * @param run - Cache operation (only called when ready).
 * @param fallback - Database path used when the cache cannot answer.
 * @returns The cache result, or the fallback's.
 */
export async function withCache<T>(run: (redis: Redis) => Promise<T>, fallback: () => T | Promise<T>): Promise<T> {
  if (!client || client.status !== 'ready') return fallback();
  try {
    return await run(client);
  } catch (err) {
    logCacheError(err);
    return fallback();
  }
}
