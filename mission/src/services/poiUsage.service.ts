/**
 * POI usage lookup, used by Inventory as a write-time lock before it lets a player move a
 * POI (`PATCH /api/me/pois/:poiId`). A POI referenced by a mission **whose assignment is
 * active** may not move: the acceptor signed a contract whose destination would change
 * under their feet.
 *
 * Only `active` assignments lock. A mission created but never accepted (or whose only
 * assignments are terminal) leaves the POI free to move.
 *
 * Both references count: a mission availability zone (`missions.zones[].poiId`) and an
 * objective target (`params.zone.poiId`, plus `params.source.poiId` for `transport`).
 */
import { sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { ACTIVE_ASSIGNMENT_STATUS, missionAssignments } from '../db/schema/assignments.js';
import { missionObjectives, missions } from '../db/schema/missions.js';

/**
 * Finds the POIs referenced by a mission with at least one active assignment.
 * @param poiIds - POI ids to check.
 * @returns Map of `poiId` → mission ids locking it (a POI absent from the map is free).
 */
export async function findPoisInUse(poiIds: string[]): Promise<Map<string, string[]>> {
  const wanted = [...new Set(poiIds)];
  const result = new Map<string, string[]>();
  if (wanted.length === 0) return result;

  /** Compared as text: `z->>'poiId'` and `params->…->>'poiId'` are both text. */
  const idsList = sql.join(wanted.map((id) => sql`${id}::text`), sql`, `);
  const active = sql`${ACTIVE_ASSIGNMENT_STATUS}::text`;

  const { rows } = await db.execute<{ poi_id: string; mission_id: string }>(sql`
    (
      select z->>'poiId' as poi_id, a.mission_id
      from ${missions} m
      join ${missionAssignments} a on a.mission_id = m.id
      cross join jsonb_array_elements(m.zones) z
      where a.status = ${active}
        and z->>'kind' = 'poi'
        and z->>'poiId' in (${idsList})
    )
    union
    (
      select o.params->'zone'->>'poiId' as poi_id, o.mission_id
      from ${missionObjectives} o
      join ${missionAssignments} a on a.mission_id = o.mission_id
      where a.status = ${active}
        and o.params->'zone'->>'poiId' in (${idsList})
    )
    union
    (
      select o.params->'source'->>'poiId' as poi_id, o.mission_id
      from ${missionObjectives} o
      join ${missionAssignments} a on a.mission_id = o.mission_id
      where a.status = ${active}
        and o.params->'source'->>'poiId' in (${idsList})
    )
  `);
  for (const row of rows as Array<{ poi_id: string; mission_id: string }>) {
    const list = result.get(row.poi_id) ?? [];
    list.push(row.mission_id);
    result.set(row.poi_id, list);
  }
  return result;
}

/** Whether a POI is referenced by a mission with at least one active assignment. */
export async function isPoiInUse(poiId: string): Promise<boolean> {
  const usage = await findPoisInUse([poiId]);
  return usage.has(poiId);
}
