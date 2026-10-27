import { z } from 'zod';

import { resolvePois } from '../../services/inventory.client.js';
import { zoneMatches, type PoiGeometryMap } from '../../services/zones.service.js';
import type { MeasureContext, ObjectiveKind } from '../types.js';
import { isStateFresh, loadPlayerState, stateLocation } from './playerState.js';

const poiZone = z.object({ kind: z.literal('poi'), poiId: z.guid() }).strict();

const paramsSchema = z.object({ zone: poiZone }).strict();

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

    const state = await loadPlayerState(ctx, ctx.playerId);
    if (!state) return 0;
    if (!isStateFresh(state)) return 0; // stale: outside every zone
    if (!state.place) return 0;

    const location = stateLocation(state);
    const geometry = await resolveVisitPois(zone.poiId);
    return zoneMatches([zone], location, geometry) ? 1 : 0;
  },
};
