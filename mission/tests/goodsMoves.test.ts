/**
 * Warehouse deliveries: a `via: 'warehouse'` objective counts the goods the game server reports
 * as moved INTO the warehouse of its zone (a crate shelved in a cargo depot), not what the
 * assignee carries while walking in. A transport counts its delivery only after a load at its
 * source warehouse, and may deliver another good than it loaded (ore loaded, ingots delivered).
 * A mission that does not auto-complete is turned in by the game server, never by the player.
 * Economy, Inventory and Social are mocked; the database is an in-process PGlite.
 */
import type { Server } from 'http';
import { and, eq } from 'drizzle-orm';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

const MINE_POI = '3c1d2b4a-5e6f-4a7b-8c9d-0e1f2a3b4c5d';
const FACTORY_POI = '4d2e3c5b-6f7a-4b8c-9d0e-1f2a3b4c5d6e';
const MINE_DEPOT = 'aaaaaaaa-1111-4111-8111-111111111111';
const FACTORY_DEPOT = 'bbbbbbbb-2222-4222-8222-222222222222';

const inventory = vi.hoisted(() => ({
  getStack: vi.fn(),
  resolvePois: vi.fn(),
}));
const economy = vi.hoisted(() => ({ creditHolder: vi.fn(), debitHolder: vi.fn() }));
const social = vi.hoisted(() => ({ notifyPlayer: vi.fn() }));

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
vi.mock('../src/services/inventory.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/inventory.client.js')>()),
  getStack: inventory.getStack,
  resolvePois: inventory.resolvePois,
}));
vi.mock('../src/services/economy.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/economy.client.js')>()),
  creditHolder: economy.creditHolder,
  debitHolder: economy.debitHolder,
}));
vi.mock('../src/services/social.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/social.client.js')>()),
  notifyPlayer: social.notifyPlayer,
  addStanding: vi.fn().mockResolvedValue({}),
  getStanding: vi.fn().mockResolvedValue({}),
}));

const { db } = await import('../src/db/connection.js');
const { missionAssignments } = await import('../src/db/schema/index.js');
const { applyMove } = await import('../src/services/goodsMoves.service.js');

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const ROLES = 'mission:read,mission:write,mission:complete,mission:progress';
const WAREHOUSES = new Map([
  [MINE_POI, MINE_DEPOT],
  [FACTORY_POI, FACTORY_DEPOT],
]);

let server: Server;
let baseUrl: string;

async function call(path: string, method = 'GET', body?: unknown) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-roles': ROLES },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

async function playerCall(path: string, method = 'POST', body: unknown = {}) {
  const res = await fetch(`${baseUrl}/api/missions${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-player-id': PLAYER },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

async function createMission(objective: Record<string, unknown>, autoComplete = false) {
  const { status, body } = await call('/missions', 'POST', {
    title: 'Ore run',
    category: 'transport',
    autoComplete,
    objectives: [{ title: 'Deliver', targetQuantity: 20, ...objective }],
    rewards: [{ type: 'credits', amount: 500 }],
  });
  expect(status).toBe(201);
  const id = body.mission.id as string;
  expect((await call(`/missions/${id}/assign`, 'POST', { playerId: PLAYER, holderType: 'npc' })).status).toBe(201);
  return id;
}

let moveCount = 0;
function move(direction: 'drop' | 'load', goodType: string, warehouseId: string, quantity = 20) {
  moveCount += 1;
  return call('/missions/goods-moves', 'POST', {
    playerId: PLAYER,
    goodType,
    quantity,
    direction,
    warehouseId,
    key: `crate-${moveCount}`,
  });
}

async function objectiveOf(missionId: string) {
  return (await call(`/missions/${missionId}`)).body.objectives[0] as Record<string, any>;
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => server?.close());

beforeEach(async () => {
  await db.delete(missionAssignments);
  inventory.getStack.mockReset().mockResolvedValue({ carried: 20, available: 0 });
  inventory.resolvePois.mockReset().mockImplementation(async (ids: string[]) => {
    const out = new Map();
    for (const id of ids) {
      const depot = WAREHOUSES.get(id);
      if (depot) out.set(id, { id, system: null, scene: 'tarsis_3', x: 0, y: 0, z: 0, radiusM: 40, metadata: { persistence: depot } });
    }
    return out;
  });
  economy.creditHolder.mockReset().mockResolvedValue({ transaction: { id: 1 } });
  social.notifyPlayer.mockReset().mockResolvedValue(undefined);
});

describe('warehouse deliveries', () => {
  it('a delivery counts the goods shelved in its warehouse, not the goods carried in its zone', async () => {
    const id = await createMission({
      type: 'deliver_material',
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    await call(`/missions/${id}/verify`, 'POST', { playerId: PLAYER });
    expect((await objectiveOf(id)).currentProgress).toBe(0); // carrying 20 is not delivering

    await move('drop', 'ore.gold', FACTORY_DEPOT); // the wrong warehouse
    expect((await objectiveOf(id)).currentProgress).toBe(0);
    await move('drop', 'ore.gold', MINE_DEPOT);
    const objective = await objectiveOf(id);
    expect(objective.currentProgress).toBe(20);
    expect(objective.status).toBe('completed');
  });

  it('a transport counts its delivery only after a load at its source, and may change the good', async () => {
    const id = await createMission({
      type: 'transport',
      params: {
        goodType: 'ore.gold',
        deliverGoodType: 'ingot.gold',
        source: { kind: 'poi', poiId: MINE_POI },
        zone: { kind: 'poi', poiId: FACTORY_POI },
        via: 'warehouse',
      },
    });
    await move('drop', 'ingot.gold', FACTORY_DEPOT); // nothing was loaded yet
    expect((await objectiveOf(id)).currentProgress).toBe(0);
    await move('load', 'ore.gold', MINE_DEPOT);
    await move('drop', 'ingot.gold', FACTORY_DEPOT);
    expect((await objectiveOf(id)).currentProgress).toBe(20);
  });

  it('a retried move counts once', () => {
    const objective = {
      type: 'deliver_material' as const,
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
      state: {},
      targetQuantity: 20,
    };
    const m = { playerId: PLAYER, goodType: 'ore.gold', quantity: 5, direction: 'drop' as const, warehouseId: MINE_DEPOT, key: 'k1' };
    const once = applyMove(objective, m, WAREHOUSES);
    expect(once?.state.delivered).toBe(5);
    expect(applyMove({ ...objective, state: once!.state }, m, WAREHOUSES)).toBeNull();
  });

  it('a crate counts once: it fills the oldest mission, the rest goes to the next', async () => {
    const first = await createMission({
      type: 'deliver_material',
      targetQuantity: 15,
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    const second = await createMission({
      type: 'deliver_material',
      targetQuantity: 15,
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    await move('drop', 'ore.gold', MINE_DEPOT, 20);
    expect((await objectiveOf(first)).currentProgress).toBe(15);
    expect((await objectiveOf(second)).currentProgress).toBe(5);
  });
});

describe('turn-in', () => {
  it('a finished mission that does not auto-complete waits, pays nothing, and refuses the player', async () => {
    const id = await createMission({
      type: 'deliver_material',
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    await move('drop', 'ore.gold', MINE_DEPOT);
    const [assignment] = await db
      .select()
      .from(missionAssignments)
      .where(and(eq(missionAssignments.missionId, id), eq(missionAssignments.playerId, PLAYER)));
    expect(assignment.status).toBe('active');
    expect(economy.creditHolder).not.toHaveBeenCalled();
    expect(social.notifyPlayer).toHaveBeenCalledWith(PLAYER, expect.objectContaining({ type: 'mission.ready' }));

    const refused = await playerCall(`/${id}/complete`);
    expect(refused.status).toBe(403);
    expect(refused.body.error ?? refused.body.code).toBeDefined();

    const turnedIn = await call(`/missions/${id}/complete`, 'POST', { playerId: PLAYER });
    expect(turnedIn.status).toBe(200);
    expect(economy.creditHolder).toHaveBeenCalled();
  });
});
