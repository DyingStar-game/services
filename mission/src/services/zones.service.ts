/**
 * Mission zone matching: a mission's `zones` are matched against the player's presence
 * location (`{system, scene, position}` reported by the game server through Social).
 *
 * Two zone kinds (geometry lives in the Inventory POI registry, not here):
 * - `system`: same system;
 * - `poi`: a POI from Inventory — a zone when a radius applies (the zone's `radiusM`
 *   overrides the POI's), otherwise the POI's scene subtree (prefix match), otherwise
 *   the exact POI coordinates. POIs are resolved through Inventory's internal API
 *   (60 s cache); an unresolved/missing POI never matches (fail-closed, exactly like
 *   an unknown location).
 *
 * Two implementations of the same semantics:
 * - `zoneFilterSql` for the listing (database-side; the resolved geometries are injected
 *   as one jsonb parameter so unresolvable POIs simply never match);
 * - `zoneMatches` for acceptance (fresh presence, fail-closed).
 *
 * An empty zone list is global (available everywhere). `scene` matching is hierarchical:
 * `tarsis_1` matches `tarsis_1` and `tarsis_1/new-paris`, never `tarsis_10`.
 */
import { sql, type SQL } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { missions, type Mission, type MissionZone, type ZoneLocation } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { t } from '../i18n/index.js';
import { isInventoryConfigured, resolvePois, type PoiGeometry } from './inventory.client.js';
import { getPlayerPresence } from './social.client.js';

/** Resolved geometry of a POI, in the shape both matchers understand. */
export interface PoiZoneGeometry {
  system: string | null;
  scene: string | null;
  center: { x: number; y: number; z: number };
  radiusM: number | null;
}

/** POI id → resolved geometry (missing entry = zone that never matches). */
export type PoiGeometryMap = Record<string, PoiZoneGeometry>;

/** Squared euclidean distance (NaN-safe: missing coordinates never match). */
function distanceSquared(
  a: { x: number; y: number; z: number },
  b: { x: number; y: number; z: number },
): number {
  return (a.x - b.x) ** 2 + (a.y - b.y) ** 2 + (a.z - b.z) ** 2;
}

function toPoiGeometry(poi: PoiGeometry): PoiZoneGeometry {
  return {
    system: poi.system,
    scene: poi.scene,
    center: { x: poi.x, y: poi.y, z: poi.z },
    radiusM: poi.radiusM,
  };
}

function toGeometryMap(resolved: Map<string, PoiGeometry>): PoiGeometryMap {
  const map: PoiGeometryMap = {};
  for (const [id, poi] of resolved) map[id] = toPoiGeometry(poi);
  return map;
}

/** The `poi` zones of a list. */
function poiZoneIds(zones: MissionZone[]): string[] {
  return zones.flatMap((zone) => (zone.kind === 'poi' ? [zone.poiId] : []));
}

/**
 * Resolves the geometries of the POIs referenced by some zones.
 * An Inventory outage degrades to "no geometry" (every `poi` zone stops matching)
 * instead of failing the caller — the fail-closed behaviour is the same.
 * @param zones - Zones about to be matched.
 * @returns POI id → geometry.
 */
async function resolveZonePois(zones: MissionZone[]): Promise<PoiGeometryMap> {
  const ids = poiZoneIds(zones);
  if (ids.length === 0) return {};
  try {
    return toGeometryMap(await resolvePois(ids));
  } catch {
    return {}; // fail closed: without geometry, no poi zone matches
  }
}

/**
 * Resolves the geometry of every POI referenced by any mission zone (listing scope).
 * A mission without `poi` zones never touches Inventory.
 * @returns POI id → geometry (empty when no mission references a POI).
 */
export async function resolveListingPoiGeometry(): Promise<PoiGeometryMap> {
  const result = await db.execute<{ poi_id: string | null }>(
    sql`select distinct z->>'poiId' as "poi_id"
        from ${missions}, jsonb_array_elements(${missions}.zones) as z
        where z->>'kind' = 'poi'
        limit 500`,
  );
  const ids = [...new Set(result.rows.map((row) => row.poi_id).filter((id): id is string => Boolean(id)))];
  if (ids.length === 0) return {};
  try {
    return toGeometryMap(await resolvePois(ids));
  } catch {
    return {}; // fail closed
  }
}

/**
 * Whether a location satisfies at least one of the mission's zones.
 * @param zones - Mission zones (empty = global).
 * @param location - Player location (null when unknown → only global matches).
 * @param poiGeometry - Resolved POI geometries (missing POIs never match).
 * @returns True when the mission is available for that location.
 */
export function zoneMatches(
  zones: MissionZone[],
  location: ZoneLocation | null | undefined,
  poiGeometry: PoiGeometryMap = {},
): boolean {
  if (zones.length === 0) return true;
  if (!location) return false;

  for (const zone of zones) {
    if (zone.kind === 'system') {
      if (location.system && location.system === zone.system) return true;
      continue;
    }

    // poi: a declared system on the POI must be present and equal.
    const poi = poiGeometry[zone.poiId];
    if (!poi) continue; // unknown/deleted POI: fail closed
    if (poi.system && location.system !== poi.system) continue;

    const radius = zone.radiusM ?? poi.radiusM;
    if (radius !== null) {
      // Area: within `radius` of the POI centre (zone override wins over the POI's).
      const position = location.position;
      if (position && distanceSquared(position, poi.center) <= radius ** 2) return true;
      continue;
    }
    if (poi.scene) {
      // Scene subtree of the POI (same hierarchical prefix convention as before).
      const scene = location.scene;
      if (scene && (scene === poi.scene || scene.startsWith(`${poi.scene}/`))) return true;
      continue;
    }
    // Plain point: only a player standing exactly on the POI matches.
    const position = location.position;
    if (
      position &&
      position.x === poi.center.x &&
      position.y === poi.center.y &&
      position.z === poi.center.z
    ) {
      return true;
    }
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
 * and a bind used only in `is not null` has no inferable type at all. The resolved POI
 * geometries travel as a single jsonb parameter; a zone referencing a POI absent from the
 * map resolves to NULL and never matches.
 * @param location - Player location (null when unknown).
 * @param poiGeometry - Resolved POI geometries (from `resolveListingPoiGeometry`).
 * @returns A drizzle condition over `missions.zones`.
 */
export function zoneFilterSql(location: ZoneLocation | null | undefined, poiGeometry: PoiGeometryMap = {}): SQL {
  const system = location?.system ?? null;
  const scene = location?.scene ?? null;
  const position = location?.position ?? null;
  const px = position?.x ?? null;
  const py = position?.y ?? null;
  const pz = position?.z ?? null;
  const poiJson = JSON.stringify(poiGeometry);
  /** The geometry of the zone's POI, or NULL when it does not resolve. */
  const poi = () => sql`(${poiJson}::jsonb -> (z->>'poiId'))`;

  return sql`(
    jsonb_array_length(${missions.zones}) = 0
    or exists (
      select 1 from jsonb_array_elements(${missions.zones}) as z
      where
        (z->>'kind' = 'system' and z->>'system' = ${system}::text)
        or (
          z->>'kind' = 'poi'
          and ${poi()} is not null
          and (${poi()}->>'system' is null or ${poi()}->>'system' = ${system}::text)
          and (
            case
              when (z->>'radiusM') is not null then (
                (${px}::float8 - (${poi()}->'center'->>'x')::float8) ^ 2
                + (${py}::float8 - (${poi()}->'center'->>'y')::float8) ^ 2
                + (${pz}::float8 - (${poi()}->'center'->>'z')::float8) ^ 2
              ) <= ((z->>'radiusM')::float8) ^ 2
              when (${poi()}->>'radiusM') is not null then (
                (${px}::float8 - (${poi()}->'center'->>'x')::float8) ^ 2
                + (${py}::float8 - (${poi()}->'center'->>'y')::float8) ^ 2
                + (${pz}::float8 - (${poi()}->'center'->>'z')::float8) ^ 2
              ) <= ((${poi()}->>'radiusM')::float8) ^ 2
              when (${poi()}->>'scene') is not null then (
                ${scene}::text = (${poi()}->>'scene')
                or ${scene}::text like (((${poi()}->>'scene')) || '/%')
              )
              else (
                (${px}::float8, ${py}::float8, ${pz}::float8)
                = ((${poi()}->'center'->>'x')::float8, (${poi()}->'center'->>'y')::float8, (${poi()}->'center'->>'z')::float8)
              )
            end
          )
        )
    )
  )`;
}

/**
 * Enforces a mission's zones at acceptance (players only). A mission without zones costs
 * nothing (no Social call). A missing location or a Social outage fails closed; POI
 * geometry resolves through the same fail-closed path.
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
  const poiGeometry = await resolveZonePois(mission.zones);
  if (!zoneMatches(mission.zones, location, poiGeometry)) {
    throw new HttpError(403, 'OUT_OF_ZONE', 'You are not in a zone where this mission is available');
  }
}

/**
 * Write-time guard: every `poi` zone must reference an existing POI. Checked only when
 * the Inventory integration is configured; an Inventory outage does not block the write
 * (matching stays fail-closed anyway), an unknown id does.
 * @param zones - Zones about to be persisted.
 * @throws 400 `POI_NOT_FOUND` when a referenced POI does not exist.
 */
export async function assertPoiZonesExist(zones: MissionZone[] | undefined): Promise<void> {
  const ids = poiZoneIds(zones ?? []);
  if (ids.length === 0 || !isInventoryConfigured()) return;
  let resolved: Map<string, PoiGeometry>;
  try {
    resolved = await resolvePois(ids);
  } catch {
    return; // outage: unavailable ≠ invalid, matching remains fail-closed
  }
  const missing = ids.find((id) => !resolved.has(id));
  if (missing) throw new HttpError(400, 'POI_NOT_FOUND', t('zones.poi_not_found', { id: missing }));
}
