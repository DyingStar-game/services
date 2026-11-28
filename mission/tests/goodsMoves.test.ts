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
const { applyMove, mainStep, pickUpDone, usedBefore } = await import('../src/services/goodsMoves.service.js');

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

  describe('what a transport loaded', () => {
    const transport = (state: Record<string, unknown>) => ({
      type: 'transport' as const,
      params: {
        goodType: 'ore.gold',
        deliverGoodType: 'ingot.gold',
        source: { kind: 'poi', poiId: MINE_POI },
        zone: { kind: 'poi', poiId: FACTORY_POI },
        via: 'warehouse',
      },
      state,
      targetQuantity: 240,
    });
    const load = (key: string, quantity: number) => ({
      playerId: PLAYER,
      goodType: 'ore.gold',
      quantity,
      direction: 'load' as const,
      warehouseId: MINE_DEPOT,
      key,
    });
    const drop = (key: string, quantity: number) => ({
      playerId: PLAYER,
      goodType: 'ingot.gold',
      quantity,
      direction: 'drop' as const,
      warehouseId: FACTORY_DEPOT,
      key,
    });

    it('adds up crate after crate, and sells nothing', () => {
      const first = applyMove(transport({}), load('l1', 15), WAREHOUSES);
      expect(first?.state.loaded).toBe(15); // one crate is not the pick-up
      expect(first?.state.loadedAt).toBeDefined();
      expect(first?.used).toBe(0);
      const second = applyMove(transport(first!.state), load('l2', 30), WAREHOUSES);
      expect(second?.state.loaded).toBe(45);
      expect(second?.state.loadedAt).toBe(first?.state.loadedAt); // latched by the first load
      expect(second?.used).toBe(0);
      expect(second?.state.moveKeys).toEqual(['l1', 'l2']);
      expect(second?.state.moveUsed).toEqual({ l1: 0, l2: 0 });
    });

    it('stops at the target, and forgets the loads past it', () => {
      const nearly = applyMove(transport({}), load('l1', 230), WAREHOUSES);
      const full = applyMove(transport(nearly!.state), load('l2', 15), WAREHOUSES);
      expect(full?.state.loaded).toBe(240);
      expect(applyMove(transport(full!.state), load('l3', 15), WAREHOUSES)).toBeNull();
      expect(full?.state.moveKeys).toEqual(['l1', 'l2']);
    });

    it('counts a replayed load once', () => {
      const once = applyMove(transport({}), load('l1', 15), WAREHOUSES);
      expect(applyMove(transport(once!.state), load('l1', 15), WAREHOUSES)).toBeNull();
      expect(usedBefore({ state: once!.state }, load('l1', 15))).toBe(0); // the replay counts nothing as delivered
    });

    it('lets a partial load be delivered', () => {
      const loaded = applyMove(transport({}), load('l1', 15), WAREHOUSES);
      const dropped = applyMove(transport(loaded!.state), drop('d1', 15), WAREHOUSES);
      expect(dropped?.used).toBe(15);
      expect(dropped?.state.delivered).toBe(15);
      expect(dropped?.state.loaded).toBe(15);
    });

    it('goes on from a state loaded before the total was kept', () => {
      const old = { loadedAt: '2026-11-27T10:00:00.000Z', delivered: 30, moveKeys: ['l0', 'd0'], moveUsed: { l0: 0, d0: 30 } };
      const dropped = applyMove(transport(old), drop('d1', 15), WAREHOUSES);
      expect(dropped?.state.delivered).toBe(45);
      expect(dropped?.state.loaded).toBeUndefined();
      const loaded = applyMove(transport(old), load('l1', 15), WAREHOUSES);
      expect(loaded?.state.loaded).toBe(15); // counted from this load on
      expect(loaded?.state.loadedAt).toBe(old.loadedAt);
      expect(loaded?.state.delivered).toBe(30);
    });

    it('is read by the client in the objective state', async () => {
      const id = await createMission({
        type: 'transport',
        params: { goodType: 'ore.gold', source: { kind: 'poi', poiId: MINE_POI }, zone: { kind: 'poi', poiId: FACTORY_POI }, via: 'warehouse' },
      });
      const body = { playerId: PLAYER, goodType: 'ore.gold', quantity: 5, direction: 'load', warehouseId: MINE_DEPOT, key: 'load-replayed' };
      expect((await call('/missions/goods-moves', 'POST', body)).body.counted).toBe(0);
      expect((await call('/missions/goods-moves', 'POST', body)).body.counted).toBe(0);
      expect((await objectiveOf(id)).state.loaded).toBe(5); // the replay added nothing
      await move('load', 'ore.gold', MINE_DEPOT, 10);
      expect((await objectiveOf(id)).state.loaded).toBe(15);
      await move('drop', 'ore.gold', FACTORY_DEPOT, 15);
      expect((await objectiveOf(id)).currentProgress).toBe(15);
      await move('load', 'ore.gold', MINE_DEPOT, 20);
      expect((await objectiveOf(id)).state.loaded).toBe(20); // the target of this mission
    });
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

  it('a move answers what the missions counted, and a replay answers the same', async () => {
    const mission = await createMission({
      type: 'deliver_material',
      targetQuantity: 15,
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    const body = { playerId: PLAYER, goodType: 'ore.gold', quantity: 20, direction: 'drop', warehouseId: MINE_DEPOT, key: 'replayed' };
    const first = await call('/missions/goods-moves', 'POST', body);
    expect(first.body.counted).toBe(15); // 5 left: the player's own
    const again = await call('/missions/goods-moves', 'POST', body);
    expect(again.body.counted).toBe(15); // not 0: nothing of the mission's is sold on a retry
    expect(again.body.applied).toEqual([]);
    expect((await objectiveOf(mission)).currentProgress).toBe(15);
    expect((await move('drop', 'ore.gold', MINE_DEPOT, 20)).body.counted).toBe(0); // the mission is full
  });

  it('a move remembered without its count answers the whole of it', () => {
    const m = { key: 'old', quantity: 7 };
    expect(usedBefore({ state: { moveKeys: ['old'] } }, m)).toBe(7);
    expect(usedBefore({ state: { moveKeys: ['old'], moveUsed: { old: 3 } } }, m)).toBe(3);
    expect(usedBefore({ state: {} }, m)).toBeNull();
  });

  it('with inventory down nothing is counted and the answer says so', async () => {
    const mission = await createMission({
      type: 'deliver_material',
      targetQuantity: 15,
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    inventory.resolvePois.mockRejectedValueOnce(new Error('down'));
    const body = { playerId: PLAYER, goodType: 'ore.gold', quantity: 20, direction: 'drop', warehouseId: MINE_DEPOT, key: 'while-down' };
    expect((await call('/missions/goods-moves', 'POST', body)).body.counted).toBeNull();
    expect((await call('/missions/goods-moves', 'POST', body)).body.counted).toBe(15); // the retry counts
    expect((await objectiveOf(mission)).currentProgress).toBe(15);
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

/** The notification types the player was sent, in order. */
function notified(): string[] {
  return social.notifyPlayer.mock.calls.map((args) => (args[1] as { type: string }).type);
}

describe('turn-in', () => {
  it('a move that completes no step refreshes its assignees silently', async () => {
    await createMission({
      type: 'deliver_material',
      params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' },
    });
    await move('drop', 'ore.gold', MINE_DEPOT, 5);
    expect(notified()).toEqual(['mission.updated']);
  });

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

describe('notifications at steps', () => {
  const transportParams = {
    goodType: 'ore.gold',
    source: { kind: 'poi', poiId: MINE_POI },
    zone: { kind: 'poi', poiId: FACTORY_POI },
    via: 'warehouse',
  };

  it('a transport tells of its pick-up once everything is loaded, then of nothing until it is done', async () => {
    await createMission({ type: 'transport', params: transportParams });
    await move('load', 'ore.gold', MINE_DEPOT, 5);
    await move('load', 'ore.gold', MINE_DEPOT, 10);
    expect(notified()).toEqual(['mission.updated', 'mission.updated']);
    await move('load', 'ore.gold', MINE_DEPOT, 5);
    expect(social.notifyPlayer).toHaveBeenLastCalledWith(
      PLAYER,
      expect.objectContaining({ type: 'mission.progress', data: expect.objectContaining({ step: 'loaded' }) }),
    );
    social.notifyPlayer.mockClear();
    await move('drop', 'ore.gold', FACTORY_DEPOT, 10);
    expect(notified()).toEqual(['mission.updated']);
    await move('drop', 'ore.gold', FACTORY_DEPOT, 10);
    expect(notified()).toEqual(['mission.updated', 'mission.ready']); // the last step: ready, not progress
  });

  it('a delivery completing one objective of several tells of that step', async () => {
    const { body } = await call('/missions', 'POST', {
      title: 'Two runs',
      category: 'transport',
      autoComplete: false,
      objectives: [
        { title: 'Gold', type: 'deliver_material', targetQuantity: 20, params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' } },
        { title: 'Iron', type: 'deliver_material', targetQuantity: 20, params: { goodType: 'ore.iron', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' } },
      ],
      rewards: [{ type: 'credits', amount: 500 }],
    });
    await call(`/missions/${body.mission.id}/assign`, 'POST', { playerId: PLAYER, holderType: 'npc' });
    await move('drop', 'ore.gold', MINE_DEPOT, 20);
    expect(social.notifyPlayer).toHaveBeenCalledTimes(1);
    expect(social.notifyPlayer).toHaveBeenCalledWith(
      PLAYER,
      expect.objectContaining({ type: 'mission.progress', data: expect.objectContaining({ step: 'delivered' }) }),
    );
  });

  it('an auto-completed mission tells of its completion only', async () => {
    await createMission(
      { type: 'deliver_material', params: { goodType: 'ore.gold', zone: { kind: 'poi', poiId: MINE_POI }, via: 'warehouse' } },
      true,
    );
    await move('drop', 'ore.gold', MINE_DEPOT, 20);
    expect(notified()).toEqual(['mission.completed']);
  });

  it('a pick-up is done when everything is loaded, or at the first load counted before the total was kept', () => {
    expect(pickUpDone({ state: {}, targetQuantity: 20 })).toBe(false);
    expect(pickUpDone({ state: { loadedAt: 'x', loaded: 15 }, targetQuantity: 20 })).toBe(false);
    expect(pickUpDone({ state: { loadedAt: 'x', loaded: 20 }, targetQuantity: 20 })).toBe(true);
    expect(pickUpDone({ state: { loadedAt: 'x' }, targetQuantity: 20 })).toBe(true);
  });

  it('a delivery says more than a pick-up', () => {
    expect(mainStep(new Set(['loaded', 'delivered']))).toBe('delivered');
    expect(mainStep(new Set(['loaded']))).toBe('loaded');
    expect(mainStep(new Set())).toBeNull();
    expect(mainStep(undefined)).toBeNull();
  });
});
