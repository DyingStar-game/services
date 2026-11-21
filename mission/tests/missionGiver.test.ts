/**
 * In-fiction mission giver (`giver: {faction, division}`): stored on service-created
 * missions, returned by the listings and usable as a filter; absent = null on both fields.
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
    title: 'Livraison au village',
    category: 'delivery',
    objectives: [{ type: 'manual', title: 'Livrer la caisse' }],
    ...overrides,
  };
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

describe('mission giver', () => {
  it('stores and returns the faction and division', async () => {
    const created = await call('/missions', 'mission:write', 'POST',
      missionBody({ giver: { faction: 'ares', division: 'hauling' } }));
    expect(created.status).toBe(201);
    expect(created.body.mission.giverFaction).toBe('ares');
    expect(created.body.mission.giverDivision).toBe('hauling');
  });

  it('leaves both fields null without a giver, and the division null for a bare faction', async () => {
    const bare = await call('/missions', 'mission:write', 'POST', missionBody());
    expect(bare.status).toBe(201);
    expect(bare.body.mission.giverFaction).toBeNull();
    expect(bare.body.mission.giverDivision).toBeNull();

    const faction = await call('/missions', 'mission:write', 'POST', missionBody({ giver: { faction: 'ares' } }));
    expect(faction.body.mission.giverFaction).toBe('ares');
    expect(faction.body.mission.giverDivision).toBeNull();
  });

  it('refuses a giver id that is not a slug', async () => {
    const res = await call('/missions', 'mission:write', 'POST',
      missionBody({ giver: { faction: 'ARES Corp', division: 'mining' } }));
    expect(res.status).toBe(400);
  });

  it('filters the listing on the faction and the division', async () => {
    await call('/missions', 'mission:write', 'POST',
      missionBody({ title: 'Minerai', category: 'mining', giver: { faction: 'ares', division: 'mining' } }));
    await call('/missions', 'mission:write', 'POST',
      missionBody({ title: 'Autre faction', giver: { faction: 'freeport', division: 'mining' } }));

    const mining = await call('/missions?giverFaction=ares&giverDivision=mining');
    expect(mining.status).toBe(200);
    expect(mining.body.missions.map((m: any) => m.title)).toEqual(['Minerai']);

    const ares = await call('/missions?giverFaction=ares');
    expect(ares.body.missions.every((m: any) => m.giverFaction === 'ares')).toBe(true);
    expect(ares.body.missions.length).toBeGreaterThanOrEqual(3);
  });
});
