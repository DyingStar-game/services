/**
 * The auto-measure driven by the player-state push: what makes a `visit` or a delivery
 * progress **without anyone calling `/verify`**, and what keeps it bounded when the push
 * carries a whole server's worth of players.
 *
 * Economy and Social are mocked (completion settles and notifies); Inventory is mocked
 * with spies so the round trips a pass costs are observable. The database is an
 * in-process PGlite migrated from `./drizzle`.
 *
 * Two ways in: the HTTP push (proves the wiring, and pushes a place like the game does),
 * and a direct call to `measureActiveAssignmentsForPlayers` (used where a push would
 * overwrite the state under test, such as a state that stopped being pushed).
 */
import type { Server } from 'http';
import { eq } from 'drizzle-orm';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

const economy = vi.hoisted(() => ({
  debitHolder: vi.fn(),
  creditHolder: vi.fn(),
}));

const inventory = vi.hoisted(() => ({
  getStack: vi.fn(),
  transferStack: vi.fn(),
  resolvePois: vi.fn(),
  isInventoryConfigured: () => true,
}));

const social = vi.hoisted(() => ({
  addStanding: vi.fn(),
  getStanding: vi.fn(),
  notifyPlayer: vi.fn(),
}));

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
vi.mock('../src/services/economy.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/economy.client.js')>()),
  debitHolder: economy.debitHolder,
  creditHolder: economy.creditHolder,
}));
vi.mock('../src/services/inventory.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/inventory.client.js')>()),
  getStack: inventory.getStack,
  transferStack: inventory.transferStack,
  resolvePois: inventory.resolvePois,
  isInventoryConfigured: inventory.isInventoryConfigured,
}));
vi.mock('../src/services/social.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/social.client.js')>()),
  addStanding: social.addStanding,
  getStanding: social.getStanding,
  notifyPlayer: social.notifyPlayer,
}));

const { db } = await import('../src/db/connection.js');
const { missionAssignments, missionObjectives, missions, missionPlayerStates } = await import(
  '../src/db/schema/index.js'
);
const { env } = await import('../src/config/env.js');
const { measureActiveAssignmentsForPlayers, verifyPlayerObjectives } = await import(
  '../src/services/assignments.service.js'
);
const { markMissionActive } = await import('../src/services/missions.service.js');

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const MISSION = '11111111-1111-4111-8111-111111111111';
const OBJECTIVE = '22222222-2222-4222-8222-222222222222';
const POI = '3f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d';

/** POI on `tarsis_3`, 10 m radius, centred on the origin of the body. */
const poiDisc = { id: POI, system: 'tarsis', scene: 'tarsis_3', x: 0, y: 0, z: 0, radiusM: 10 };

/** Budget restored between tests (tests that change it say so). */
const DEFAULT_REMOTE_BUDGET = 100;

let server: Server;
let baseUrl: string;

/** Internal API call as the game server. */
async function call(path: string, method = 'GET', body?: unknown) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-roles': 'mission:write,mission:progress' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

/** Mocks the Inventory geometry resolver for one POI. */
function mockPoi(poi: Record<string, unknown>) {
  inventory.resolvePois.mockImplementation(async (ids: string[]) =>
    new Map(ids.filter((id) => id === poi.id).map((id) => [id, poi])),
  );
}

/** Adds one objective to the mission built by {@link setupMission}. */
async function addObjective(
  type: string,
  params: Record<string, unknown>,
  opts: { targetQuantity?: number; status?: string; currentProgress?: number } = {},
) {
  await db.insert(missionObjectives).values({
    id: OBJECTIVE,
    missionId: MISSION,
    type,
    title: 'Objectif',
    params,
    targetQuantity: opts.targetQuantity ?? 1,
    status: opts.status as never,
    currentProgress: opts.currentProgress ?? 0,
  });
}

/** Creates a mission and one active assignment per player, as `accept` would. */
async function setupMission(playerIds: string[]) {
  await db.delete(missionAssignments);
  await db.delete(missionObjectives);
  await db.delete(missions);
  await db.insert(missions).values({ id: MISSION, title: 'Contrat' });
  if (playerIds.length > 0) {
    await db.insert(missionAssignments).values(
      playerIds.map((id) => ({
        missionId: MISSION,
        playerId: id,
        status: 'active' as const,
        rewardExternalId: `auto-${id}`,
      })),
    );
    await markMissionActive(MISSION);
  }
}

/** Writes a player state directly (bypasses the push, so a stale state stays stale). */
async function place(playerId: string, x: number, ageMs = 0) {
  await placeMany([playerId], x, ageMs);
}

/** Writes many player states in one statement (scale tests). */
async function placeMany(playerIds: string[], x: number, ageMs = 0) {
  if (playerIds.length === 0) return;
  const place = { system: 'tarsis', scene: 'tarsis_3', x, y: 0, z: 0 };
  const receivedAt = new Date(Date.now() - ageMs);
  await db
    .insert(missionPlayerStates)
    .values(playerIds.map((playerId) => ({ playerId, place, receivedAt })))
    .onConflictDoUpdate({
      target: missionPlayerStates.playerId,
      set: { place, receivedAt },
    });
}

/** Pushes a player state through the real endpoint, carrying its place. */
async function push(playerId: string, x: number) {
  await pushBatch([playerId], x);
}

/** Pushes a batch of player states through the real endpoint (500 is the schema limit). */
async function pushBatch(playerIds: string[], x = 8) {
  const { status } = await call('/missions/player-states', 'PUT', {
    players: playerIds.map((playerId) => ({
      playerId,
      place: { system: 'tarsis', scene: 'tarsis_3', x, y: 0, z: 0 },
    })),
  });
  expect(status).toBe(200);
}

/** Waits for the fire-and-forget pass to settle on a predicate. */
async function settled(predicate: () => Promise<boolean>) {
  await vi.waitFor(predicate, { timeout: 3_000, interval: 20 });
}

async function objectiveRow() {
  const [row] = await db.select().from(missionObjectives).where(eq(missionObjectives.id, OBJECTIVE));
  return row;
}

async function missionRow() {
  const [row] = await db.select().from(missions).where(eq(missions.id, MISSION));
  return row;
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

beforeEach(async () => {
  await db.delete(missionPlayerStates);
  await db.delete(missionAssignments);
  await db.delete(missionObjectives);
  await db.delete(missions);
  inventory.getStack.mockReset().mockResolvedValue(null);
  inventory.resolvePois.mockReset().mockResolvedValue(new Map());
  inventory.transferStack.mockReset().mockResolvedValue(undefined);
  economy.debitHolder.mockReset().mockResolvedValue({ transaction: { id: 1 } });
  economy.creditHolder.mockReset().mockResolvedValue({ transaction: { id: 2 } });
  social.addStanding.mockReset().mockResolvedValue({});
  social.getStanding.mockReset().mockResolvedValue({});
  social.notifyPlayer.mockReset().mockResolvedValue(undefined);
  env.mission.autoMeasure = true;
  env.mission.autoMeasureRemoteBudget = DEFAULT_REMOTE_BUDGET;
});

// ── The push drives the measurement ───────────────────────────────────────────

describe('auto-measure on the player-state push', () => {
  it('progresses a visit and completes the mission without anyone calling /verify', async () => {
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });

    await push(PLAYER, 8); // inside the 10 m disc

    await settled(async () => (await missionRow()).status === 'completed');
    const objective = await objectiveRow();
    expect(objective.status).toBe('completed');
    expect(objective.currentProgress).toBe(1);

    const [assignment] = await db
      .select()
      .from(missionAssignments)
      .where(eq(missionAssignments.missionId, MISSION));
    expect(assignment.status).toBe('completed');
    expect(assignment.completedAt).not.toBeNull();
  });

  it('leaves a player outside the target untouched, while the push still succeeds', async () => {
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });

    await push(PLAYER, 5_000); // far outside the disc

    await settled(async () => (await objectiveRow()).currentProgress === 0);
    // Give a wrong result the time to show up if it were coming.
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect((await objectiveRow()).currentProgress).toBe(0);
    expect((await objectiveRow()).status).toBe('pending');
    expect((await missionRow()).status).toBe('active');
  });

  it('treats a state that stopped being pushed as outside every zone', async () => {
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });
    // Inside the disc, but the game last pushed a minute ago: no push, so the state
    // stays stale and the pass must ignore it.
    await place(PLAYER, 8, 60_000);

    await measureActiveAssignmentsForPlayers([PLAYER]);
    expect((await objectiveRow()).currentProgress).toBe(0);
  });

  it('does nothing at all when none of the pushed players holds an active assignment', async () => {
    await setupMission([]); // nobody is assigned
    await place(PLAYER, 8);

    await push(PLAYER, 8);
    await new Promise((resolve) => setTimeout(resolve, 150));

    // One indexed SELECT, then the pass stops: nothing resolved, nothing read.
    expect(inventory.resolvePois).not.toHaveBeenCalled();
    expect(inventory.getStack).not.toHaveBeenCalled();
  });

  it('leaves a mission made only of issuer objectives to its confirm endpoint', async () => {
    await setupMission([PLAYER]);
    await addObjective('manual', {});
    await place(PLAYER, 8);

    await push(PLAYER, 8);
    await new Promise((resolve) => setTimeout(resolve, 150));

    expect((await objectiveRow()).status).toBe('pending');
    expect((await missionRow()).status).toBe('active');
    expect(inventory.getStack).not.toHaveBeenCalled();
    expect(inventory.resolvePois).not.toHaveBeenCalled();
  });

  it('sets the transport latch when the assignee is seen at the source with goods', async () => {
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('transport', {
      goodType: 'fer',
      source: { kind: 'poi', poiId: POI },
      zone: { kind: 'poi', poiId: POI },
    });
    inventory.getStack.mockResolvedValue({ available: 5, quantity: 5, carried: 5 });

    await push(PLAYER, 8);

    await settled(async () => Boolean((await objectiveRow()).state?.pickedUpAt));
  });
});

// ── Historical behaviour, still available ─────────────────────────────────────

describe('MISSION_AUTO_MEASURE', () => {
  it('reverts to the historical contract when disabled: the push stores, nothing measures', async () => {
    env.mission.autoMeasure = false;
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });

    await push(PLAYER, 8);

    await new Promise((resolve) => setTimeout(resolve, 150));
    expect((await objectiveRow()).currentProgress).toBe(0);
    expect(inventory.resolvePois).not.toHaveBeenCalled();
  });

  it('keeps /verify working as an explicit re-measure', async () => {
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });
    await place(PLAYER, 8);

    const result = await verifyPlayerObjectives(MISSION, PLAYER);
    expect(result.objectives[0].currentProgress).toBe(1);
    expect(result.completed).toBe(true);
    expect((await missionRow()).status).toBe('completed');
  });
});

// ── Cost control ──────────────────────────────────────────────────────────────

describe('measurement cost', () => {
  it('never reads a completed high-water objective again', async () => {
    mockPoi(poiDisc);
    await setupMission([PLAYER]);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } }, {
      status: 'completed',
      currentProgress: 1,
    });
    await place(PLAYER, 8);

    await measureActiveAssignmentsForPlayers([PLAYER]);
    expect(inventory.resolvePois).not.toHaveBeenCalled();

    // …and the same skip applies to the explicit endpoint.
    await verifyPlayerObjectives(MISSION, PLAYER);
    expect(inventory.resolvePois).not.toHaveBeenCalled();
  });

  it('budgets the remote (Inventory) reads of a pass and serves another tranche next time', async () => {
    const players = [
      'a0000000-0000-4000-8000-000000000000',
      'b0000000-0000-4000-8000-000000000001',
      'c0000000-0000-4000-8000-000000000002',
      'd0000000-0000-4000-8000-000000000003',
      'e0000000-0000-4000-8000-000000000004',
    ];
    env.mission.autoMeasureRemoteBudget = 2;
    await setupMission(players);
    await addObjective('owns_items', { itemId: 'minerai' }, { targetQuantity: 10 });
    for (const playerId of players) await place(playerId, 8);

    await measureActiveAssignmentsForPlayers(players);
    expect(inventory.getStack).toHaveBeenCalledTimes(2);
    const firstServed = inventory.getStack.mock.calls.map((call) => call[0].holderId as string);
    expect(firstServed).toEqual([players[0], players[1]]);

    // The order rotates, so the pass never starves the tail of a large batch.
    inventory.getStack.mockClear();
    await measureActiveAssignmentsForPlayers(players);
    expect(inventory.getStack).toHaveBeenCalledTimes(2);
    const nextServed = inventory.getStack.mock.calls.map((call) => call[0].holderId as string);
    expect(nextServed).not.toEqual(firstServed);
    expect(nextServed[0]).toBe(players[2]);
  });

  it('measures a whole batch of state-based objectives with no Inventory round trip', async () => {
    mockPoi(poiDisc);
    const players = Array.from(
      { length: 60 },
      (_, i) => `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
    );
    await setupMission(players);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });
    for (const playerId of players) await place(playerId, 8);

    const started = Date.now();
    await measureActiveAssignmentsForPlayers(players);
    const elapsed = Date.now() - started;

    // visit reads preloaded states only: the whole batch costs no HTTP call at all,
    // and the mission closes on the first fully measured assignee.
    expect(inventory.getStack).not.toHaveBeenCalled();
    expect((await missionRow()).status).toBe('completed');
    expect(elapsed).toBeLessThan(2_000);
  });
});

// ── A server's worth of players ───────────────────────────────────────────────

describe('scale: a thousand players', () => {
  const THOUSAND = Array.from(
    { length: 1_000 },
    (_, i) => `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
  );

  it('costs nothing measurable when none of them holds an assignment', async () => {
    await setupMission([]);
    await placeMany(THOUSAND, 8);

    // The game pushes in batches of 500 at most.
    for (let offset = 0; offset < THOUSAND.length; offset += 500) {
      const started = Date.now();
      await pushBatch(THOUSAND.slice(offset, offset + 500));
      expect(Date.now() - started).toBeLessThan(2_000);
    }
    await new Promise((resolve) => setTimeout(resolve, 200));

    // One indexed SELECT per pass, then it stops: no geometry resolved, no Inventory read.
    expect(inventory.resolvePois).not.toHaveBeenCalled();
    expect(inventory.getStack).not.toHaveBeenCalled();
  });

  it('measures the whole batch of state-based objectives without one Inventory call', async () => {
    mockPoi(poiDisc);
    await setupMission(THOUSAND);
    await addObjective('visit', { zone: { kind: 'poi', poiId: POI } });
    // Outside the target: every assignment stays measurable, so this is the worst case —
    // a pass that has to measure all thousand of them rather than stop on the first.
    await placeMany(THOUSAND, 5_000);

    const started = Date.now();
    await measureActiveAssignmentsForPlayers(THOUSAND);
    const elapsed = Date.now() - started;

    // The state-based kinds are served from the batch's one SELECT: no HTTP at all, and
    // a thousand measurements stay well inside a single push interval.
    expect(inventory.getStack).not.toHaveBeenCalled();
    expect(inventory.resolvePois).toHaveBeenCalledTimes(1_000); // one per objective, all mocked
    expect(elapsed).toBeLessThan(3_000);
    expect((await objectiveRow()).currentProgress).toBe(0);
    expect((await missionRow()).status).toBe('active');
  });

  it('caps the Inventory round trips of a pass at the remote budget', async () => {
    env.mission.autoMeasureRemoteBudget = 100;
    await setupMission(THOUSAND);
    await addObjective('owns_items', { itemId: 'minerai' }, { targetQuantity: 10 });
    await placeMany(THOUSAND, 8);

    const started = Date.now();
    await measureActiveAssignmentsForPlayers(THOUSAND);
    const elapsed = Date.now() - started;

    // The whole answer to "does it slow the server down": a thousand players cost a
    // hundred remote reads, and the next pass picks up where this one stopped.
    expect(inventory.getStack).toHaveBeenCalledTimes(100);
    expect(elapsed).toBeLessThan(3_000);
  });
});
