/**
 * Anti-stacking: a player may only hold `MISSION_MAX_ACTIVE_ASSIGNMENTS` active
 * assignments at once. The environment is set before the app is imported, so the cap is
 * visible to the service.
 */
import type { Server } from 'http';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

process.env.MISSION_MAX_ACTIVE_ASSIGNMENTS = '2';

const { startTestApp } = await import('./helpers/testApp.js');

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const ALL_ROLES = 'mission:read,mission:write,mission:complete,mission:progress';

let server: Server;
let baseUrl: string;

async function call(path: string, method = 'GET', body?: unknown) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-roles': ALL_ROLES },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

describe('MISSION_MAX_ACTIVE_ASSIGNMENTS', () => {
  it('refuses a third concurrent assignment', async () => {
    const ids: string[] = [];
    for (const title of ['A', 'B', 'C']) {
      const created = await call('/missions', 'POST', {
        title,
        category: 'exploration',
        objectives: [{ type: 'manual', title: `Visit ${title}`, targetQuantity: 1 }],
      });
      expect(created.status).toBe(201);
      ids.push(created.body.mission.id as string);
    }

    const first = await call(`/missions/${ids[0]}/assign`, 'POST', { playerId: PLAYER });
    expect(first.status).toBe(201);
    const second = await call(`/missions/${ids[1]}/assign`, 'POST', { playerId: PLAYER });
    expect(second.status).toBe(201);

    const third = await call(`/missions/${ids[2]}/assign`, 'POST', { playerId: PLAYER });
    expect(third.status).toBe(409);
    expect(third.body.error).toBe('ASSIGNMENT_LIMIT_REACHED');
  });
});
