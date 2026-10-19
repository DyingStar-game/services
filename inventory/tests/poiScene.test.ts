/**
 * POI creation and geometry-immutability rules: system/scene coherence (including the
 * reserved `space` scene) and the read-only player route. Economy and Social are mocked;
 * the database is an in-process PGlite migrated from `./drizzle`.
 */
import type { Server } from 'http';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

const missionClient = vi.hoisted(() => ({
  inUse: new Set<string>(),
  fail: false,
}));
vi.mock('../src/services/mission.client.js', () => ({
  isMissionConfigured: () => true,
  isPoiInUse: async (poiId: string) => {
    if (missionClient.fail) throw new Error('mission down');
    return missionClient.inUse.has(poiId);
  },
  findPoisInUse: async () => new Map<string, string[]>(),
}));

const socialClient = vi.hoisted(() => ({
  allow: false,
}));
vi.mock('../src/services/social.client.js', () => ({
  isSocialConfigured: () => true,
  authorizeAction: async () => ({ allowed: socialClient.allow, reason: socialClient.allow ? null : 'not_member' }),
}));

const { db } = await import('../src/db/connection.js');
const { inventoryPois } = await import('../src/db/schema/index.js');
const { eq } = await import('drizzle-orm');

const byId = (id: string) => eq(inventoryPois.id, id);

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const OTHER = '1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e';

let server: Server;
let baseUrl: string;

/** Authenticated API call as PLAYER (or OTHER). */
async function call(
  path: string,
  method: string,
  body?: unknown,
  playerId: string = PLAYER,
): Promise<{ status: number; body: any }> {
  const res = await fetch(`${baseUrl}${path}`, {
    method,
    headers: { 'content-type': 'application/json', 'x-test-player-id': playerId },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: res.status, body: await res.json().catch(() => null) };
}

beforeAll(async () => {
  ({ baseUrl, server } = await startTestApp());
});

afterAll(() => {
  server.close();
});

beforeEach(async () => {
  await db.delete(inventoryPois);
});

describe('POST /api/me/pois — system/scene coherence', () => {
  it('accepts system + body scene (underscore)', async () => {
    const { status, body } = await call('/api/me/pois', 'POST', {
      name: 'Surface base',
      system: 'tarsis',
      scene: 'tarsis_3',
      x: 120,
      y: 6_000_000,
      z: -35,
    });
    expect(status).toBe(201);
    expect(body.system).toBe('tarsis');
    expect(body.scene).toBe('tarsis_3');
  });

  it('accepts system + sub-scene (slash)', async () => {
    const { status } = await call('/api/me/pois', 'POST', {
      name: 'Space station',
      system: 'tarsis',
      scene: 'tarsis/new-paris',
      x: 1000,
      y: 0,
      z: 0,
    });
    expect(status).toBe(201);
  });

  it('accepts system + space scene', async () => {
    const { status, body } = await call('/api/me/pois', 'POST', {
      name: 'Deep-space beacon',
      system: 'tarsis',
      scene: 'space',
      x: 1_000_000,
      y: 0,
      z: 0,
    });
    expect(status).toBe(201);
    expect(body.scene).toBe('space');
  });

  it('accepts scene === system', async () => {
    const { status } = await call('/api/me/pois', 'POST', {
      name: 'System centre',
      system: 'tarsis',
      scene: 'tarsis',
      x: 0,
      y: 0,
      z: 0,
    });
    expect(status).toBe(201);
  });

  it('accepts system + body scene with underscore then slash', async () => {
    const { status } = await call('/api/me/pois', 'POST', {
      name: 'Sub-scene under body',
      system: 'tarsis',
      scene: 'tarsis_3/district-9',
      x: 0,
      y: 0,
      z: 0,
    });
    expect(status).toBe(201);
  });

  it('rejects scene that does not belong to the system', async () => {
    const { status, body } = await call('/api/me/pois', 'POST', {
      name: 'Orphan',
      system: 'tarsis',
      scene: 'truc',
      x: 0,
      y: 0,
      z: 0,
    });
    expect(status).toBe(400);
    expect(body.error).toBe('POI_SCENE_SYSTEM_MISMATCH');
  });

  it('rejects a body from another system', async () => {
    const { status, body } = await call('/api/me/pois', 'POST', {
      name: 'Cross-system',
      system: 'tarsis',
      scene: 'vega_1',
      x: 0,
      y: 0,
      z: 0,
    });
    expect(status).toBe(400);
    expect(body.error).toBe('POI_SCENE_SYSTEM_MISMATCH');
  });

  it('rejects scene from a system whose name is a prefix (underscore, not segment)', async () => {
    const { status } = await call('/api/me/pois', 'POST', {
      name: 'Wrong prefix',
      system: 'tarsis',
      scene: 'tarsisx',
      x: 0,
      y: 0,
      z: 0,
    });
    expect(status).toBe(400);
  });
});

describe('PATCH /api/me/pois/:poiId — geometry is editable by its creator', () => {
  beforeEach(() => {
    missionClient.inUse.clear();
    missionClient.fail = false;
    socialClient.allow = false;
  });

  /** Creates a POI and returns its id. */
  async function create(poi: Record<string, unknown> = {}, playerId: string = PLAYER): Promise<string> {
    const { body } = await call(
      '/api/me/pois',
      'POST',
      { name: 'Editable', system: 'tarsis', scene: 'tarsis_3', x: 0, y: 0, z: 0, ...poi },
      playerId,
    );
    return body.id;
  }

  it('lets the creator move their own POI (test 13)', async () => {
    const id = await create();
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { x: 42, radiusM: 500 });
    expect(status).toBe(200);
    expect(body.x).toBe(42);
    expect(body.radiusM).toBe(500);
  });

  it('refuses the geometry of a POI the caller did not create (test 14)', async () => {
    // OTHER is the owner: POIs are owned by their creator at creation time, so simulate
    // the ownership-transfer case where the owner changed but `createdBy` did not.
    const id = await create();
    await db.update(inventoryPois).set({ ownerId: OTHER }).where(byId(id));
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { x: 1 }, OTHER);
    expect(status).toBe(403);
    expect(body.error).toBe('NOT_POI_OWNER');
    // …but renaming stays allowed for the new owner.
    const rename = await call(`/api/me/pois/${id}`, 'PATCH', { name: 'Renamed' }, OTHER);
    expect(rename.status).toBe(200);
    expect(rename.body.name).toBe('Renamed');
  });

  it('lets a corporation manager move a POI they did not create (test 15)', async () => {
    // Created by OTHER; the acting player is a manager of the owning corporation.
    const id = await create({}, OTHER);
    await db.update(inventoryPois).set({ ownerType: 'corporation', ownerId: PLAYER }).where(byId(id));
    socialClient.allow = true;
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { x: 7 });
    expect(status).toBe(200);
    expect(body.x).toBe(7);
  });

  it('refuses geometry to a corporation member without the manage permission (test 16)', async () => {
    const id = await create({}, OTHER);
    await db.update(inventoryPois).set({ ownerType: 'corporation', ownerId: PLAYER }).where(byId(id));
    socialClient.allow = false;
    const { status } = await call(`/api/me/pois/${id}`, 'PATCH', { x: 7 });
    expect(status).toBe(403);
  });

  it('rejects a geometry move while the POI is the target of an accepted mission (test 17)', async () => {
    const id = await create();
    missionClient.inUse.add(id);
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { x: 1 });
    expect(status).toBe(400);
    expect(body.error).toBe('POI_IN_USE');
    // …but a non-geometry patch still goes through.
    const rename = await call(`/api/me/pois/${id}`, 'PATCH', { name: 'Renamed' });
    expect(rename.status).toBe(200);
  });

  it('fails closed when the Mission check cannot be performed (test 18)', async () => {
    const id = await create();
    missionClient.fail = true;
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { x: 1 });
    expect(status).toBe(503);
    expect(body.error).toBe('POI_IN_USE_CHECK_FAILED');
  });

  it('still rejects an incoherent system/scene on a geometry patch (test 19)', async () => {
    const id = await create();
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { system: 'kepler', scene: 'tarsis_3' });
    expect(status).toBe(400);
    expect(body.error).toBe('POI_SCENE_SYSTEM_MISMATCH');
  });

  it('accepts a patch with name only (test 20)', async () => {
    const id = await create();
    const { status, body } = await call(`/api/me/pois/${id}`, 'PATCH', { name: 'Renamed' });
    expect(status).toBe(200);
    expect(body.name).toBe('Renamed');
  });
});

describe('GET /api/me/pois — a mission POI is not a player place (tests 21-24)', () => {
  beforeEach(() => {
    missionClient.inUse.clear();
    missionClient.fail = false;
    socialClient.allow = false;
  });

  /** Creates a POI (optionally for a non-player holder) and returns the response body. */
  async function create(owner?: Record<string, unknown>, playerId: string = PLAYER): Promise<any> {
    const { status, body } = await call(
      '/api/me/pois',
      'POST',
      {
        name: 'Mission zone',
        system: 'tarsis',
        scene: 'tarsis_3',
        x: 10,
        y: 20,
        z: 30,
        ...(owner ? { owner } : {}),
      },
      playerId,
    );
    expect(status).toBe(201);
    return body;
  }

  /** Ids returned by the caller's own POI list. */
  async function listedIds(playerId: string): Promise<string[]> {
    const { status, body } = await call('/api/me/pois?limit=50', 'GET', undefined, playerId);
    expect(status).toBe(200);
    return body.items.map((poi: { id: string }) => poi.id);
  }

  it('keeps a mission POI out of the list of the player who created it (test 21)', async () => {
    const own = await create();
    const mission = await create({ type: 'mission', id: OTHER });
    expect(mission.ownerType).toBe('mission');
    expect(mission.createdBy).toBe(PLAYER);

    const ids = await listedIds(PLAYER);
    expect(ids).toContain(own.id); // control: the list itself still works
    expect(ids).not.toContain(mission.id);
  });

  it('keeps a mission POI out of every player list, whoever created it (test 22)', async () => {
    // Created by OTHER: under the old scope it landed in OTHER own list too.
    const theirs = await create({ type: 'mission', id: PLAYER }, OTHER);
    expect(theirs.createdBy).toBe(OTHER);
    expect(await listedIds(OTHER)).not.toContain(theirs.id);
    expect(await listedIds(PLAYER)).not.toContain(theirs.id);
  });

  it('reads a mission POI by id for its creator and for anyone else (test 23)', async () => {
    const mission = await create({ type: 'mission', id: OTHER });

    const byCreator = await call(`/api/me/pois/${mission.id}`, 'GET', undefined, PLAYER);
    expect(byCreator.status).toBe(200);
    expect(byCreator.body.id).toBe(mission.id);
    expect(byCreator.body.name).toBe('Mission zone');

    // The whole point: the client shows a mission point it did not create.
    const byStranger = await call(`/api/me/pois/${mission.id}`, 'GET', undefined, OTHER);
    expect(byStranger.status).toBe(200);
    expect(byStranger.body.id).toBe(mission.id);
    expect(byStranger.body.name).toBe('Mission zone');
  });

  it('leaves an ordinary player POI in the creator list only (test 24)', async () => {
    const own = await create();
    expect(await listedIds(PLAYER)).toContain(own.id);
    expect(await listedIds(OTHER)).not.toContain(own.id);
  });
});
