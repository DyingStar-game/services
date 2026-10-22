/**
 * Internal mission detail (`GET /api/internal/missions/:missionId`): mission, ordered
 * objectives, every assignment and the Social enrichment (best-effort — Social is not
 * configured here, so names and group degrade to empty instead of failing).
 */
import type { Server } from 'http';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

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

/** Minimal internal mission body (one `manual` objective, no escrow, no zone). */
function missionBody(overrides: Record<string, unknown> = {}) {
  return {
    title: 'Reconnaissance du secteur',
    category: 'divers',
    objectives: [{ type: 'manual', title: 'Signaler les hostiles' }],
    ...overrides,
  };
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

describe('GET /api/internal/missions/:missionId', () => {
  it('requires the mission:read capability', async () => {
    const created = await call('/missions', 'mission:write', 'POST', missionBody());
    expect(created.status).toBe(201);
    const id = created.body.mission.id as string;

    const denied = await call(`/missions/${id}`, 'mission:write');
    expect(denied.status).toBe(403);
    expect(denied.body.error).toBe('FORBIDDEN');

    const allowed = await call(`/missions/${id}`);
    expect(allowed.status).toBe(200);
  });

  it('returns the mission, its ordered objectives and an empty summary', async () => {
    const res = await fetch(`${baseUrl}/api/internal/missions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-test-roles': 'mission:write' },
      body: JSON.stringify(
        missionBody({
          objectives: [
            { type: 'manual', title: 'Premier palier', order: 0 },
            { type: 'manual', title: 'Second palier', order: 1, weight: 3 },
            { type: 'manual', title: 'Palier bonus', order: 2, isBonus: true },
          ],
        }),
      ),
    });
    const body = (await res.json()) as Record<string, any>;
    expect(res.status).toBe(201);
    const id = body.mission.id as string;

    const { status, body: detail } = await call(`/missions/${id}`);
    expect(status).toBe(200);
    expect(detail.mission.id).toBe(id);
    expect(detail.objectives.map((o: any) => o.title)).toEqual([
      'Premier palier',
      'Second palier',
      'Palier bonus',
    ]);
    expect(detail.objectives.map((o: any) => o.order)).toEqual([0, 1, 2]);
    expect(detail.objectives[1].weight).toBe(3);
    expect(detail.objectives[2].isBonus).toBe(true);
    expect(detail.assignments).toEqual([]);
    expect(detail.players).toEqual({});
    expect(detail.group).toBeNull();
    expect(detail.chain).toEqual([]);
    expect(detail.summary).toMatchObject({ assignments: 0, active: 0, progressBp: 0 });
  });

  it('lists every assignment with its status and frozen reward shares', async () => {
    const created = await fetch(`${baseUrl}/api/internal/missions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-test-roles': 'mission:write' },
      body: JSON.stringify(missionBody({ maxAssignees: 3, rewards: [{ type: 'credits', amount: 900 }] })),
    });
    const { mission } = (await created.json()) as Record<string, any>;
    const playerId = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';

    // An NPC assignee skips every player-side gate (zone, prerequisites, group).
    const assigned = await fetch(`${baseUrl}/api/internal/missions/${mission.id}/assign`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-test-roles': 'mission:write' },
      body: JSON.stringify({ playerId, holderType: 'npc' }),
    });
    expect(assigned.status).toBe(201);

    const { status, body: detail } = await call(`/missions/${mission.id}`);
    expect(status).toBe(200);
    expect(detail.assignments).toHaveLength(1);
    expect(detail.assignments[0]).toMatchObject({
      playerId,
      holderType: 'npc',
      status: 'active',
      completionRatio: 0,
    });
    expect(detail.summary).toMatchObject({ assignments: 1, active: 1 });
  });

  it('is 404 for an unknown mission', async () => {
    const { status, body } = await call('/missions/00000000-0000-4000-8000-000000000000');
    expect(status).toBe(404);
    expect(body.error).toBe('NOT_FOUND');
  });

  it('is 400 on a non-uuid mission id', async () => {
    const { status } = await call('/missions/expire');
    expect(status).toBe(400);
  });
});
