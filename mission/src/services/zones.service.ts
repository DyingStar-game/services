/**
 * Mission zone matching: a mission's `zones` are matched against the player's presence
 * location (`{system, scene, position}` reported by the game server through Social).
 *
 * Two implementations of the same semantics:
 * - `zoneFilterSql` for the listing (database-side, binds nulls when the location is
 *   unknown so zoned missions simply never match);
 * - `zoneMatches` for acceptance (fresh presence, fail-closed).
 *
 * An empty zone list is global (available everywhere). `scene` matching is hierarchical:
 * `tarsis_1` matches `tarsis_1` and `tarsis_1/new-paris`, never `tarsis_10`.
 */
import { sql, type SQL } from 'drizzle-orm';

import { missions, type Mission, type MissionZone, type ZoneLocation } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { getPlayerPresence } from './social.client.js';

/** Squared euclidean distance (NaN-safe: missing coordinates never match). */
function distanceSquared(
  a: { x: number; y: number; z: number },
  b: { x: number; y: number; z: number },
): number {
  return (a.x - b.x) ** 2 + (a.y - b.y) ** 2 + (a.z - b.z) ** 2;
}

/**
 * Whether a location satisfies at least one of the mission's zones.
 * @param zones - Mission zones (empty = global).
 * @param location - Player location (null when unknown → only global matches).
 * @returns True when the mission is available for that location.
 */
export function zoneMatches(zones: MissionZone[], location: ZoneLocation | null | undefined): boolean {
  if (zones.length === 0) return true;
  if (!location) return false;

  for (const zone of zones) {
    if (zone.kind === 'system') {
      if (location.system && location.system === zone.system) return true;
      continue;
    }
    // scene & area: a declared system must be present and equal.
    if (zone.system && location.system !== zone.system) continue;

    if (zone.kind === 'scene') {
      const scene = location.scene;
      if (scene && (scene === zone.scene || scene.startsWith(`${zone.scene}/`))) return true;
      continue;
    }

    const position = location.position;
    if (position && distanceSquared(position, zone.center) <= zone.radiusM ** 2) return true;
  }
  return false;
}

/**
 * Database-side equivalent of `zoneMatches` for the mission listing. `location` bindings
 * are plain parameters: when a part of the location is unknown (null), the corresponding
 * branch evaluates to NULL (never true), so zoned missions are hidden (fail-safe).
 *
 * Every binding carries an explicit cast (`::text`, `::float8`): PostgreSQL cannot infer
 * the type of a bare parameter (`42P18 could not determine data type of parameter`),
 * and a bind used only in `is not null` has no inferable type at all.
 * @param location - Player location (null when unknown).
 * @returns A drizzle condition over `missions.zones`.
 */
export function zoneFilterSql(location: ZoneLocation | null | undefined): SQL {
  const system = location?.system ?? null;
  const scene = location?.scene ?? null;
  const position = location?.position ?? null;
  const px = position?.x ?? null;
  const py = position?.y ?? null;
  const pz = position?.z ?? null;

  return sql`(
    jsonb_array_length(${missions.zones}) = 0
    or exists (
      select 1 from jsonb_array_elements(${missions.zones}) as z
      where
        (z->>'kind' = 'system' and z->>'system' = ${system}::text)
        or (
          z->>'kind' = 'scene'
          and (z->>'system' is null or z->>'system' = ${system}::text)
          and (z->>'scene' = ${scene}::text or ${scene}::text like ((z->>'scene') || '/%'))
        )
        or (
          z->>'kind' = 'area'
          and (z->>'system' is null or z->>'system' = ${system}::text)
          and (
            (${px}::float8 - (z->'center'->>'x')::float8) ^ 2
            + (${py}::float8 - (z->'center'->>'y')::float8) ^ 2
            + (${pz}::float8 - (z->'center'->>'z')::float8) ^ 2
          ) <= ((z->>'radiusM')::float8) ^ 2
        )
    )
  )`;
}

/**
 * Enforces a mission's zones at acceptance (players only). A mission without zones costs
 * nothing (no Social call). A missing location or a Social outage fails closed.
 * @param mission - Mission being accepted.
 * @param playerId - Player accepting.
 * @throws 403 `OUT_OF_ZONE` when the player is not in a zone where the mission is available.
 */
export async function assertMissionZone(mission: Mission, playerId: string): Promise<void> {
  if (!mission.zones || mission.zones.length === 0) return;

  let location: ZoneLocation | null = null;
  try {
    location = (await getPlayerPresence(playerId))?.location ?? null;
  } catch {
    location = null; // fail closed: without a verified location, only global missions
  }
  if (!zoneMatches(mission.zones, location)) {
    throw new HttpError(403, 'OUT_OF_ZONE', 'You are not in a zone where this mission is available');
  }
}
