import { z } from 'zod';

import { and, eq } from 'drizzle-orm';
import { db } from '../../db/connection.js';
import { ACTIVE_ASSIGNMENT_STATUS, missionAssignments } from '../../db/schema/assignments.js';
import { missionPlayerStates } from '../../db/schema/index.js';
import { getStack } from '../../services/inventory.client.js';
import { resolvePois } from '../../services/inventory.client.js';
import { placeToLocation, zoneMatches, type PoiGeometryMap } from '../../services/zones.service.js';
import type { MeasureContext, ObjectiveKind } from '../types.js';
import { STATE_TTL_MS, isStateFresh, loadPlayerState } from './playerState.js';

const poiZone = z.object({ kind: z.literal('poi'), poiId: z.string().uuid() }).strict();

const paramsSchema = z
  .object({
    /** Inventory good type to deliver. */
    goodType: z.string().trim().min(1).max(128),
    /** Delivery zone: a POI the assignee must be inside of. */
    zone: poiZone,
    /**
     * `assignee` (default): only the measured assignee's own carried quantity counts.
     * `all`: the sum over **every active assignee of the mission who is currently inside
     * the target**, so several carriers can pool a large delivery. A carrier outside the
     * target contributes nothing, no matter how much they carry.
     */
    scope: z.enum(['assignee', 'all']).default('assignee'),
  })
  .strict();

async function resolveZonePois(poiId: string): Promise<PoiGeometryMap> {
  try {
    const resolved = await resolvePois([poiId]);
    const map: PoiGeometryMap = {};
    for (const poi of resolved.values()) {
      map[poi.id] = {
        system: poi.system,
        scene: poi.scene,
        center: { x: poi.x, y: poi.y, z: poi.z },
        radiusM: poi.radiusM,
      };
    }
    return map;
  } catch {
    return {}; // fail closed: no geometry = never matches
  }
}

/**
 * Sums the carried quantity of a good type over every active assignee of the mission who
 * is **currently inside the target**. Uses the pushed player-state snapshot (one query for
 * the whole mission); an assignee whose state is missing or stale counts as outside.
 * @param missionId - Mission whose assignees are pooled.
 * @param zone - Delivery target (a bare POI zone).
 * @param goodType - Inventory good type being summed.
 * @param geometry - Resolved geometry of the target POI.
 * @returns Total carried by the assignees inside the target.
 */
async function carriedByAssigneesInZone(
  missionId: string,
  zone: { kind: 'poi'; poiId: string },
  goodType: string,
  geometry: PoiGeometryMap,
): Promise<number> {
  const assignees = await db
    .select({
      place: missionPlayerStates.place,
      carried: missionPlayerStates.carried,
      receivedAt: missionPlayerStates.receivedAt,
    })
    .from(missionAssignments)
    .leftJoin(missionPlayerStates, eq(missionPlayerStates.playerId, missionAssignments.playerId))
    .where(
      and(
        eq(missionAssignments.missionId, missionId),
        eq(missionAssignments.status, ACTIVE_ASSIGNMENT_STATUS),
      ),
    );

  let total = 0;
  for (const assignee of assignees) {
    if (!assignee.place || !assignee.receivedAt) continue; // no pushed state: outside
    if (Date.now() - assignee.receivedAt.getTime() > STATE_TTL_MS) continue; // stale
    const location = placeToLocation(assignee.place);
    if (!zoneMatches([zone], location, geometry)) continue;
    total += assignee.carried?.[goodType] ?? 0;
  }
  return total;
}

/**
 * Deliver material: the carried quantity of a good type, measured while the carrier is
 * inside the delivery POI. High-water mark: a completed delivery stays complete even if
 * the carrier leaves the zone. Partial delivery is supported (progress = quantity
 * currently carried in the zone). With `scope: 'all'` the quantities of every active
 * assignee inside the target are summed, so a group can pool a delivery.
 */
export const deliverMaterialKind: ObjectiveKind = {
  kind: 'deliver_material',
  evaluation: 'service',
  quantity: true,
  progress: 'highWater',
  remoteRead: true,
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['goodType', 'zone'],
    properties: {
      goodType: { type: 'string', maxLength: 128, description: 'Inventory good type to deliver' },
      zone: {
        type: 'object',
        required: ['kind', 'poiId'],
        properties: {
          kind: { const: 'poi' },
          poiId: { type: 'string', format: 'uuid' },
        },
        additionalProperties: false,
      },
      scope: {
        type: 'string',
        enum: ['assignee', 'all'],
        default: 'assignee',
        description:
          "who counts: 'assignee' = the measured carrier only; 'all' = the sum over every active assignee currently inside the target",
      },
    },
    additionalProperties: false,
  },
  name: { en: 'Material delivery', fr: 'Livraison de matériau' },
  summary: {
    en: 'Deliver {targetQuantity} of {goodType} to the POI at {zone.poiId} (measured against carried goods + player-state; scope:all pools every carrier inside the target)',
    fr: "Livrer {targetQuantity} de {goodType} au POI {zone.poiId} (mesuré sur les biens portés + l'état joueur ; scope:all cumule tous les porteurs dans la cible)",
  },
  categories: ['delivery', 'transport', 'mining'],
  async measure(ctx: MeasureContext): Promise<number> {
    const { goodType, zone, scope } = paramsSchema.parse(ctx.objective.params ?? {});
    const geometry = await resolveZonePois(zone.poiId);

    // Group delivery: pool every active assignee currently inside the target.
    if (scope === 'all') {
      const total = await carriedByAssigneesInZone(ctx.missionId, zone, goodType, geometry);
      return Math.min(total, ctx.objective.targetQuantity);
    }

    // Check the assignee is currently inside the delivery POI.
    const state = await loadPlayerState(ctx, ctx.playerId);
    if (!state) return 0;
    if (!isStateFresh(state)) return 0; // stale
    const location = placeToLocation(state.place);
    if (!location || !zoneMatches([zone], location, geometry)) return 0;

    // Read the assignee's carried quantity for this good type.
    const stack = await getStack({ holderType: ctx.holderType, holderId: ctx.playerId }, goodType);
    return Math.min(stack?.carried ?? 0, ctx.objective.targetQuantity);
  },
};
