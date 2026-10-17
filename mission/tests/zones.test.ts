/**
 * Mission zone matching in the local-body frame: scene compatibility before any distance
 * comparison, the SQL twin for listing, and an end-to-end verify through the internal API.
 * The database is an in-process PGlite migrated from `./drizzle`.
 */
import type { Server } from 'http';
import { eq } from 'drizzle-orm';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';
import type { PoiGeometryMap } from '../src/services/zones.service.js';
import type { ZoneLocation } from '../src/db/schema/index.js';

const inventory = vi.hoisted(() => ({
  resolvePois: vi.fn(),
}));

const social = vi.hoisted(() => ({
  getPlayerPresence: vi.fn(),
}));

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
vi.mock('../src/services/inventory.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/inventory.client.js')>()),
  resolvePois: inventory.resolvePois,
}));
vi.mock('../src/services/social.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/social.client.js')>()),
  getPlayerPresence: social.getPlayerPresence,
}));

const { db } = await import('../src/db/connection.js');
const { missions } = await import('../src/db/schema/index.js');
const { zoneMatches, zoneFilterSql, scenesCompatible } = await import('../src/services/zones.service.js');
const { upsertPlayerStates } = await import('../src/services/playerStates.service.js');

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const POI_TARSIS_3 = '11111111-1111-4111-8111-111111111111';
const POI_TARSIS_4 = '22222222-2222-4222-9222-222222222222';
const POI_TARSIS_1 = '33333333-3333-4333-a333-333333333333';
const POI_SPACE = '44444444-4444-4444-8444-444444444444';
const POI_NO_SCENE = '55555555-5555-4555-9555-555555555555';

const ALL_ROLES = 'mission:read,mission:write,mission:complete,mission:progress';

let server: Server;
let baseUrl: string;

/** Internal API call. */
async function call(
  path: string,
  method: string,
  body?: unknown,
  roles: string = ALL_ROLES,
): Promise<{ status: number; body: any }> {
  const res = await fetch(`${baseUrl}${path}`, {
    method,
    headers: { 'content-type': 'application/json', 'x-test-client': 'svc-universesimulator', 'x-test-roles': roles },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: res.status, body: await res.json().catch(() => null) };
}

// ── Geometry fixtures ──────────────────────────────────────────────────────────

/** POI on `tarsis_3`, 3 km from its centre, zone radius 5 km. */
const poiTarsis3 = { system: 'tarsis', scene: 'tarsis_3', x: 3000, y: 0, z: 0, radiusM: 5000 };
/** POI on `tarsis_4`, 3 km from its centre, zone radius 5 km. */
const poiTarsis4 = { system: 'tarsis', scene: 'tarsis_4', x: 3000, y: 0, z: 0, radiusM: 5000 };
/** POI on `tarsis_1`, scene-subtree (no radius). */
const poiTarsis1 = { system: 'tarsis', scene: 'tarsis_1', x: 0, y: 0, z: 0, radiusM: null };
/** POI in open space of `tarsis`, zone radius 5 km. */
const poiSpace = { system: 'tarsis', scene: 'space', x: 1_000_000, y: 0, z: 0, radiusM: 5000 };
/** POI with no scene (fail-closed). */
const poiNoScene = { system: 'tarsis', scene: null, x: 0, y: 0, z: 0, radiusM: null };

/** Player on `tarsis_3`, 3 km from its centre. */
const placeTarsis3: ZoneLocation = { system: 'tarsis', scene: 'tarsis_3', position: { x: 3000, y: 0, z: 0 } };
/** Player on `tarsis_4`, 3 km from its centre (same numeric offset as placeTarsis3). */
const placeTarsis4: ZoneLocation = { system: 'tarsis', scene: 'tarsis_4', position: { x: 3000, y: 0, z: 0 } };
/** Player in open space of `tarsis`. */
const placeSpace: ZoneLocation = { system: 'tarsis', scene: 'space', position: { x: 1_000_000, y: 0, z: 0 } };
/** Player on `tarsis_1`. */
const placeTarsis1: ZoneLocation = { system: 'tarsis', scene: 'tarsis_1', position: { x: 0, y: 0, z: 0 } };
/** Player on `tarsis_1/new-paris` (sub-scene of tarsis_1). */
const placeNewParis: ZoneLocation = { system: 'tarsis', scene: 'tarsis_1/new-paris', position: { x: 0, y: 0, z: 0 } };
/** Player on `tarsis_10` (different body, similar prefix). */
const placeTarsis10: ZoneLocation = { system: 'tarsis', scene: 'tarsis_10', position: { x: 0, y: 0, z: 0 } };

/** Player-state format: `{system, scene, x, y, z}` (what `PUT /player-states` stores). */
const stateTarsis3 = { system: 'tarsis', scene: 'tarsis_3', x: 3000, y: 0, z: 0 };
const stateTarsis4 = { system: 'tarsis', scene: 'tarsis_4', x: 3000, y: 0, z: 0 };
const stateSpace = { system: 'tarsis', scene: 'space', x: 1_000_000, y: 0, z: 0 };

/** Builds a poi zone with a radius. */
function zone(poiId: string, radiusM: number) {
  return { kind: 'poi' as const, poiId, radiusM };
}

/** Builds a geometry map from POI id → geometry (converts flat x/y/z to `center`). */
function geo(...entries: [string, { system: string | null; scene: string | null; x: number; y: number; z: number; radiusM: number | null }][]): PoiGeometryMap {
  return Object.fromEntries(
    entries.map(([id, g]) => [id, { system: g.system, scene: g.scene, center: { x: g.x, y: g.y, z: g.z }, radiusM: g.radiusM }]),
  );
}

// ── scenesCompatible ───────────────────────────────────────────────────────────

describe('scenesCompatible', () => {
  it('matches equal scenes', () => {
    expect(scenesCompatible('tarsis_3', 'tarsis_3')).toBe(true);
    expect(scenesCompatible('space', 'space')).toBe(true);
  });

  it('matches segment prefix (parent ⟷ child)', () => {
    expect(scenesCompatible('tarsis_1', 'tarsis_1/new-paris')).toBe(true);
    expect(scenesCompatible('tarsis_1/new-paris', 'tarsis_1')).toBe(true); // symmetric
  });

  it('never matches bare character prefix', () => {
    expect(scenesCompatible('tarsis_1', 'tarsis_10')).toBe(false);
    expect(scenesCompatible('tarsis_10', 'tarsis_1')).toBe(false);
  });

  it('never matches missing or empty scenes', () => {
    expect(scenesCompatible('tarsis_3', null)).toBe(false);
    expect(scenesCompatible('tarsis_3', undefined)).toBe(false);
    expect(scenesCompatible('tarsis_3', '')).toBe(false);
    expect(scenesCompatible(null, 'tarsis_3')).toBe(false);
    expect(scenesCompatible('', 'tarsis_3')).toBe(false);
  });
});

// ── zoneMatches (unit) ─────────────────────────────────────────────────────────

describe('zoneMatches', () => {
  it('test 1: same numeric offset on different bodies → no match', () => {
    // Player on tarsis_3 at 3 km; POI on tarsis_4 at 3 km; zone radius 5 km.
    const zones = [zone(POI_TARSIS_4, 5000)];
    const geometry = geo([POI_TARSIS_4, poiTarsis4]);
    expect(zoneMatches(zones, placeTarsis3, geometry)).toBe(false);
  });

  it('test 2: same scene, same offset, small radius → match', () => {
    const zones = [zone(POI_TARSIS_3, 100)];
    const geometry = geo([POI_TARSIS_3, poiTarsis3]);
    expect(zoneMatches(zones, placeTarsis3, geometry)).toBe(true);
  });

  it('test 3: segment prefix (tarsis_1 ⟷ tarsis_1/new-paris) → match', () => {
    const zones = [zone(POI_TARSIS_1, 5000)];
    const geometry = geo([POI_TARSIS_1, { ...poiTarsis1, radiusM: 5000, x: 0, y: 0, z: 0 }]);
    expect(zoneMatches(zones, placeNewParis, geometry)).toBe(true);
  });

  it('test 3 (reverse): POI on sub-scene, player on parent scene → no match (subtree is asymmetric)', () => {
    const zones = [{ kind: 'poi' as const, poiId: POI_TARSIS_1 }];
    const geometry = geo([POI_TARSIS_1, { system: 'tarsis', scene: 'tarsis_1/new-paris', x: 0, y: 0, z: 0, radiusM: null }]);
    expect(zoneMatches(zones, placeTarsis1, geometry)).toBe(false);
  });

  it('test 4: tarsis_1 vs tarsis_10 (bare prefix, not segment) → no match', () => {
    const zones = [zone(POI_TARSIS_1, 5000)];
    const geometry = geo([POI_TARSIS_1, { ...poiTarsis1, radiusM: 5000, x: 0, y: 0, z: 0 }]);
    expect(zoneMatches(zones, placeTarsis10, geometry)).toBe(false);
  });

  it('test 5: null place → no match', () => {
    const zones = [zone(POI_TARSIS_3, 5000)];
    const geometry = geo([POI_TARSIS_3, poiTarsis3]);
    expect(zoneMatches(zones, null, geometry)).toBe(false);
    expect(zoneMatches(zones, undefined, geometry)).toBe(false);
  });

  it('test 6: POI system "" vs player system "tarsis" → no match', () => {
    const zones = [zone(POI_TARSIS_3, 5000)];
    const geometry = geo([POI_TARSIS_3, { ...poiTarsis3, system: '' }]);
    expect(zoneMatches(zones, placeTarsis3, geometry)).toBe(false);
  });

  it('test 7: player re-pushed at the same place → match is stable', () => {
    const zones = [zone(POI_TARSIS_3, 5000)];
    const geometry = geo([POI_TARSIS_3, poiTarsis3]);
    expect(zoneMatches(zones, placeTarsis3, geometry)).toBe(true);
    expect(zoneMatches(zones, placeTarsis3, geometry)).toBe(true); // same input, same result
  });

  it('space POI vs player in space → match', () => {
    const zones = [zone(POI_SPACE, 5000)];
    const geometry = geo([POI_SPACE, poiSpace]);
    expect(zoneMatches(zones, placeSpace, geometry)).toBe(true);
  });

  it('space POI vs player on a body → no match', () => {
    const zones = [zone(POI_SPACE, 5000)];
    const geometry = geo([POI_SPACE, poiSpace]);
    expect(zoneMatches(zones, placeTarsis3, geometry)).toBe(false);
  });

  it('body POI vs player in space → no match', () => {
    const zones = [zone(POI_TARSIS_3, 5000)];
    const geometry = geo([POI_TARSIS_3, poiTarsis3]);
    expect(zoneMatches(zones, placeSpace, geometry)).toBe(false);
  });

  it('POI without scene → fail-closed (never matches)', () => {
    const radiusZones = [zone(POI_NO_SCENE, 5000)];
    const geometry = geo([POI_NO_SCENE, poiNoScene]);
    expect(zoneMatches(radiusZones, placeTarsis3, geometry)).toBe(false);

    const pointZones = [{ kind: 'poi' as const, poiId: POI_NO_SCENE }];
    expect(zoneMatches(pointZones, placeTarsis3, geometry)).toBe(false);
  });

  it('system zone still matches by system', () => {
    expect(zoneMatches([{ kind: 'system' as const, system: 'tarsis' }], placeTarsis3)).toBe(true);
    expect(zoneMatches([{ kind: 'system' as const, system: 'vega' }], placeTarsis3)).toBe(false);
  });

  it('empty zones are global', () => {
    expect(zoneMatches([], null)).toBe(true);
    expect(zoneMatches([], placeTarsis3)).toBe(true);
  });
});

// ── zoneFilterSql (integration with PGlite) ────────────────────────────────────

describe('zoneFilterSql', () => {
  async function insertMission(title: string, zones: unknown[]) {
    const [row] = await db.insert(missions).values({ title, zones: zones as any }).returning({ id: missions.id });
    return row.id;
  }

  /** Returns the ids of missions visible for a given location + geometry map. */
  async function listing(location: unknown, geometry: PoiGeometryMap): Promise<string[]> {
    const rows = await db.select({ id: missions.id }).from(missions).where(zoneFilterSql(location as any, geometry));
    return rows.map((r) => r.id);
  }

  beforeEach(async () => {
    await db.delete(missions);
  });

  it('test 1 (SQL): same offset on different bodies → no match', async () => {
    await insertMission('on tarsis_4', [zone(POI_TARSIS_4, 5000)]);
    const ids = await listing(placeTarsis3, geo([POI_TARSIS_4, poiTarsis4]));
    expect(ids).toHaveLength(0);
  });

  it('test 2 (SQL): same scene, same offset, small radius → match', async () => {
    await insertMission('on tarsis_3', [zone(POI_TARSIS_3, 100)]);
    const ids = await listing(placeTarsis3, geo([POI_TARSIS_3, poiTarsis3]));
    expect(ids).toHaveLength(1);
  });

  it('test 3 (SQL): segment prefix → match', async () => {
    await insertMission('on tarsis_1', [{ kind: 'poi', poiId: POI_TARSIS_1 }]);
    const ids = await listing(placeNewParis, geo([POI_TARSIS_1, poiTarsis1]));
    expect(ids).toHaveLength(1);
  });

  it('test 4 (SQL): bare prefix tarsis_1 vs tarsis_10 → no match', async () => {
    await insertMission('on tarsis_1', [{ kind: 'poi', poiId: POI_TARSIS_1 }]);
    const ids = await listing(placeTarsis10, geo([POI_TARSIS_1, poiTarsis1]));
    expect(ids).toHaveLength(0);
  });

  it('test 5 (SQL): null place → no match', async () => {
    await insertMission('zoned', [zone(POI_TARSIS_3, 5000)]);
    const ids = await listing(null, geo([POI_TARSIS_3, poiTarsis3]));
    expect(ids).toHaveLength(0);
  });

  it('test 6 (SQL): POI system "" vs player system "tarsis" → no match', async () => {
    await insertMission('outside', [zone(POI_TARSIS_3, 5000)]);
    const ids = await listing(placeTarsis3, geo([POI_TARSIS_3, { ...poiTarsis3, system: '' }]));
    expect(ids).toHaveLength(0);
  });

  it('space POI (SQL): player in space → match; player on body → no match', async () => {
    await insertMission('space zone', [zone(POI_SPACE, 5000)]);
    const geometry = geo([POI_SPACE, poiSpace]);
    expect(await listing(placeSpace, geometry)).toHaveLength(1);
    expect(await listing(placeTarsis3, geometry)).toHaveLength(0);
  });
});

// ── End-to-end: player-states + listing + verify ───────────────────────────────

describe('end-to-end zone matching', () => {
  beforeAll(async () => {
    ({ baseUrl, server } = await startTestApp());
  });

  afterAll(() => {
    server.close();
  });

  beforeEach(async () => {
    await db.delete(missions);
    inventory.resolvePois.mockReset();
    social.getPlayerPresence.mockReset();
  });

  it('mission visible on the right planet, hidden on the other', async () => {
    // Resolve two POIs on different planets.
    inventory.resolvePois.mockImplementation(async (ids: string[]) => {
      const all: Record<string, any> = {
        [POI_TARSIS_3]: { id: POI_TARSIS_3, ...poiTarsis3 },
        [POI_TARSIS_4]: { id: POI_TARSIS_4, ...poiTarsis4 },
      };
      return new Map(ids.filter((id) => all[id]).map((id) => [id, all[id]]));
    });

    // Create a mission zoned on the tarsis_3 POI.
    const { status: createStatus, body: created } = await call('/api/internal/missions', 'POST', {
      title: 'Planet-locked mission',
      zones: [zone(POI_TARSIS_3, 5000)],
      objectives: [{ type: 'manual', title: 'Explore', targetQuantity: 1 }],
    });
    expect(createStatus).toBe(201);
    const missionId = created.mission.id as string;

    // Player on tarsis_4 (wrong planet): accept must fail with OUT_OF_ZONE.
    await upsertPlayerStates([{ playerId: PLAYER, place: stateTarsis4 }]);
    social.getPlayerPresence.mockResolvedValue({ location: placeTarsis4 });
    const { status: rejectStatus, body: rejectBody } = await call(`/api/missions/${missionId}/accept`, 'POST');
    expect(rejectStatus).toBe(403);
    expect(rejectBody.error).toBe('OUT_OF_ZONE');

    // Player moves to tarsis_3 (right planet): accept must succeed.
    await upsertPlayerStates([{ playerId: PLAYER, place: stateTarsis3 }]);
    social.getPlayerPresence.mockResolvedValue({ location: placeTarsis3 });
    const { status: acceptStatus } = await call(`/api/missions/${missionId}/accept`, 'POST');
    expect(acceptStatus).toBe(201);
  });

  it('mission with a space POI is visible in open space only', async () => {
    inventory.resolvePois.mockImplementation(async (ids: string[]) => {
      const all: Record<string, any> = { [POI_SPACE]: { id: POI_SPACE, ...poiSpace } };
      return new Map(ids.filter((id) => all[id]).map((id) => [id, all[id]]));
    });

    const { body: created } = await call('/api/internal/missions', 'POST', {
      title: 'Space mission',
      zones: [zone(POI_SPACE, 5000)],
      objectives: [{ type: 'manual', title: 'Explore space', targetQuantity: 1 }],
    });
    const missionId = created.mission.id as string;

    // Player on tarsis_3 (body): accept must fail with OUT_OF_ZONE.
    social.getPlayerPresence.mockResolvedValue({ location: placeTarsis3 });
    const { status: ko, body: koBody } = await call(`/api/missions/${missionId}/accept`, 'POST');
    expect(ko).toBe(403);
    expect(koBody.error).toBe('OUT_OF_ZONE');

    // Player in open space: accept must succeed.
    social.getPlayerPresence.mockResolvedValue({ location: placeSpace });
    const { status: ok } = await call(`/api/missions/${missionId}/accept`, 'POST');
    expect(ok).toBe(201);
  });
});
