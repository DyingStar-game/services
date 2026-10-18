/**
 * Objective kinds, measured directly against an in-process database (PGlite) with a
 * mocked Inventory client. Two things are covered here:
 *   - the target applies the **POI's own radius** (there is no radius anywhere else);
 *   - `deliver_material` with `scope: 'all'` pools every active assignee inside the
 *     target, and only them.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
vi.mock('../src/services/inventory.client.js', () => ({
  resolvePois: vi.fn(async () => new Map()),
  getStack: vi.fn(async () => null),
  isInventoryConfigured: () => true,
}));

const { db } = await import('../src/db/connection.js');
const { missionAssignments, missionObjectives, missions, missionPlayerStates } = await import(
  '../src/db/schema/index.js'
);
const inventory = await import('../src/services/inventory.client.js');
const { deliverMaterialKind } = await import('../src/kinds/objectives/deliverMaterial.js');
const { visitKind } = await import('../src/kinds/objectives/visit.js');

const POI = '3f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d';
const MISSION = '11111111-1111-4111-8111-111111111111';
const OBJECTIVE = '22222222-2222-4222-8222-222222222222';

const A = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const B = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const C = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';

/** POI on `tarsis_3`, 10 m radius, centre at the origin of the body. */
const poiDisc = { id: POI, system: 'tarsis', scene: 'tarsis_3', x: 0, y: 0, z: 0, radiusM: 10 };

/** Mocks the Inventory resolver for one POI. */
function mockPoi(poi: Record<string, unknown>) {
  vi.mocked(inventory.resolvePois).mockImplementation(async (ids: string[]) =>
    new Map(ids.filter((id) => id === poi.id).map((id) => [id, poi])),
  );
}

/** Builds the context handed to `measure`. */
function ctx(objective: Record<string, unknown>, playerId = A) {
  return {
    playerId,
    holderType: 'player' as const,
    missionId: MISSION,
    objective,
  } as any;
}

/** A minimal objective row for `deliver_material`. */
function objective(params: Record<string, unknown>, targetQuantity = 100) {
  return { id: OBJECTIVE, type: 'deliver_material', params, targetQuantity };
}

/** A minimal objective row for `visit`. */
function visitObjective(params: Record<string, unknown>) {
  return { id: OBJECTIVE, type: 'visit', params, targetQuantity: 1 };
}

/** Pushes a player state (`place` + `carried`), fresh unless `ageMs` says otherwise. */
async function place(playerId: string, x: number, carried: Record<string, number> = {}, ageMs = 0) {
  await db
    .insert(missionPlayerStates)
    .values({
      playerId,
      place: { system: 'tarsis', scene: 'tarsis_3', x, y: 0, z: 0 },
      carried,
      receivedAt: new Date(Date.now() - ageMs),
    })
    .onConflictDoUpdate({
      target: missionPlayerStates.playerId,
      set: {
        place: { system: 'tarsis', scene: 'tarsis_3', x, y: 0, z: 0 },
        carried,
        receivedAt: new Date(Date.now() - ageMs),
      },
    });
}

/** Creates a mission + one objective + N active assignees. */
async function setupMission(assigneeIds: string[], objectiveParams: Record<string, unknown>) {
  await db.delete(missionAssignments);
  await db.delete(missionObjectives);
  await db.delete(missions);
  await db.insert(missions).values({ id: MISSION, title: 'Contrat' });
  await db.insert(missionObjectives).values({
    id: OBJECTIVE,
    missionId: MISSION,
    type: 'deliver_material',
    title: 'Livraison',
    params: objectiveParams,
    targetQuantity: 100,
  });
  for (const id of assigneeIds) {
    await db
      .insert(missionAssignments)
      .values({ missionId: MISSION, playerId: id, status: 'active', rewardExternalId: `test-${id}` });
  }
}

beforeEach(async () => {
  await db.delete(missionPlayerStates);
  vi.mocked(inventory.resolvePois).mockReset();
  vi.mocked(inventory.getStack).mockReset();
});

// ── §4.3: the target applies the POI's radius ─────────────────────────────────

describe('objective target reads the POI radius', () => {
  it('a visit 8 m from a 10 m POI is measured as done', async () => {
    mockPoi(poiDisc);
    await place(A, 8);
    const params = { zone: { kind: 'poi', poiId: POI } };
    expect(await visitKind.measure!(ctx(visitObjective(params)))).toBe(1);
  });

  it('a visit 15 m from a 10 m POI is not reached', async () => {
    mockPoi(poiDisc);
    await place(A, 15);
    const params = { zone: { kind: 'poi', poiId: POI } };
    expect(await visitKind.measure!(ctx(visitObjective(params)))).toBe(0);
  });

  it('a point POI (no radius) still covers its whole scene, so 5 m matches', async () => {
    // A POI without a radius falls back to the scene subtree, exactly like a zone does.
    mockPoi({ ...poiDisc, radiusM: null });
    await place(A, 5);
    const params = { zone: { kind: 'poi', poiId: POI } };
    expect(await visitKind.measure!(ctx(visitObjective(params)))).toBe(1);
  });
});

// ── §4.4: scope:'all' pools the carriers inside the target ────────────────────

describe("deliver_material scope: 'all'", () => {
  it('sums only the carriers who are inside the target', async () => {
    mockPoi(poiDisc);
    const params = { goodType: 'metal', zone: { kind: 'poi', poiId: POI }, scope: 'all' };
    await setupMission([A, B, C], params);
    await place(A, 0, { metal: 5 });
    await place(B, 1, { metal: 5 });
    await place(C, 1000, { metal: 90 }); // outside the 10 m disc: ignored

    expect(await deliverMaterialKind.measure!(ctx(objective(params)))).toBe(10);
  });

  it('ignores a carrier whose state is stale', async () => {
    mockPoi(poiDisc);
    const params = { goodType: 'metal', zone: { kind: 'poi', poiId: POI }, scope: 'all' };
    await setupMission([A, B], params);
    await place(A, 0, { metal: 4 }, 60_000); // stale: outside every zone
    await place(B, 0, { metal: 6 });

    expect(await deliverMaterialKind.measure!(ctx(objective(params)))).toBe(6);
  });

  it('ignores a carrier with no pushed state at all (an NPC co-assignee)', async () => {
    mockPoi(poiDisc);
    const params = { goodType: 'metal', zone: { kind: 'poi', poiId: POI }, scope: 'all' };
    await setupMission([A, B], params);
    await place(A, 0, { metal: 7 });
    // B never got a state row: no place, no carried.

    expect(await deliverMaterialKind.measure!(ctx(objective(params)))).toBe(7);
  });

  it('is capped by the objective target', async () => {
    mockPoi(poiDisc);
    const params = { goodType: 'metal', zone: { kind: 'poi', poiId: POI }, scope: 'all' };
    await setupMission([A], params);
    await place(A, 0, { metal: 500 });

    expect(await deliverMaterialKind.measure!(ctx(objective(params, 10)))).toBe(10);
  });

  it("scope 'assignee' (the default) counts only the measured carrier", async () => {
    mockPoi(poiDisc);
    const params = { goodType: 'metal', zone: { kind: 'poi', poiId: POI } };
    await setupMission([A, B], params);
    await place(A, 0, { metal: 3 });
    await place(B, 0, { metal: 9 });
    vi.mocked(inventory.getStack).mockResolvedValue({ carried: 3 } as any);

    expect(await deliverMaterialKind.measure!(ctx(objective(params)))).toBe(3);
  });

  it('rejects an unknown scope value', () => {
    expect(() =>
      deliverMaterialKind.paramsSchema.parse({
        goodType: 'metal',
        zone: { kind: 'poi', poiId: POI },
        scope: 'everyone',
      }),
    ).toThrow();
  });
});
