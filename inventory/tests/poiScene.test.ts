/**
 * POI creation and geometry-immutability rules: system/scene coherence (including the
 * reserved `space` scene) and the read-only player route. Economy and Social are mocked;
 * the database is an in-process PGlite migrated from `./drizzle`.
 */
import type { Server } from 'http';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

const { db } = await import('../src/db/connection.js');
const { inventoryPois } = await import('../src/db/schema/index.js');

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

describe('PATCH /api/me/pois/:poiId — geometry is read-only', () => {
  let poiId: string;

  beforeEach(async () => {
    const { body } = await call('/api/me/pois', 'POST', {
      name: 'Immutable point',
      system: 'tarsis',
      scene: 'tarsis_3',
      x: 0,
      y: 0,
      z: 0,
    });
    poiId = body.id;
  });

  it('rejects a patch with x', async () => {
    const { status, body } = await call(`/api/me/pois/${poiId}`, 'PATCH', { x: 1 });
    expect(status).toBe(400);
    expect(body.error).toBe('POI_GEOMETRY_READONLY');
  });

  it('rejects a patch with radiusM', async () => {
    const { status, body } = await call(`/api/me/pois/${poiId}`, 'PATCH', { radiusM: 500 });
    expect(status).toBe(400);
    expect(body.error).toBe('POI_GEOMETRY_READONLY');
  });

  it('accepts a patch with name only', async () => {
    const { status, body } = await call(`/api/me/pois/${poiId}`, 'PATCH', { name: 'Renamed' });
    expect(status).toBe(200);
    expect(body.name).toBe('Renamed');
  });
});
