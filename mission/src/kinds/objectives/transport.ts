import { z } from 'zod';

import { eq } from 'drizzle-orm';
import { db } from '../../db/connection.js';
import { missionObjectives, missionPlayerStates } from '../../db/schema/index.js';
import { getStack, resolvePois } from '../../services/inventory.client.js';
import { zoneMatches, type PoiGeometryMap } from '../../services/zones.service.js';
import type { MeasureContext, ObjectiveKind } from '../types.js';

const poiZone = z.object({ kind: z.literal('poi'), poiId: z.string().uuid() }).strict();

const paramsSchema = z
  .object({
    /** Inventory good type to transport. */
    goodType: z.string().trim().min(1).max(128),
    /** Pickup POI: the assignee must be here with the goods carried (latched). */
    source: poiZone,
    /** Delivery POI: the assignee must be here with the goods carried. */
    zone: poiZone,
  })
  .strict();

/** Player-state TTL in seconds. */
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
    return {};
  }
}

/**
 * Transport: two-phase objective. Phase 1 — latch `pickedUpAt` when the assignee is at
 * the source POI with goods carried. Phase 2 — measure the carried quantity in the
 * delivery POI. High-water mark: a completed transport stays complete.
 */
export const transportKind: ObjectiveKind = {
  kind: 'transport',
  evaluation: 'service',
  quantity: true,
  progress: 'highWater',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['goodType', 'source', 'zone'],
    properties: {
      goodType: { type: 'string', maxLength: 128 },
      source: {
        type: 'object',
        required: ['kind', 'poiId'],
        properties: { kind: { const: 'poi' }, poiId: { type: 'string', format: 'uuid' } },
        additionalProperties: false,
      },
      zone: {
        type: 'object',
        required: ['kind', 'poiId'],
        properties: { kind: { const: 'poi' }, poiId: { type: 'string', format: 'uuid' } },
        additionalProperties: false,
      },
    },
    additionalProperties: false,
  },
  name: { en: 'Transport', fr: 'Transport' },
  summary: {
    en: 'Pick up {goodType} at {source.poiId} then deliver to {zone.poiId} (two-phase, high-water mark)',
    fr: "Charger {goodType} au POI {source.poiId} puis livrer au POI {zone.poiId} (deux phases, high-water mark)",
  },
  categories: ['transport', 'delivery', 'mining'],
  async measure(ctx: MeasureContext): Promise<number> {
    const { goodType, source, zone } = paramsSchema.parse(ctx.objective.params ?? {});
    const state = await db
      .select()
      .from(missionPlayerStates)
      .where(eq(missionPlayerStates.playerId, ctx.playerId))
      .limit(1)
      .then((rows) => rows[0]);
    if (!state) return 0;
    if (Date.now() - state.receivedAt.getTime() > STATE_TTL_MS) return 0; // stale
    if (!state.place) return 0;

    const location = {
      system: state.place.system,
      scene: state.place.scene,
      position: state.place.x !== undefined ? { x: state.place.x, y: state.place.y!, z: state.place.z! } : null,
    };

    // Phase 1: latch pickedUpAt when the assignee is at the source POI with goods carried.
    const stateObj = (ctx.objective.state ?? {}) as Record<string, unknown>;
    if (!stateObj.pickedUpAt) {
      const sourceGeometry = await resolveZonePois(source.poiId);
      const atSource = zoneMatches([source], location, sourceGeometry);
      if (atSource) {
        const stack = await getStack({ holderType: ctx.holderType, holderId: ctx.playerId }, goodType);
        if ((stack?.carried ?? 0) > 0) {
          // Latch: the assignee has been seen at the source with goods carried.
          await db
            .update(missionObjectives)
            .set({ state: { ...stateObj, pickedUpAt: { poiId: source.poiId } }, updatedAt: new Date() })
            .where(eq(missionObjectives.id, ctx.objective.id));
        }
      }
      return 0; // not yet delivered, regardless of pickup status
    }

    // Phase 2: check delivery at the destination POI.
    const deliveryGeometry = await resolveZonePois(zone.poiId);
    if (!zoneMatches([zone], location, deliveryGeometry)) return 0;

    const stack = await getStack({ holderType: ctx.holderType, holderId: ctx.playerId }, goodType);
    return Math.min(stack?.carried ?? 0, ctx.objective.targetQuantity);
  },
};
