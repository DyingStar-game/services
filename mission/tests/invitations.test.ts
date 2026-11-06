/**
 * Sharing a mission already taken: a participant invites one player or their own group; joining
 * opens an assignment of the SAME mission past its slot count (the invitation is the gate), so
 * progress is shared and the reward split equally between the participants, any of whom turns it
 * in. Economy, Inventory and Social are mocked; the database is an in-process PGlite.
 */
import type { Server } from 'http';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { startTestApp } from './helpers/testApp.js';

const A = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';
const B = '1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e';
const C = '2c3d4e5f-6a7b-4c8d-9e0f-1a2b3c4d5e6f';
const GROUP = '3d4e5f6a-7b8c-4d9e-8f1a-2b3c4d5e6f7a';

const social = vi.hoisted(() => ({
  notifyPlayer: vi.fn(),
  members: new Set<string>(),
}));
const economy = vi.hoisted(() => ({ creditHolder: vi.fn(), debitHolder: vi.fn() }));

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
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
  isGroupMember: async (playerId: string, groupId: string) => groupId === GROUP && social.members.has(playerId),
  getPlayerGroup: async (playerId: string) =>
    social.members.has(playerId) ? { group: { id: GROUP, name: 'Crew' }, member: { groupId: GROUP, playerId, joinedAt: '' } } : null,
  getGroup: async () => ({ id: GROUP, name: 'Crew', members: [...social.members].map((playerId) => ({ playerId })) }),
}));

const { db } = await import('../src/db/connection.js');
const { missionAssignments, missionInvitations } = await import('../src/db/schema/index.js');
const { listInvitationsFor } = await import('../src/services/invitations.service.js');

let server: Server;
let baseUrl: string;

async function internal(path: string, method = 'GET', body?: unknown) {
  const res = await fetch(`${baseUrl}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'x-test-roles': 'mission:read,mission:write,mission:complete,mission:progress' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return { status: res.status, body: (await res.json().catch(() => null)) as Record<string, any> };
}

async function asPlayer(player: string, path: string, body: unknown = {}) {
  const res = await fetch(`${baseUrl}/api/missions${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'x-test-player-id': player },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: (await res.json().catch(() => null)) as Record<string, any> };
}

/** A one-slot manual mission taken by A. */
async function takenByA(): Promise<string> {
  const { body } = await internal('/missions', 'POST', {
    title: 'Ore run',
    category: 'divers',
    maxAssignees: 1,
    autoComplete: false,
    objectives: [{ type: 'manual', title: 'Deliver', targetQuantity: 1 }],
    rewards: [{ type: 'credits', amount: 1000 }],
  });
  const id = body.mission.id as string;
  expect((await asPlayer(A, `/${id}/accept`)).status).toBe(201);
  return id;
}

beforeAll(async () => {
  ({ server, baseUrl } = await startTestApp());
});
afterAll(() => server?.close());
beforeEach(async () => {
  await db.delete(missionInvitations);
  await db.delete(missionAssignments);
  social.members = new Set();
  social.notifyPlayer.mockReset().mockResolvedValue(undefined);
  economy.creditHolder.mockReset().mockResolvedValue({ transaction: { id: 1 } });
});

describe('sharing a mission already taken', () => {
  it('a player invited by a participant joins it past its slot count, and is told', async () => {
    const id = await takenByA();
    const invited = await asPlayer(A, `/${id}/invite`, { playerId: B });
    expect(invited.status).toBe(201);
    expect(social.notifyPlayer).toHaveBeenCalledWith(B, expect.objectContaining({ type: 'mission.invited' }));
    expect((await listInvitationsFor(B)).map((i) => i.missionId)).toEqual([id]);

    expect((await asPlayer(B, `/invitations/${invited.body.id}/join`)).status).toBe(201);
    expect(await listInvitationsFor(B)).toEqual([]);
  });

  it('only a participant shares it, and only with their own group', async () => {
    const id = await takenByA();
    expect((await asPlayer(B, `/${id}/invite`, { playerId: C })).status).toBe(403);
    expect((await asPlayer(A, `/${id}/invite`, { groupId: GROUP })).status).toBe(403);
    social.members = new Set([A, B, C]);
    expect((await asPlayer(A, `/${id}/invite`, { groupId: GROUP })).status).toBe(201);
  });

  it('a group invitation lets every member join; somebody else cannot', async () => {
    social.members = new Set([A, B, C]);
    const id = await takenByA();
    const invitation = (await asPlayer(A, `/${id}/invite`, { groupId: GROUP })).body.id as string;
    expect((await asPlayer(B, `/invitations/${invitation}/join`)).status).toBe(201);
    expect((await asPlayer(C, `/invitations/${invitation}/join`)).status).toBe(201);
    const outsider = 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d';
    expect((await asPlayer(outsider, `/invitations/${invitation}/join`)).status).toBe(403);
  });

  it('any participant turns it in, and the reward is split equally', async () => {
    const id = await takenByA();
    const invitation = (await asPlayer(A, `/${id}/invite`, { playerId: B })).body.id as string;
    await asPlayer(B, `/invitations/${invitation}/join`);
    const turnedIn = await internal(`/missions/${id}/complete`, 'POST', { playerId: B, force: true });
    expect(turnedIn.status).toBe(200);
    const paid = economy.creditHolder.mock.calls.map((call) => call[2] ?? call);
    expect(economy.creditHolder).toHaveBeenCalledTimes(2);
    expect(JSON.stringify(paid)).toContain('500');
  });

  it('a declined invitation is gone', async () => {
    const id = await takenByA();
    const invitation = (await asPlayer(A, `/${id}/invite`, { playerId: B })).body.id as string;
    expect((await asPlayer(B, `/invitations/${invitation}/decline`)).status).toBe(204);
    expect(await listInvitationsFor(B)).toEqual([]);
  });
});

describe('the mission detail', () => {
  it('lists the participants to each of them', async () => {
    const id = await takenByA();
    const invitation = (await asPlayer(A, `/${id}/invite`, { playerId: B })).body.id as string;
    await asPlayer(B, `/invitations/${invitation}/join`);
    const res = await fetch(`${baseUrl}/api/missions/${id}`, { headers: { 'x-test-player-id': B } });
    const detail = (await res.json()) as { participants: { playerId: string }[] };
    expect(detail.participants.map((p) => p.playerId).sort()).toEqual([A, B].sort());
  });
});
