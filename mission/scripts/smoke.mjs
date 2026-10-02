/**
 * End-to-end smoke test for the Mission API.
 *
 * Exercises the mission lifecycle against a running server, using the dev bypass headers
 * (`INTERNAL_DEV_BYPASS=true`, `AUTH_DEV_BYPASS=true`) or real Keycloak service accounts:
 * creation, listing, acceptance, verifiable objective progress, scenario ordering,
 * completion and idempotent reward settlement.
 *
 *   node scripts/smoke.mjs
 *   BASE=http://host.docker.internal:3000 KEY=test-internal-key node scripts/smoke.mjs
 *
 * To also assert the actual Economy credit, point ECONOMY_BASE at a running Economy service
 * (same shared key when INTERNAL_DEV_BYPASS=true on Economy):
 *   BASE=... KEY=... ECONOMY_BASE=http://host.docker.internal:3000 node scripts/smoke.mjs
 *
 * Exits with code 1 as soon as one assertion failed.
 */

const BASE = process.env.BASE ?? 'http://localhost:3000';
const KEY = process.env.KEY ?? 'test-internal-key';
const ECONOMY_BASE = process.env.ECONOMY_BASE ?? '';
const ECONOMY_KEY = process.env.ECONOMY_KEY ?? KEY;

const KC_BASE = process.env.KC_BASE ?? '';
const KC_REALM = process.env.KC_REALM ?? 'dyingstar';
const KC_CLIENT_ID = process.env.KC_CLIENT_ID ?? '';
const KC_CLIENT_SECRET = process.env.KC_CLIENT_SECRET ?? '';

const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const C = '33333333-3333-4333-8333-333333333333';
const D = '44444444-4444-4444-8444-444444444444';

const asPlayer = (id, name = 'alice') => ({ 'X-Player-Id': id, 'X-Player-Name': name });

/** Fetches a Keycloak service-account token (client_credentials grant). */
async function serviceToken(clientId, clientSecret) {
  const res = await fetch(`${KC_BASE}/realms/${KC_REALM}/protocol/openid-connect/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'client_credentials',
      client_id: clientId,
      client_secret: clientSecret,
    }),
  });
  if (!res.ok) throw new Error(`token request for ${clientId} failed: ${res.status} ${await res.text()}`);
  return (await res.json()).access_token;
}

const serviceTokenValue =
  KC_BASE && KC_CLIENT_ID && KC_CLIENT_SECRET ? await serviceToken(KC_CLIENT_ID, KC_CLIENT_SECRET) : null;
if (serviceTokenValue) {
  console.log(`# internal auth: Keycloak service account ${KC_CLIENT_ID}`);
} else {
  console.log('# internal auth: legacy X-Internal-Key (INTERNAL_DEV_BYPASS=true)');
}
const internal = serviceTokenValue ? { Authorization: `Bearer ${serviceTokenValue}` } : { 'X-Internal-Key': KEY };

let passed = 0;
const failures = [];

function check(label, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a === e) {
    passed++;
    console.log(`  ok   ${label}`);
  } else {
    failures.push(label);
    console.log(`  FAIL ${label}\n         expected ${e}\n         actual   ${a}`);
  }
}

/** Calls the API; never throws on 4xx so error responses can be asserted. */
async function req(method, path, body, headers = {}, base = BASE) {
  const res = await fetch(base + path, {
    method,
    headers: { 'Content-Type': 'application/json', ...headers },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = text;
  }
  return { status: res.status, json };
}

// ── Health and authentication ────────────────────────────────────────────────

console.log('\n# health and auth');
{
  const { status, json } = await req('GET', '/api/health');
  check('GET /api/health', [status, json.service], [200, 'mission']);

  const noAuth = await req('GET', '/api/missions');
  check('missions without credentials', [noAuth.status, noAuth.json.error], [401, 'UNAUTHORIZED']);

  const noCreds = await req('POST', '/api/internal/missions', {});
  check('internal route without credentials', [noCreds.status, noCreds.json.error], [401, 'UNAUTHORIZED']);
}

// ── Mission lifecycle ────────────────────────────────────────────────────────

console.log('\n# mission lifecycle');
let deliveryMissionId;
let deliveryObjectiveId;
{
  const created = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Livraison de minerai',
      description: 'Livrer 10 unités de minerai au comptoir',
      kind: 'dynamic',
      category: 'delivery',
      issuerType: 'system',
      objectives: [
        { type: 'deliver_material', title: 'Minerai livré', targetQuantity: 10, unit: 'units' },
      ],
    },
    internal,
  );
  check('create mission', [created.status, created.json.mission?.status, created.json.objectives?.length], [201, 'available', 1]);
  deliveryMissionId = created.json.mission?.id;
  deliveryObjectiveId = created.json.objectives?.[0]?.id;

  const list = await req('GET', '/api/missions', undefined, asPlayer(A));
  check('available missions listed for a player', list.json.missions?.some((m) => m.id === deliveryMissionId), true);

  const detail = await req('GET', `/api/missions/${deliveryMissionId}`, undefined, asPlayer(A));
  check('mission detail exposes objectives', detail.json.objectives?.[0]?.targetQuantity, 10);

  const accepted = await req('POST', `/api/missions/${deliveryMissionId}/accept`, undefined, asPlayer(A));
  check('accept mission', [accepted.status, accepted.json.assignment?.status], [201, 'active']);

  const duplicate = await req('POST', `/api/missions/${deliveryMissionId}/accept`, undefined, asPlayer(A));
  check('accepting twice', [duplicate.status, duplicate.json.error], [409, 'ALREADY_ASSIGNED']);

  const partial = await req(
    'POST',
    `/api/missions/${deliveryMissionId}/objectives/${deliveryObjectiveId}/progress`,
    { quantity: 4 },
    asPlayer(A),
  );
  check('partial progress', [partial.json.objective?.currentProgress, partial.json.objective?.status, partial.json.allComplete], [4, 'in_progress', false]);

  const early = await req('POST', `/api/missions/${deliveryMissionId}/complete`, undefined, asPlayer(A));
  check('completing an unfinished mission', [early.status, early.json.error], [409, 'OBJECTIVES_INCOMPLETE']);

  const done = await req(
    'POST',
    `/api/missions/${deliveryMissionId}/objectives/${deliveryObjectiveId}/progress`,
    { quantity: 6 },
    asPlayer(A),
  );
  check('progress reaches the target', [done.json.objective?.currentProgress, done.json.objective?.status, done.json.allComplete], [10, 'completed', true]);

  const completed = await req('POST', `/api/missions/${deliveryMissionId}/complete`, undefined, asPlayer(A));
  check('complete mission', [completed.status, completed.json.assignment?.status, completed.json.assignment?.rewardSettled], [200, 'completed', true]);

  const mine = await req('GET', '/api/me/missions?status=completed', undefined, asPlayer(A));
  check('my completed missions', mine.json.missions?.some((m) => m.mission?.id === deliveryMissionId), true);
}

// ── Scenario ordering ────────────────────────────────────────────────────────

console.log('\n# scenario ordering');
{
  const created = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Reconnaissance scénarisée',
      kind: 'scenario',
      category: 'generic',
      scriptId: 'scenario-scout-1',
      objectives: [
        { type: 'visit', title: 'Étape 1', targetQuantity: 1, order: 0 },
        { type: 'visit', title: 'Étape 2', targetQuantity: 1, order: 1 },
      ],
    },
    internal,
  );
  const missionId = created.json.mission?.id;
  const [first, second] = created.json.objectives ?? [];

  await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(B));
  const locked = await req(
    'POST',
    `/api/missions/${missionId}/objectives/${second.id}/progress`,
    { quantity: 1 },
    asPlayer(B),
  );
  check('locked scenario objective', [locked.status, locked.json.error], [409, 'OBJECTIVE_LOCKED']);

  await req('POST', `/api/missions/${missionId}/objectives/${first.id}/progress`, { quantity: 1 }, asPlayer(B));
  const unlocked = await req(
    'POST',
    `/api/missions/${missionId}/objectives/${second.id}/progress`,
    { quantity: 1 },
    asPlayer(B),
  );
  check('unlocked after the previous step', [unlocked.status, unlocked.json.allComplete], [200, true]);

  const duplicateScript = await req(
    'POST',
    '/api/internal/missions',
    { title: 'Doublon', scriptId: 'scenario-scout-1', objectives: [{ type: 'visit', title: 'x' }] },
    internal,
  );
  check('duplicate scriptId', [duplicateScript.status, duplicateScript.json.error], [409, 'DUPLICATE_SCRIPT_ID']);
}

// ── Capacity and cancellation ────────────────────────────────────────────────

console.log('\n# capacity and cancellation');
{
  const created = await req(
    'POST',
    '/api/internal/missions',
    { title: 'Mission unique', maxAssignees: 1, objectives: [{ type: 'visit', title: 'Unique' }] },
    internal,
  );
  const missionId = created.json.mission?.id;

  await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(C, 'carol'));
  const full = await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(D, 'dave'));
  check('mission at capacity', [full.status, full.json.error], [409, 'MISSION_FULL']);

  const cancelled = await req('POST', `/api/internal/missions/${missionId}/cancel`, undefined, internal);
  check('cancel mission', cancelled.json.status, 'cancelled');

  const acceptCancelled = await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(D, 'dave'));
  check('accepting a cancelled mission', [acceptCancelled.status, acceptCancelled.json.error], [409, 'MISSION_NOT_OPEN']);
}

// ── Validation ───────────────────────────────────────────────────────────────

console.log('\n# validation');
{
  const noObjectives = await req('POST', '/api/internal/missions', { title: 'Sans objectif' }, internal);
  check('mission without objectives', [noObjectives.status, noObjectives.json.error], [400, 'VALIDATION_ERROR']);

  const badId = await req('GET', '/api/missions/not-a-uuid', undefined, asPlayer(A));
  check('non-uuid mission id', [badId.status, badId.json.error], [400, 'VALIDATION_ERROR']);

  const zeroProgress = await req(
    'POST',
    `/api/missions/${deliveryMissionId}/objectives/${deliveryObjectiveId}/progress`,
    { quantity: 0 },
    asPlayer(A),
  );
  check('zero progress', [zeroProgress.status, zeroProgress.json.error], [400, 'VALIDATION_ERROR']);
}

// ── Economic reward settlement (optional, needs a running Economy) ────────────

/** Economy wallet balance of a player (credits account). */
async function playerBalance(id) {
  const { json } = await req(
    'GET',
    `/api/internal/players/${id}/wallet`,
    undefined,
    { 'X-Internal-Key': ECONOMY_KEY },
    ECONOMY_BASE,
  );
  return json.accounts?.find((a) => a.currency === 'credits')?.balance ?? 0;
}

console.log('\n# economic reward');
if (ECONOMY_BASE) {
  const reward = 750;
  const created = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Prime économique',
      category: 'transport',
      reward: { economic: { currency: 'credits', amount: reward } },
      objectives: [{ type: 'transport', title: 'Livrer la cargaison' }],
    },
    internal,
  );
  const missionId = created.json.mission?.id;
  const objectiveId = created.json.objectives?.[0]?.id;

  const before = await playerBalance(C);
  await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(C, 'carol'));
  await req('POST', `/api/missions/${missionId}/objectives/${objectiveId}/progress`, { quantity: 1 }, asPlayer(C, 'carol'));
  const completed = await req('POST', `/api/missions/${missionId}/complete`, undefined, asPlayer(C, 'carol'));
  check('economic reward settled', [completed.status, completed.json.settlement?.reason], [200, 'credited']);
  check('economy wallet credited', (await playerBalance(C)) - before, reward);

  const replay = await req('POST', `/api/internal/missions/${missionId}/settle`, { playerId: C }, internal);
  check('settle replay is idempotent', [replay.status, replay.json.settlements?.[0]?.reason], [200, 'already_settled']);

  // ── Multiplayer: shared objectives, equal split (remainder to the earliest) ──
  const total = 101;
  const multi = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Escorte partagée',
      maxAssignees: 2,
      reward: { economic: { currency: 'credits', amount: total } },
      objectives: [{ type: 'visit', title: 'Escorter le convoi' }],
    },
    internal,
  );
  const multiId = multi.json.mission?.id;
  const multiObjective = multi.json.objectives?.[0]?.id;

  const cBefore = await playerBalance(C);
  const dBefore = await playerBalance(D);
  await req('POST', `/api/missions/${multiId}/accept`, undefined, asPlayer(C, 'carol'));
  await req('POST', `/api/missions/${multiId}/accept`, undefined, asPlayer(D, 'dave'));
  await req('POST', `/api/missions/${multiId}/objectives/${multiObjective}/progress`, { quantity: 1 }, asPlayer(C, 'carol'));
  const multiDone = await req('POST', `/api/missions/${multiId}/complete`, undefined, asPlayer(C, 'carol'));
  check('multiplayer completes every participant', [multiDone.status, multiDone.json.assignments?.length, multiDone.json.settlements?.length], [200, 2, 2]);
  check('split shares sum to the reward', (await playerBalance(C)) - cBefore + ((await playerBalance(D)) - dBefore), total);
  check('earliest participant gets the remainder', (await playerBalance(C)) - cBefore, Math.ceil(total / 2));

  // ── Player-created mission: escrow held then refunded on cancel ─────────────
  const escrow = 300;
  const pBefore = await playerBalance(C);
  const playerMission = await req(
    'POST',
    '/api/missions',
    {
      title: 'Contrat de joueur',
      reward: { economic: { currency: 'credits', amount: escrow } },
      objectives: [{ type: 'deliver_material', title: 'Apporter la marchandise', targetQuantity: 5 }],
    },
    asPlayer(C, 'carol'),
  );
  check('player mission created and escrow held', [playerMission.status, playerMission.json.mission?.escrowStatus], [201, 'held']);
  check('escrow debited from the creator', pBefore - (await playerBalance(C)), escrow);

  const pMissionId = playerMission.json.mission?.id;
  await req('POST', `/api/internal/missions/${pMissionId}/cancel`, undefined, internal);
  check('escrow refunded on cancel', await playerBalance(C), pBefore);

  // ── Insufficient funds for the escrow ───────────────────────────────────────
  const broke = await req(
    'POST',
    '/api/missions',
    {
      title: 'Trop cher',
      reward: { economic: { currency: 'credits', amount: 1_000_000 } },
      objectives: [{ type: 'visit', title: 'Impossible' }],
    },
    asPlayer(A, 'alice'),
  );
  check('escrow refused without funds', [broke.status, broke.json.error], [409, 'INSUFFICIENT_FUNDS']);
} else {
  console.log('  skip real Economy credit (set ECONOMY_BASE to assert the wallet movement)');
}

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
