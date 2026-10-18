/**
 * `POST /api/internal/poi-usage` — the POI lock Inventory asks for before it lets a player
 * move a POI. Only an **active** assignment locks: a mission created but never accepted
 * (or whose assignments are terminal) leaves the POI free.
 */
import type { Server } from 'http';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

const { db } = await import('../src/db/connection.js');
const { missionAssignments } = await import('../src/db/schema/index.js');

/** A POI that only exists as an opaque id (Inventory is not called by this endpoint). */
const POI = '3f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d';
const OTHER_POI = 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d';
const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';

let server: Server;
let baseUrl: string;

/** Internal API call with a capability role. */
async function call(path: string, roles = 'mission:read', method = 'GET', payload?: unknown) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-roles': roles },
    ...(payload === undefined ? {} : { body: JSON.stringify(payload) }),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

/** Creates an internal mission and returns its id. */
async function createMission(overrides: Record<string, unknown> = {}): Promise<string> {
  const res = await call('/missions', 'mission:write', 'POST', {
    title: 'Contrat de test',
    category: 'divers',
    objectives: [{ type: 'manual', title: 'Balisage' }],
    ...overrides,
  });
  expect(res.status).toBe(201);
  return res.body.mission.id as string;
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

beforeEach(async () => {
  await db.delete(missionAssignments);
});

describe('POST /api/internal/poi-usage', () => {
  it('requires the mission:read capability', async () => {
    const denied = await call('/poi-usage', 'mission:write', 'POST', { poiIds: [POI] });
    expect(denied.status).toBe(403);

    const allowed = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [POI] });
    expect(allowed.status).toBe(200);
  });

  it('rejects an empty poiIds list', async () => {
    const res = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [] });
    expect(res.status).toBe(400);
  });

  it('leaves a POI free when the mission has no assignee', async () => {
    const missionId = await createMission({ zones: [{ kind: 'poi', poiId: POI }] });

    const res = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [POI] });
    expect(res.status).toBe(200);
    expect(res.body[POI]).toBeUndefined();
    expect(missionId).toBeTruthy();
  });

  it('locks a POI referenced by a zone once a mission is assigned', async () => {
    const missionId = await createMission({ zones: [{ kind: 'poi', poiId: POI }] });
    // `npc` skips the zone gate (no presence to check); the lock applies to any holder type.
    const assigned = await call(`/missions/${missionId}/assign`, 'mission:write', 'POST', {
      playerId: PLAYER,
      holderType: 'npc',
    });
    expect(assigned.status).toBe(201);

    const res = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [POI] });
    expect(res.body[POI]).toEqual([missionId]);
  });

  it('leaves the POI free again once the assignment is terminal', async () => {
    const missionId = await createMission({ zones: [{ kind: 'poi', poiId: POI }] });
    await call(`/missions/${missionId}/assign`, 'mission:write', 'POST', {
      playerId: PLAYER,
      holderType: 'npc',
    });
    await db
      .update(missionAssignments)
      .set({ status: 'completed' })
      .where();

    const res = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [POI] });
    expect(res.body[POI]).toBeUndefined();
  });

  it('ignores a system zone (it references no POI)', async () => {
    const missionId = await createMission({ zones: [{ kind: 'system', system: 'tarsis' }] });
    await call(`/missions/${missionId}/assign`, 'mission:write', 'POST', {
      playerId: PLAYER,
      holderType: 'npc',
    });

    const res = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [POI] });
    expect(res.body[POI]).toBeUndefined();
  });

  it('locks a POI referenced by an objective delivery zone', async () => {
    const missionId = await createMission({
      objectives: [
        {
          type: 'deliver_material',
          title: 'Livraison',
          params: { goodType: 'metal', zone: { kind: 'poi', poiId: POI } },
        },
      ],
    });
    await call(`/missions/${missionId}/assign`, 'mission:write', 'POST', {
      playerId: PLAYER,
      holderType: 'npc',
    });

    const res = await call('/poi-usage', 'mission:read', 'POST', { poiIds: [POI] });
    expect(res.body[POI]).toEqual([missionId]);
  });

  it('locks a POI referenced by an objective transport source', async () => {
    const missionId = await createMission({
      objectives: [
        {
          type: 'transport',
          title: 'Acheminement',
          params: {
            goodType: 'metal',
            source: { kind: 'poi', poiId: OTHER_POI },
            zone: { kind: 'poi', poiId: POI },
          },
        },
      ],
    });
    await call(`/missions/${missionId}/assign`, 'mission:write', 'POST', {
      playerId: PLAYER,
      holderType: 'npc',
    });

    const res = await call('/poi-usage', 'mission:read', 'POST', {
      poiIds: [POI, OTHER_POI],
    });
    expect(res.body[POI]).toEqual([missionId]);
    expect(res.body[OTHER_POI]).toEqual([missionId]);
  });

  it('reports no lock for an unknown POI id', async () => {
    const missionId = await createMission({ zones: [{ kind: 'poi', poiId: POI }] });
    await call(`/missions/${missionId}/assign`, 'mission:write', 'POST', {
      playerId: PLAYER,
      holderType: 'npc',
    });

    const res = await call('/poi-usage', 'mission:read', 'POST', {
      poiIds: [OTHER_POI],
    });
    expect(res.body[OTHER_POI]).toBeUndefined();
  });
});
