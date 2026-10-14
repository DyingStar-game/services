import { z } from 'zod';

import { eq } from 'drizzle-orm';
import { db } from '../../db/connection.js';
import { missionPlayerStates } from '../../db/schema/index.js';
import { getStack } from '../../services/inventory.client.js';
import { resolvePois } from '../../services/inventory.client.js';
import { zoneMatches, type PoiGeometryMap } from '../../services/zones.service.js';
import type { MeasureContext, ObjectiveKind } from '../types.js';

const poiZone = z.object({ kind: z.literal('poi'), poiId: z.string().uuid() }).strict();

const paramsSchema = z
  .object({
    /** Inventory good type to deliver. */
    goodType: z.string().trim().min(1).max(128),
    /** Delivery zone: a POI the assignee must be inside of. */
    zone: poiZone,
  })
  .strict();

/** Player-state TTL in seconds: a state older than this counts as "outside every zone". */
const STATE_TTL_MS = 30_000;

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
 * Deliver material: the assignee's carried quantity of a good type, measured while they
 * are inside the delivery POI. High-water mark: a completed delivery stays complete even
 * if the assignee leaves the zone. Partial delivery is supported (progress = quantity
 * currently carried in the zone).
 */
export const deliverMaterialKind: ObjectiveKind = {
  kind: 'deliver_material',
  evaluation: 'service',
  quantity: true,
  progress: 'highWater',
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
    },
    additionalProperties: false,
  },
  name: { en: 'Material delivery', fr: 'Livraison de matériau' },
  summary: {
    en: 'Deliver {targetQuantity} of {goodType} to the POI at {zone.poiId} (measured against carried goods + player-state)',
    fr: "Livrer {targetQuantity} de {goodType} au POI {zone.poiId} (mesuré sur les biens portés + l'état joueur)",
  },
  categories: ['delivery', 'transport', 'mining'],
  async measure(ctx: MeasureContext): Promise<number> {
    const { goodType, zone } = paramsSchema.parse(ctx.objective.params ?? {});

    // Check the player is currently inside the delivery POI.
    const [state] = await db
      .select()
      .from(missionPlayerStates)
      .where(eq(missionPlayerStates.playerId, ctx.playerId))
      .limit(1);
    if (!state) return 0;
    if (Date.now() - state.receivedAt.getTime() > STATE_TTL_MS) return 0; // stale
    if (!state.place) return 0;

    const location = {
      system: state.place.system,
      scene: state.place.scene,
      position: state.place.x !== undefined ? { x: state.place.x, y: state.place.y!, z: state.place.z! } : null,
    };
    const geometry = await resolveZonePois(zone.poiId);
    if (!zoneMatches([zone], location, geometry)) return 0;

    // Read the assignee's carried quantity for this good type.
    const stack = await getStack({ holderType: ctx.holderType, holderId: ctx.playerId }, goodType);
    return Math.min(stack?.carried ?? 0, ctx.objective.targetQuantity);
  },
};
