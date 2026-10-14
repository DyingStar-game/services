import { z } from 'zod';

import { eq } from 'drizzle-orm';
import { db } from '../../db/connection.js';
import { missionPlayerStates } from '../../db/schema/index.js';
import { resolvePois, type PoiGeometry } from '../../services/inventory.client.js';
import { zoneMatches, type PoiGeometryMap } from '../../services/zones.service.js';
import type { MeasureContext, ObjectiveKind } from '../types.js';

const poiZone = z.object({ kind: z.literal('poi'), poiId: z.string().uuid() }).strict();

const paramsSchema = z.object({ zone: poiZone }).strict();

/** Player-state TTL in seconds: a state older than this counts as "outside every zone". */
const STATE_TTL_MS = 30_000;

async function resolveVisitPois(poiId: string): Promise<PoiGeometryMap> {
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
 * Visit a POI: measured against the player-state store (pushed by the game server).
 * High-water mark: once the player has been inside the POI, the objective stays complete.
 */
export const visitKind: ObjectiveKind = {
  kind: 'visit',
  evaluation: 'service',
  quantity: false,
  progress: 'highWater',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['zone'],
    properties: {
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
  name: { en: 'Visit', fr: 'Visite' },
  summary: {
    en: 'Reach the POI at {zone.poiId} (measured against player-state; latch on first visit)',
    fr: "Atteindre le POI {zone.poiId} (mesuré sur l'état joueur ; latch à la première visite)",
  },
  categories: ['exploration', 'delivery', 'transport', 'mining'],
  async measure(ctx: MeasureContext): Promise<number> {
    const { zone } = paramsSchema.parse(ctx.objective.params ?? {});

    const [state] = await db
      .select()
      .from(missionPlayerStates)
      .where(eq(missionPlayerStates.playerId, ctx.playerId))
      .limit(1);
    if (!state) return 0;
    if (Date.now() - state.receivedAt.getTime() > STATE_TTL_MS) return 0; // stale: outside every zone
    if (!state.place) return 0;

    const location = { system: state.place.system, scene: state.place.scene, position: state.place.x !== undefined ? { x: state.place.x, y: state.place.y!, z: state.place.z! } : null };
    const geometry = await resolveVisitPois(zone.poiId);
    return zoneMatches([zone], location, geometry) ? 1 : 0;
  },
};
