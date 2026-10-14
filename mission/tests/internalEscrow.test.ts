/**
 * Internal missions funded by a political treasury (`escrow` on `POST /api/internal/missions`):
 * debit at creation, refund on cancel/expire, claim at settlement, role gate and refusals.
 * Economy is mocked; the database is an in-process PGlite migrated from `./drizzle`.
 */
import type { Server } from 'http';
import { eq } from 'drizzle-orm';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { HttpError } from '../src/lib/httpError.js';
import { startTestApp } from './helpers/testApp.js';

const economy = vi.hoisted(() => ({
  debitHolder: vi.fn(),
  creditHolder: vi.fn(),
}));

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
vi.mock('../src/services/economy.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/economy.client.js')>()),
  debitHolder: economy.debitHolder,
  creditHolder: economy.creditHolder,
}));

const { db } = await import('../src/db/connection.js');
const { missionObjectives, missions } = await import('../src/db/schema/index.js');

const ARES = '7b0c3f2e-1a4d-4c8e-9f21-3d5a6b7c8d90';
const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const ALL_ROLES = 'mission:write,mission:escrow:politics,mission:complete';

let server: Server;
let baseUrl: string;

/** Internal mission body funded by ARES (credits only, delivery to a warehouse). */
function escrowedMission(overrides: Record<string, unknown> = {}) {
  return {
    title: 'Transport de minerai pour ARES',
    category: 'delivery',
    issuerType: 'politics',
    issuerId: ARES,
    rewards: [{ type: 'credits', amount: 4000 }],
    escrow: { payerType: 'politics', payerId: ARES },
    objectives: [
      {
        type: 'owns_items',
        title: 'Avoir 50 minerais',
        targetQuantity: 50,
        params: { itemId: 'iron_ore' },
      },
    ],
    ...overrides,
  };
}

async function post(path: string, body: unknown, roles = ALL_ROLES) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'x-test-roles': roles },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: (await res.json()) as Record<string, any> };
}

async function loadMission(id: string) {
  const [row] = await db.select().from(missions).where(eq(missions.id, id));
  return row;
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});

afterAll(() => {
  server?.close();
});

beforeEach(() => {
  economy.debitHolder.mockReset().mockResolvedValue({ transaction: { id: 1 }, amount: 4000 });
  economy.creditHolder.mockReset().mockResolvedValue({ transaction: { id: 2 }, amount: 4000 });
});

describe('POST /api/internal/missions with escrow', () => {
  it('debits the political treasury and holds the escrow', async () => {
    const { status, body } = await post('/missions', escrowedMission());

    expect(status).toBe(201);
    const id = body.mission.id as string;
    expect(economy.debitHolder).toHaveBeenCalledTimes(1);
    expect(economy.debitHolder).toHaveBeenCalledWith('politics', ARES, {
      amount: 4000,
      currency: 'credits',
      reference: 'mission_escrow',
      externalId: `mission-escrow:${id}`,
    });
    expect(body.mission).toMatchObject({
      escrowStatus: 'held',
      escrowAmount: 4000,
      escrowCurrency: 'credits',
      escrowPayerType: 'politics',
      escrowPayerId: ARES,
      escrowExternalId: `mission-escrow:${id}`,
      createdBy: 'svc-universesimulator',
    });
    expect(body.objectives[0].params.itemId).toBe('iron_ore');
  });

  it('refunds the treasury when the mission is cancelled', async () => {
    const created = await post('/missions', escrowedMission());
    const id = created.body.mission.id as string;

    const { status } = await post(`/missions/${id}/cancel`, {});

    expect(status).toBe(200);
    expect(economy.creditHolder).toHaveBeenCalledWith('politics', ARES, {
      amount: 4000,
      currency: 'credits',
      reference: 'mission_refund',
      externalId: `mission-refund:${id}`,
    });
    expect((await loadMission(id)).escrowStatus).toBe('refunded');
  });

  it('refunds the treasury when the mission expires', async () => {
    const created = await post('/missions', escrowedMission({ expiresAt: new Date(Date.now() - 1000).toISOString() }));
    const id = created.body.mission.id as string;

    const { status } = await post('/missions/expire', {});

    expect(status).toBe(200);
    expect(economy.creditHolder).toHaveBeenCalledWith('politics', ARES, expect.objectContaining({
      amount: 4000,
      externalId: `mission-refund:${id}`,
    }));
    expect(await loadMission(id)).toMatchObject({ status: 'expired', escrowStatus: 'refunded' });
  });

  it('pays the assignee from the escrow and marks it claimed on settlement', async () => {
    const created = await post('/missions', escrowedMission());
    const id = created.body.mission.id as string;
    expect((await post(`/missions/${id}/assign`, { playerId: PLAYER })).status).toBe(201);

    const { status, body } = await post(`/missions/${id}/complete`, { playerId: PLAYER, force: true });

    expect(status).toBe(200);
    expect(body.settlement.settled).toBe(true);
    expect(economy.creditHolder).toHaveBeenCalledTimes(1);
    expect(economy.creditHolder).toHaveBeenCalledWith('player', PLAYER, {
      amount: 4000,
      currency: 'credits',
      reference: `mission:${id}`,
      externalId: `mission:${id}:${PLAYER}`,
    });
    expect(await loadMission(id)).toMatchObject({ status: 'completed', escrowStatus: 'claimed' });
  });

  it('returns 409 INSUFFICIENT_FUNDS and persists nothing when the treasury is short', async () => {
    economy.debitHolder.mockRejectedValueOnce(
      new HttpError(409, 'INSUFFICIENT_FUNDS', 'Insufficient balance for the mission escrow'),
    );

    const missionsBefore = (await db.select().from(missions)).length;
    const objectivesBefore = (await db.select().from(missionObjectives)).length;

    const { status, body } = await post('/missions', escrowedMission({ title: 'Broke treasury' }));

    expect(status).toBe(409);
    expect(body.error).toBe('INSUFFICIENT_FUNDS');
    expect(economy.debitHolder).toHaveBeenCalledTimes(1);
    expect(await db.select().from(missions)).toHaveLength(missionsBefore);
    expect(await db.select().from(missionObjectives)).toHaveLength(objectivesBefore);
  });

  it('refuses the escrow without the mission:escrow:politics role (403, no debit)', async () => {
    const { status, body } = await post('/missions', escrowedMission({ title: 'No role' }), 'mission:write');

    expect(status).toBe(403);
    expect(body.error).toBe('FORBIDDEN');
    expect(body.message).toContain('mission:escrow:politics');
    expect(economy.debitHolder).not.toHaveBeenCalled();
    expect(await db.select().from(missions).where(eq(missions.title, 'No role'))).toHaveLength(0);
  });

  it('rejects item rewards on an escrowed mission (400 ESCROW_ITEM_REWARD_UNSUPPORTED)', async () => {
    const { status, body } = await post(
      '/missions',
      escrowedMission({
        rewards: [
          { type: 'credits', amount: 4000 },
          { type: 'item', itemId: 'medkit', quantity: 1 },
        ],
      }),
    );

    expect(status).toBe(400);
    expect(body.error).toBe('ESCROW_ITEM_REWARD_UNSUPPORTED');
    expect(economy.debitHolder).not.toHaveBeenCalled();
  });

  it('rejects an escrow without a credits reward (400 ESCROW_CREDITS_REQUIRED)', async () => {
    const { status, body } = await post('/missions', escrowedMission({ rewards: [] }));

    expect(status).toBe(400);
    expect(body.error).toBe('ESCROW_CREDITS_REQUIRED');
  });

  it('rejects an unsupported payer type (400 VALIDATION_ERROR)', async () => {
    const { status, body } = await post(
      '/missions',
      escrowedMission({ escrow: { payerType: 'corporation', payerId: ARES } }),
    );

    expect(status).toBe(400);
    expect(body.error).toBe('VALIDATION_ERROR');
  });
});

describe('POST /api/internal/missions without escrow', () => {
  it('keeps faucet-funded missions untouched (no debit, no escrow)', async () => {
    const { escrow: _escrow, ...body } = escrowedMission();
    const { status, body: created } = await post('/missions', body, 'mission:write');

    expect(status).toBe(201);
    expect(economy.debitHolder).not.toHaveBeenCalled();
    expect(created.mission).toMatchObject({ escrowStatus: 'none', escrowExternalId: null });
  });

  it('persists the isEvent flag', async () => {
    const { escrow: _escrow, ...body } = escrowedMission();
    const { status, body: created } = await post('/missions', { ...body, isEvent: true, maxAssignees: 500 }, 'mission:write');

    expect(status).toBe(201);
    expect(created.mission).toMatchObject({ isEvent: true, maxAssignees: 500 });
  });

  it('rejects deliver_material with a missing zone', async () => {
    const { escrow: _escrow, ...body } = escrowedMission();
    const { status, body: error } = await post(
      '/missions',
      {
        ...body,
        objectives: [
          {
            type: 'deliver_material',
            title: 'Livrer',
            params: { goodType: 'ore.iron' },
          },
        ],
      },
      'mission:write',
    );

    expect(status).toBe(400);
    expect(error.error).toBe('INVALID_OBJECTIVE_PARAMS');
  });
});
