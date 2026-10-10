/**
 * Player presence (online status + location) reported by the game server.
 *
 * The cache (Valkey) is the live source of truth: heartbeats (single call or batch) only
 * refresh a TTL key, so a player who stops heartbeating expires back to `offline`
 * without any game-server event. PostgreSQL is written when the status changes — or on
 * every heartbeat while the cache is unreachable (graceful degradation).
 *
 * A database row older than the TTL reads back as `offline` with no location: it means
 * the cache lost the player (restart, eviction) and no heartbeat refreshed it since —
 * reporting it online would resurrect a stale state. The stale rule only applies when a
 * cache is configured; without one, the database keeps its legacy semantics.
 */
import { inArray, sql } from 'drizzle-orm';

import { cacheConfigured, PRESENCE_KEY_PREFIX, withCache } from '../cache/valkey.js';
import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  playerPresence,
  playerProfiles,
  type PlayerLocation,
  type PlayerPresence,
  type PresenceStatus,
} from '../db/schema/index.js';
import { requireProfile } from './profiles.service.js';

/** JSON shape stored under `presence:{playerId}`. */
interface CachedPresence {
  status: PresenceStatus;
  location: PlayerLocation | null;
  updatedAt: string;
}

/** One player of a batch heartbeat; an omitted `status` keeps a live one (`online` if none). */
export interface PresenceBatchEntry {
  playerId: string;
  status?: PresenceStatus;
  location?: PlayerLocation | null;
}

/** Result of a batch heartbeat: players refreshed and ids without a profile. */
export interface PresenceBatchResult {
  refreshed: number;
  unknown: string[];
}

function presenceKey(playerId: string): string {
  return `${PRESENCE_KEY_PREFIX}${playerId}`;
}

function toCached(row: PlayerPresence): CachedPresence {
  return { status: row.status, location: row.location ?? null, updatedAt: row.updatedAt.toISOString() };
}

function parseCached(playerId: string, raw: string | null): PlayerPresence | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as CachedPresence;
    const updatedAt = new Date(value.updatedAt);
    if (Number.isNaN(updatedAt.getTime())) return null;
    if (value.status !== 'online' && value.status !== 'mission' && value.status !== 'offline') return null;
    return { playerId, status: value.status, location: value.location ?? null, updatedAt };
  } catch {
    return null;
  }
}

/** A non-offline database row that no heartbeat has refreshed within the TTL. */
function isStale(row: PlayerPresence): boolean {
  return row.status !== 'offline' && Date.now() - row.updatedAt.getTime() > env.valkey.presenceTtlSeconds * 1000;
}

function staleToOffline(row: PlayerPresence): PlayerPresence {
  return { ...row, status: 'offline', location: null };
}

/** Upserts presence rows in one statement (multi-row `excluded.*` values on conflict). */
async function upsertPresenceRows(rows: PlayerPresence[]): Promise<void> {
  if (rows.length === 0) return;
  await db
    .insert(playerPresence)
    .values(
      rows.map((row) => ({
        playerId: row.playerId,
        status: row.status,
        location: row.location,
        updatedAt: row.updatedAt,
      })),
    )
    .onConflictDoUpdate({
      target: playerPresence.playerId,
      set: {
        status: sql`excluded.status`,
        location: sql`excluded.location`,
        updatedAt: sql`excluded.updated_at`,
      },
    });
}

/**
 * Upserts the presence of a player (single transition: login, mission, logout).
 * The cache write refreshes the TTL; the database only follows a status change, unless
 * the cache is unreachable (then the database is written every time, as before).
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
  const previous = (await getPresenceMap([playerId])).get(playerId);
  const presence: PlayerPresence = {
    playerId,
    status,
    location: location !== undefined ? location : (previous?.location ?? null),
    updatedAt: new Date(),
  };
  const cached = await withCache(
    (redis) =>
      redis
        .set(presenceKey(playerId), JSON.stringify(toCached(presence)), 'EX', env.valkey.presenceTtlSeconds)
        .then(() => true),
    () => Promise.resolve(false),
  );
  if (!cached || !previous || previous.status !== status) {
    await upsertPresenceRows([presence]);
  }
  return presence;
}

/**
 * Batch heartbeat: refreshes the TTL of every listed player in one cache pipeline.
 * Players absent from the call simply expire (TTL), no explicit offline event needed.
 * The database only follows the status transitions (or everything, when the cache is
 * unreachable), so a full server of stable players costs zero writes per minute.
 * Omitted `status` keeps the player's current non-offline status (default `online`).
 * @param entries - Active players of one game server (last entry per id wins).
 * @returns Refreshed count and ids without a profile.
 */
export async function setPresenceBatch(entries: PresenceBatchEntry[]): Promise<PresenceBatchResult> {
  const byId = new Map<string, PresenceBatchEntry>();
  for (const entry of entries) byId.set(entry.playerId, entry);
  const ids = [...byId.keys()];

  const known = new Set(
    (
      await db
        .select({ playerId: playerProfiles.playerId })
        .from(playerProfiles)
        .where(inArray(playerProfiles.playerId, ids))
    ).map((row) => row.playerId),
  );
  const unknown = ids.filter((id) => !known.has(id));
  const current = await getPresenceMap(ids.filter((id) => known.has(id)));

  const now = new Date();
  const rows: PlayerPresence[] = [];
  const changed: PlayerPresence[] = [];
  for (const id of ids) {
    if (!known.has(id)) continue;
    const entry = byId.get(id)!;
    const previous = current.get(id);
    // Omitted status keeps a live `mission` (a bare heartbeat must not demote it) and
    // defaults to `online` for new or previously-offline players.
    const status = entry.status ?? (previous && previous.status !== 'offline' ? previous.status : 'online');
    const row: PlayerPresence = {
      playerId: id,
      status,
      location: entry.location !== undefined ? entry.location : (previous?.location ?? null),
      updatedAt: now,
    };
    rows.push(row);
    if (!previous || previous.status !== status) changed.push(row);
  }

  const cached = await withCache(
    (redis) => {
      const pipeline = redis.pipeline();
      for (const row of rows) {
        pipeline.set(presenceKey(row.playerId), JSON.stringify(toCached(row)), 'EX', env.valkey.presenceTtlSeconds);
      }
      return pipeline.exec().then((results) => {
        const failed = results?.find(([err]) => err);
        if (failed) throw failed[0]; // → withCache falls back to the database
        return true;
      });
    },
    () => Promise.resolve(false),
  );
  await upsertPresenceRows(cached ? changed : rows);
  return { refreshed: rows.length, unknown };
}

/**
 * Fetches presence for several players, keyed by player id (missing = offline).
 * Cache hits answer first; misses fall back to the database, where a row older than the
 * TTL is reported offline (see the stale rule in this file's header).
 * @param playerIds - Player ids.
 * @returns Map of presence by player id.
 */
export async function getPresenceMap(playerIds: string[]): Promise<Map<string, PlayerPresence>> {
  const ids = [...new Set(playerIds)];
  if (ids.length === 0) return new Map();

  const cachedRows = await withCache(
    async (redis) => {
      const raws = await redis.mget(ids.map(presenceKey));
      return raws.map((raw, index) => parseCached(ids[index], raw));
    },
    () => null as PlayerPresence[] | null,
  );

  const map = new Map<string, PlayerPresence>();
  const missing: string[] = [];
  if (cachedRows) {
    cachedRows.forEach((row, index) => {
      if (row) map.set(row.playerId, row);
      else missing.push(ids[index]);
    });
  } else {
    missing.push(...ids);
  }

  if (missing.length > 0) {
    const rows = await db.select().from(playerPresence).where(inArray(playerPresence.playerId, missing));
    for (const row of rows) {
      map.set(row.playerId, cacheConfigured() && isStale(row) ? staleToOffline(row) : row);
    }
  }
  return map;
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
