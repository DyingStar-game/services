import { z } from 'zod';

import { resolvePois } from '../../services/inventory.client.js';
import { zoneMatches, type PoiGeometryMap } from '../../services/zones.service.js';
import type { MeasureContext, ObjectiveKind } from '../types.js';
import { isStateFresh, loadPlayerState, stateLocation } from './playerState.js';

const poiZone = z.object({ kind: z.literal('poi'), poiId: z.string().uuid() }).strict();

const paramsSchema = z
  .object({
    /** Passenger UUID (a player). Must not be the driver. */
    passengerId: z.string().uuid(),
    /** Drop-off POI. */
    zone: poiZone,
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
    return {};
  }
}

/**
 * Passenger transport: the passenger (a player) must be inside the drop-off POI while
 * seated in a non-driver seat. The passenger's own state is used (not the driver's).
 * High-water mark: a completed transport stays complete.
 */
export const transportPassengerKind: ObjectiveKind = {
  kind: 'transport_passenger',
  evaluation: 'service',
  quantity: false,
  progress: 'highWater',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['passengerId', 'zone'],
    properties: {
      passengerId: { type: 'string', format: 'uuid', description: 'Passenger player UUID (not the driver)' },
      zone: {
        type: 'object',
        required: ['kind', 'poiId'],
        properties: { kind: { const: 'poi' }, poiId: { type: 'string', format: 'uuid' } },
        additionalProperties: false,
      },
    },
    additionalProperties: false,
  },
  name: { en: 'Passenger transport', fr: 'Transport de passager' },
  summary: {
    en: 'Deliver passenger {passengerId} to the POI at {zone.poiId} (measured on the passenger\'s state, must be seated as non-driver)',
    fr: "Amener le passager {passengerId} au POI {zone.poiId} (mesuré sur l'état du passager, assis en siège non-pilote)",
  },
  categories: ['transport'],
  async measure(ctx: MeasureContext): Promise<number> {
    const { passengerId, zone } = paramsSchema.parse(ctx.objective.params ?? {});

    // Read the passenger's state (not the driver's).
    const state = await loadPlayerState(ctx, passengerId);
    if (!state) return 0;
    if (!isStateFresh(state)) return 0; // stale
    if (!state.place) return 0;
    if (state.onFoot) return 0; // passenger must be seated
    if (!state.passenger) return 0; // must be a passenger, not the driver

    const location = stateLocation(state);
    const geometry = await resolveZonePois(zone.poiId);
    return zoneMatches([zone], location, geometry) ? 1 : 0;
  },
};
