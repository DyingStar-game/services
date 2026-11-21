/**
 * Presence batch in the local-body frame: a presence with a `scene` and a local position
 * is accepted and read back identically. The database is an in-process PGlite migrated
 * from `./drizzle`; Valkey is not configured (cache falls back to the database).
 */
import type { Server } from 'http';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

const { db } = await import('../src/db/connection.js');
const { playerProfiles, playerPresence } = await import('../src/db/schema/index.js');

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const PROFILE_ROLES = 'social:profile:read,social:profile:write';

let server: Server;
let baseUrl: string;

/** Internal API call. */
async function call(
  path: string,
  method: string,
  body?: unknown,
  roles: string = PROFILE_ROLES,
): Promise<{ status: number; body: any }> {
  const res = await fetch(`${baseUrl}${path}`, {
    method,
    headers: { 'content-type': 'application/json', 'x-test-client': 'svc-game', 'x-test-roles': roles },
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
  await db.delete(playerPresence);
  await db.delete(playerProfiles);
  // A profile must exist for a presence to be accepted.
  await db.insert(playerProfiles).values({ playerId: PLAYER, displayName: 'Tester' });
});

describe('PUT /api/internal/players/presence/batch — local-body frame', () => {
  it('accepts a presence with scene + local position and reads it back identically', async () => {
    const location = {
      system: 'tarsis',
      scene: 'tarsis_3',
      position: { x: 120.0, y: 6_000_000.0, z: -35.0 },
    };
    const { status, body } = await call('/api/internal/players/presence/batch', 'PUT', {
      players: [{ playerId: PLAYER, status: 'online', location }],
    });
    expect(status).toBe(200);
    expect(body.refreshed).toBe(1);

    // Read back through the single-player GET route.
    const { status: getStatus, body: presence } = await call(
      `/api/internal/players/${PLAYER}/presence`,
      'GET',
    );
    expect(getStatus).toBe(200);
    expect(presence.status).toBe('online');
    expect(presence.location).toEqual(location);
  });

  it('accepts a presence in open space (scene: "space")', async () => {
    const location = {
      system: 'tarsis',
      scene: 'space',
      position: { x: 1_000_000.0, y: 0, z: 0 },
    };
    const { status } = await call('/api/internal/players/presence/batch', 'PUT', {
      players: [{ playerId: PLAYER, status: 'online', location }],
    });
    expect(status).toBe(200);

    const { body: presence } = await call(`/api/internal/players/${PLAYER}/presence`, 'GET');
    expect(presence.location).toEqual(location);
  });

  it('rejects a batch with no players', async () => {
    const { status, body } = await call('/api/internal/players/presence/batch', 'PUT', {
      players: [],
    });
    expect(status).toBe(400);
  });

  it('reports unknown player ids', async () => {
    const { status, body } = await call('/api/internal/players/presence/batch', 'PUT', {
      players: [{ playerId: '11111111-1111-4111-8111-111111111111', status: 'online' }],
    });
    expect(status).toBe(200);
    expect(body.refreshed).toBe(0);
    expect(body.unknown).toEqual(['11111111-1111-4111-8111-111111111111']);
  });
});
