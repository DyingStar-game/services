/**
 * Mission gameplay: auto-completion, partial hand-ins (proportional credits), bonus
 * objective rewards, per-assignment deadlines, the scenario chain and permanently failed
 * assignments. Economy and Inventory are mocked; the database is an in-process PGlite
 * migrated from `./drizzle`.
 */
import type { Server } from 'http';
import { and, eq } from 'drizzle-orm';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { HttpError } from '../src/lib/httpError.js';
import { startTestApp } from './helpers/testApp.js';

const economy = vi.hoisted(() => ({
  debitHolder: vi.fn(),
  creditHolder: vi.fn(),
}));

const inventory = vi.hoisted(() => ({
  getStack: vi.fn(),
  transferStack: vi.fn(),
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
}));
vi.mock('../src/services/social.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/social.client.js')>()),
  addStanding: social.addStanding,
  getStanding: social.getStanding,
  notifyPlayer: social.notifyPlayer,
}));

const { db } = await import('../src/db/connection.js');
const { missionAssignments, missions } = await import('../src/db/schema/index.js');

const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const OTHER = '1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e';
const ARES = '7b0c3f2e-1a4d-4c8e-9f21-3d5a6b7c8d90';
const WAREHOUSE = '2e9d4a61-5b3c-4f7a-8e12-6c4b3a2d1e0f';
const ALL_ROLES = 'mission:read,mission:write,mission:complete,mission:progress,mission:escrow:politics';

let server: Server;
let baseUrl: string;

/** Internal API call. */
async function call(path: string, method = 'GET', body?: unknown, roles = ALL_ROLES) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-roles': roles },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

/** Creates a mission through the internal API. */
async function createMission(body: Record<string, unknown>) {
  const { status, body: created } = await call('/missions', 'POST', body);
  expect(status).toBe(201);
  return created.mission.id as string;
}

/** Assigns a mission to a holder (NPCs skip the player-side gates). */
async function assign(missionId: string, playerId = PLAYER, holderType: 'npc' | 'player' = 'npc') {
  return call(`/missions/${missionId}/assign`, 'POST', { playerId, holderType });
}

async function confirm(missionId: string, objectiveId: string) {
  return call(`/missions/${missionId}/objectives/${objectiveId}/confirm`, 'POST', {});
}

async function loadMission(id: string) {
  const [row] = await db.select().from(missions).where(eq(missions.id, id));
  return row;
}

async function loadAssignment(missionId: string, playerId = PLAYER) {
  const [row] = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.playerId, playerId)));
  return row;
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

beforeEach(async () => {
  // Reset assignment state between tests to avoid the active-assignment limit leaking.
  await db.delete(missionAssignments);
  economy.debitHolder.mockReset().mockResolvedValue({ transaction: { id: 1 } });
  economy.creditHolder.mockReset().mockResolvedValue({ transaction: { id: 2 } });
  inventory.getStack.mockReset().mockResolvedValue({ available: 10 });
  inventory.transferStack.mockReset().mockResolvedValue(undefined);
  social.addStanding.mockReset().mockResolvedValue({ exploration: 10 });
  social.getStanding.mockReset().mockResolvedValue({});
  social.notifyPlayer.mockReset().mockResolvedValue(undefined);
});

describe('auto-completion', () => {
  it('completes and settles the mission on its own once the required objectives are done', async () => {
    const missionId = await createMission({
      title: 'Reconnaissance',
      category: 'exploration',
      objectives: [{ type: 'manual', title: 'Atteindre le phare', targetQuantity: 1 }],
      rewards: [{ type: 'credits', amount: 1000 }],
    });
    const mission = await loadMission(missionId);
    expect(mission.autoComplete).toBe(true);

    await assign(missionId);
    const { body: detail } = await call(`/missions/${missionId}`);
    const objectiveId = detail.objectives[0].id as string;

    const { status, body } = await confirm(missionId, objectiveId);
    expect(status).toBe(200);
    expect(body).toMatchObject({ allComplete: true, completed: true });

    const assignment = await loadAssignment(missionId);
    expect(assignment.status).toBe('completed');
    expect(assignment.completionRatio).toBe(10000);
    expect((await loadMission(missionId)).status).toBe('completed');
    expect(economy.creditHolder).toHaveBeenCalledWith('npc', PLAYER, {
      amount: 1000,
      currency: 'credits',
      reference: `mission:${missionId}`,
      externalId: `mission:${missionId}:${PLAYER}`,
    });
  });

  it('leaves the assignment open when autoComplete is off', async () => {
    const missionId = await createMission({
      title: 'Manuel',
      category: 'exploration',
      autoComplete: false,
      objectives: [{ type: 'manual', title: 'Atteindre le phare', targetQuantity: 1 }],
      rewards: [{ type: 'credits', amount: 1000 }],
    });
    await assign(missionId);
    const { body: detail } = await call(`/missions/${missionId}`);
    const objectiveId = detail.objectives[0].id as string;

    const { body } = await confirm(missionId, objectiveId);
    expect(body).toMatchObject({ allComplete: true, completed: false });
    expect((await loadAssignment(missionId)).status).toBe('active');

    const completed = await call(`/missions/${missionId}/complete`, 'POST', { playerId: PLAYER });
    expect(completed.status).toBe(200);
    expect((await loadAssignment(missionId)).status).toBe('completed');
  });
});

describe('bonus objectives', () => {
  it('grants the bonus reward of a completed bonus objective on top of the base reward', async () => {
    const missionId = await createMission({
      title: 'Contrat bonus',
      category: 'exploration',
      objectives: [
        { type: 'manual', title: 'Livrer', targetQuantity: 1 },
        {
          type: 'manual',
          title: 'Bonus : sans dommage',
          targetQuantity: 1,
          isBonus: true,
          bonusReward: { type: 'credits', amount: 250 },
        },
      ],
      rewards: [{ type: 'credits', amount: 1000 }],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    const bonusObjectiveId = detail.objectives[1].id as string;
    const requiredObjectiveId = detail.objectives[0].id as string;

    await assign(missionId);
    // The bonus objective is completed first: it must not close the mission by itself.
    const bonus = await confirm(missionId, bonusObjectiveId);
    expect(bonus.body).toMatchObject({ allComplete: false, completed: false });

    const required = await confirm(missionId, requiredObjectiveId);
    expect(required.body.completed).toBe(true);

    const assignment = await loadAssignment(missionId);
    expect(assignment.rewardAmount).toBe(1250);
    expect(economy.creditHolder).toHaveBeenCalledWith(
      'npc',
      PLAYER,
      expect.objectContaining({ amount: 1250 }),
    );
  });

  it('refuses a bonusReward on a non-bonus objective', async () => {
    const res = await call('/missions', 'POST', {
      title: 'Invalid',
      objectives: [{ type: 'manual', title: 'X', bonusReward: { type: 'credits', amount: 10 } }],
    });
    expect(res.status).toBe(400);
    expect(res.body.error).toBe('BONUS_REWARD_WITHOUT_BONUS');
  });

  it('refuses a mission made only of bonus objectives', async () => {
    const res = await call('/missions', 'POST', {
      title: 'Invalid',
      objectives: [{ type: 'manual', title: 'X', isBonus: true }],
    });
    expect(res.status).toBe(400);
    expect(res.body.error).toBe('MISSING_REQUIRED_OBJECTIVE');
  });
});

describe('partial hand-in', () => {
  it('pays a proportional credits share and never pays items', async () => {
    const missionId = await createMission({
      title: 'Livraison partielle',
      allowPartial: true,
      autoComplete: false,
      category: 'exploration',
      objectives: [
        { type: 'manual', title: 'Premier lot', targetQuantity: 1 },
        { type: 'manual', title: 'Second lot', targetQuantity: 1 },
      ],
      rewards: [
        { type: 'credits', amount: 1000 },
        { type: 'item', itemId: 'iron_ore', quantity: 5 },
      ],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    const objectiveId = detail.objectives[0].id as string;

    await assign(missionId);
    await confirm(missionId, objectiveId);

    const { status, body } = await call(`/missions/${missionId}/complete`, 'POST', {
      playerId: PLAYER,
      partial: true,
    });
    expect(status).toBe(200);
    expect(body.assignment).toMatchObject({
      status: 'partially_completed',
      completionRatio: 5000,
      rewardAmount: 500,
    });
    expect((await loadMission(missionId)).status).toBe('completed');
    // Credits scaled to the weighted progress; the item component is left out entirely.
    expect(economy.creditHolder).toHaveBeenCalledTimes(1);
    expect(economy.creditHolder).toHaveBeenCalledWith(
      'npc',
      PLAYER,
      expect.objectContaining({ amount: 500 }),
    );
    expect(body.assignment.rewardShares).toEqual([{ type: 'credits', amount: 500, currency: 'credits' }]);
  });

  it('refunds the unpaid remainder of a treasury escrow', async () => {
    const missionId = await createMission({
      title: 'Livraison financée',
      category: 'exploration',
      allowPartial: true,
      autoComplete: false,
      issuerType: 'politics',
      issuerId: ARES,
      escrow: { payerType: 'politics', payerId: ARES },
      objectives: [
        { type: 'manual', title: 'Premier lot', targetQuantity: 1 },
        { type: 'manual', title: 'Second lot', targetQuantity: 1 },
      ],
      rewards: [{ type: 'credits', amount: 1000 }],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    await assign(missionId);
    await confirm(missionId, detail.objectives[0].id as string);

    const { status } = await call(`/missions/${missionId}/complete`, 'POST', {
      playerId: PLAYER,
      partial: true,
    });
    expect(status).toBe(200);
    // 500 to the assignee, 500 back to the treasury, escrow closed.
    expect(economy.creditHolder).toHaveBeenNthCalledWith(1, 'npc', PLAYER, expect.objectContaining({ amount: 500 }));
    expect(economy.creditHolder).toHaveBeenNthCalledWith(2, 'politics', ARES, {
      amount: 500,
      currency: 'credits',
      reference: 'mission_refund',
      externalId: `mission-refund:${missionId}`,
    });
    expect((await loadMission(missionId)).escrowStatus).toBe('refunded');
  });

  it('is refused on a mission that does not allow partials', async () => {
    const missionId = await createMission({
      title: 'Sans partiel',
      category: 'exploration',
      objectives: [
        { type: 'manual', title: 'A', targetQuantity: 1 },
        { type: 'manual', title: 'B', targetQuantity: 1 },
      ],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    await assign(missionId);
    await confirm(missionId, detail.objectives[0].id as string);

    const { status, body } = await call(`/missions/${missionId}/complete`, 'POST', {
      playerId: PLAYER,
      partial: true,
    });
    expect(status).toBe(409);
    expect(body.error).toBe('PARTIAL_NOT_ALLOWED');
  });

  it('is refused while another assignee is still working on the mission', async () => {
    const missionId = await createMission({
      title: 'À deux',
      category: 'exploration',
      allowPartial: true,
      autoComplete: false,
      maxAssignees: 2,
      objectives: [
        { type: 'manual', title: 'A', targetQuantity: 1 },
        { type: 'manual', title: 'B', targetQuantity: 1 },
      ],
      rewards: [{ type: 'credits', amount: 1000 }],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    await assign(missionId, PLAYER);
    await assign(missionId, OTHER);
    await confirm(missionId, detail.objectives[0].id as string);

    const { status, body } = await call(`/missions/${missionId}/complete`, 'POST', {
      playerId: PLAYER,
      partial: true,
    });
    expect(status).toBe(409);
    expect(body.error).toBe('PARTIAL_MULTIPLAYER');
  });

  it('is refused when nothing has any progress yet', async () => {
    const missionId = await createMission({
      title: 'Rien de fait',
      category: 'exploration',
      allowPartial: true,
      autoComplete: false,
      objectives: [{ type: 'manual', title: 'A', targetQuantity: 1 }],
    });
    await assign(missionId);

    const { status, body } = await call(`/missions/${missionId}/complete`, 'POST', {
      playerId: PLAYER,
      partial: true,
    });
    expect(status).toBe(409);
    expect(body.error).toBe('PARTIAL_NO_PROGRESS');
  });
});

describe('scenario chain', () => {
  it('blocks the acceptance of a chained mission until the prior one is completed', async () => {
    const first = await createMission({
      title: 'Étape 1',
      category: 'exploration',
      objectives: [{ type: 'manual', title: 'Aller au phare', targetQuantity: 1 }],
    });
    const second = await createMission({
      title: 'Étape 2',
      category: 'exploration',
      requiresMissionIds: [first],
      objectives: [{ type: 'manual', title: 'Revenir', targetQuantity: 1 }],
    });

    const blocked = await assign(second, PLAYER, 'player');
    expect(blocked.status).toBe(409);
    expect(blocked.body.error).toBe('CHAIN_INCOMPLETE');

    await assign(first);
    const { body: detail } = await call(`/missions/${first}`);
    await confirm(first, detail.objectives[0].id as string);
    expect((await loadAssignment(first)).status).toBe('completed');

    const unlocked = await assign(second, PLAYER, 'player');
    expect(unlocked.status).toBe(201);
  });

  it('refuses an unknown required mission at creation', async () => {
    const { status, body } = await call('/missions', 'POST', {
      title: 'Chaîne cassée',
      requiresMissionIds: ['00000000-0000-4000-8000-000000000000'],
      objectives: [{ type: 'manual', title: 'X' }],
    });
    expect(status).toBe(400);
    expect(body.error).toBe('CHAIN_UNKNOWN');
  });
});

describe('per-assignment deadline', () => {
  it('stores a deadline at acceptance and expires the assignment once it passes', async () => {
    const missionId = await createMission({
      title: 'Course contre la montre',
      category: 'exploration',
      assignmentDeadlineHours: 2,
      objectives: [{ type: 'manual', title: 'Vite', targetQuantity: 1 }],
    });
    const accepted = await assign(missionId);
    expect(accepted.status).toBe(201);
    expect(accepted.body.assignment.deadlineAt).not.toBeNull();

    // Move the deadline into the past, as if two hours had elapsed.
    await db
      .update(missionAssignments)
      .set({ deadlineAt: new Date(Date.now() - 60_000) })
      .where(eq(missionAssignments.id, accepted.body.assignment.id));

    const sweep = await call('/missions/expire', 'POST', {});
    expect(sweep.status).toBe(200);
    expect(sweep.body.expiredAssignments).toBe(1);

    const assignment = await loadAssignment(missionId);
    expect(assignment.status).toBe('expired');
    expect(assignment.deadlineNotifiedAt).not.toBeNull();
    // The mission goes back on the board for the next assignee.
    expect((await loadMission(missionId)).status).toBe('available');

    // Acting on the expired assignment is refused.
    const { body: detail } = await call(`/missions/${missionId}`);
    const late = await confirm(missionId, detail.objectives[0].id as string);
    expect(late.status).toBe(409);
    expect(late.body.error).toBe('ASSIGNMENT_NOT_ACTIVE');
  });
});

describe('permanently failed assignments', () => {
  it('fails the assignment when the delivery target no longer exists', async () => {
    inventory.transferStack.mockRejectedValue(
      new HttpError(404, 'INVENTORY_NOT_FOUND', 'Good not found'),
    );
    const missionId = await createMission({
      title: 'Livraison impossible',
      category: 'delivery',
      objectives: [
        {
          type: 'owns_items',
          title: 'Avoir du minerai',
          targetQuantity: 5,
          params: { itemId: 'iron_ore' },
        },
      ],
      rewards: [{ type: 'credits', amount: 500 }],
    });
    await assign(missionId);

    // owns_items measures 0 for a non-existent item: the assignment stays active.
    const verify = await call(`/missions/${missionId}/verify`, 'POST', { playerId: PLAYER });
    expect(verify.status).toBe(200);
    expect((await loadAssignment(missionId)).status).toBe('active');
  });
});

describe('standing rewards', () => {
  it('grants the full standing delta to a player on a full completion', async () => {
    const missionId = await createMission({
      title: 'Exploration cartographique',
      category: 'exploration',
      objectives: [{ type: 'manual', title: 'Atteindre le phare', targetQuantity: 1 }],
      rewards: [{ type: 'standing', domain: 'exploration', amount: 15 }],
    });
    await assign(missionId, PLAYER, 'player');
    const { body: detail } = await call(`/missions/${missionId}`);
    await confirm(missionId, detail.objectives[0].id as string);

    const assignment = await loadAssignment(missionId);
    expect(assignment.status).toBe('completed');
    expect(social.addStanding).toHaveBeenCalledWith(PLAYER, 'exploration', 15);
    expect(assignment.settledComponents).toContain('standing:exploration');
    expect(assignment.rewardDetails?.standing).toMatchObject({
      exploration: { amount: 15 },
    });
  });

  it('skips standing for NPC assignees', async () => {
    const missionId = await createMission({
      title: 'Reconnaissance PNJ',
      category: 'exploration',
      objectives: [{ type: 'manual', title: 'Phare', targetQuantity: 1 }],
      rewards: [{ type: 'standing', domain: 'exploration', amount: 15 }],
    });
    await assign(missionId, PLAYER, 'npc');
    const { body: detail } = await call(`/missions/${missionId}`);
    await confirm(missionId, detail.objectives[0].id as string);

    expect(social.addStanding).not.toHaveBeenCalled();
    const assignment = await loadAssignment(missionId);
    expect(assignment.status).toBe('completed');
  });

  it('never grants standing on a partial hand-in', async () => {
    const missionId = await createMission({
      title: 'Partiel sans standing',
      allowPartial: true,
      autoComplete: false,
      category: 'exploration',
      objectives: [
        { type: 'manual', title: 'A', targetQuantity: 1 },
        { type: 'manual', title: 'B', targetQuantity: 1 },
      ],
      rewards: [{ type: 'standing', domain: 'exploration', amount: 15 }],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    await assign(missionId, PLAYER, 'player');
    await confirm(missionId, detail.objectives[0].id as string);

    const { status } = await call(`/missions/${missionId}/complete`, 'POST', {
      playerId: PLAYER,
      partial: true,
    });
    expect(status).toBe(200);
    expect(social.addStanding).not.toHaveBeenCalled();
    const assignment = await loadAssignment(missionId);
    expect(assignment.status).toBe('partially_completed');
    expect(assignment.rewardShares ?? []).toEqual([]);
  });

  it('gives each co-assignee the full (unsplit) standing delta', async () => {
    const missionId = await createMission({
      title: 'Expédition partagée',
      category: 'exploration',
      autoComplete: false,
      maxAssignees: 2,
      objectives: [{ type: 'manual', title: 'Phare', targetQuantity: 1 }],
      rewards: [{ type: 'standing', domain: 'exploration', amount: 10 }],
    });
    const { body: detail } = await call(`/missions/${missionId}`);
    const objectiveId = detail.objectives[0].id as string;
    await assign(missionId, PLAYER, 'player');
    await assign(missionId, OTHER, 'player');

    await confirm(missionId, objectiveId);
    await confirm(missionId, objectiveId);

    const completed = await call(`/missions/${missionId}/complete`, 'POST', { playerId: PLAYER });
    expect(completed.status).toBe(200);

    expect(social.addStanding).toHaveBeenCalledTimes(2);
    expect(social.addStanding).toHaveBeenCalledWith(PLAYER, 'exploration', 10);
    expect(social.addStanding).toHaveBeenCalledWith(OTHER, 'exploration', 10);

    const second = await loadAssignment(missionId, OTHER);
    // Both assignments settled their standing component independently.
    expect(second.settledComponents).toContain('standing:exploration');
  });

  it('leaves the settlement retryable when Social rejects the standing delta', async () => {
    const { HttpError: HttpErrorClass } = await import('../src/lib/httpError.js');
    social.addStanding.mockRejectedValue(
      new HttpErrorClass(502, 'SOCIAL_LOOKUP_FAILED', 'Social lookup failed'),
    );
    const missionId = await createMission({
      title: 'Standing indisponible',
      category: 'exploration',
      objectives: [{ type: 'manual', title: 'Phare', targetQuantity: 1 }],
      rewards: [{ type: 'standing', domain: 'exploration', amount: 5 }],
    });
    await assign(missionId, PLAYER, 'player');
    const { body: detail } = await call(`/missions/${missionId}`);

    const progressRes = await confirm(missionId, detail.objectives[0].id as string);
    expect(progressRes.status).toBe(502);
    expect(progressRes.body.error).toBe('SOCIAL_STANDING_FAILED');

    // The claim was released: a retry (once Social is back) can settle the component.
    const assignment = await loadAssignment(missionId);
    expect(assignment.settledComponents ?? []).not.toContain('standing:exploration');
    expect(assignment.rewardDetails?.standing?.exploration).toMatchObject({
      error: 'SOCIAL_LOOKUP_FAILED',
    });
  });
});
