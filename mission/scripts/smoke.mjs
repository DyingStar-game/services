/**
 * End-to-end smoke test for the Mission API.
 *
 * Exercises the programmable pipeline against a running server, using the dev bypass
 * headers (`INTERNAL_DEV_BYPASS=true`, `AUTH_DEV_BYPASS=true`) or real Keycloak service
 * accounts: kind catalogue, spec validation, creation, listing (full missions hidden),
 * acceptance, game-objective progress, scenario ordering, manual (issuer) confirmation,
 * completion and idempotent reward settlement.
 *
 *   node scripts/smoke.mjs
 *   BASE=http://host.docker.internal:3000 KEY=test-internal-key node scripts/smoke.mjs
 *
 * To also assert the actual Economy credit/prerequisites, point ECONOMY_BASE at a running
 * Economy service (same shared key when INTERNAL_DEV_BYPASS=true on Economy):
 *   BASE=... KEY=... ECONOMY_BASE=http://host.docker.internal:3000 node scripts/smoke.mjs
 *
 * To exercise zone visibility/acceptance against a running Social service (the mission
 * service itself needs SOCIAL_API_URL + credentials to read presence):
 *   BASE=... KEY=... SOCIAL_BASE=http://host.docker.internal:3001 node scripts/smoke.mjs
 *
 * Exits with code 1 as soon as one assertion failed.
 */

const BASE = process.env.BASE ?? 'http://localhost:3000';
const KEY = process.env.KEY ?? 'test-internal-key';
const ECONOMY_BASE = process.env.ECONOMY_BASE ?? '';
const ECONOMY_KEY = process.env.ECONOMY_KEY ?? KEY;
const SOCIAL_BASE = process.env.SOCIAL_BASE ?? '';
const SOCIAL_KEY = process.env.SOCIAL_KEY ?? KEY;

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

const socialAuth = { 'X-Internal-Key': SOCIAL_KEY };

/** Writes a player's presence on Social (game-server channel). */
async function putPresence(playerId, body) {
  return req('PUT', `/api/internal/players/${playerId}/presence`, body, socialAuth, SOCIAL_BASE);
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

// ── Kind catalogue and spec validation ───────────────────────────────────────

console.log('\n# kind catalogue and spec validation');
{
  const catalog = await req('GET', '/api/missions/kinds', undefined, asPlayer(A));
  check(
    'kind catalogue',
    [
      catalog.status,
      catalog.json.objectiveKinds?.some((k) => k.kind === 'deliver_items' && k.evaluation === 'service'),
      catalog.json.objectiveKinds?.some((k) => k.kind === 'manual' && k.evaluation === 'issuer'),
      catalog.json.prerequisiteKinds?.some((k) => k.kind === 'has_credits'),
      Array.isArray(catalog.json.categories) && catalog.json.categories.length >= 10,
    ],
    [200, true, true, true, true],
  );

  const valid = await req(
    'POST',
    '/api/missions/validate',
    {
      mode: 'internal',
      mission: {
        title: 'Dry run',
        objectives: [{ type: 'owns_items', title: 'Avoir du fer', params: { itemId: 'iron_ore' }, targetQuantity: 10 }],
        rewards: [{ type: 'credits', currency: 'credits', amount: 10 }],
      },
    },
    asPlayer(A),
  );
  check('valid spec dry-run', [valid.status, valid.json.valid], [200, true]);

  const unknownKind = await req(
    'POST',
    '/api/missions/validate',
    { mission: { title: 'Bad', objectives: [{ type: 'nope', title: 'x' }] } },
    asPlayer(A),
  );
  check('unknown objective kind', [unknownKind.status, unknownKind.json.error], [400, 'UNKNOWN_OBJECTIVE_KIND']);

  const badParams = await req(
    'POST',
    '/api/missions/validate',
    { mission: { title: 'Bad params', objectives: [{ type: 'deliver_material', title: 'x' }] } },
    asPlayer(A),
  );
  check('missing kind params', [badParams.status, badParams.json.error], [400, 'INVALID_OBJECTIVE_PARAMS']);

  // i18n: Accept-Language: fr localizes the catalogue summaries.
  const frCatalog = await req('GET', '/api/missions/kinds', undefined, {
    ...asPlayer(A),
    'Accept-Language': 'fr',
  });
  check(
    'kind catalogue in French',
    frCatalog.json.objectiveKinds?.some((k) => k.kind === 'deliver_material' && k.summary.startsWith('Livrer')),
    true,
  );
  check(
    'kind catalogue in English by default',
    (await req('GET', '/api/missions/kinds', undefined, asPlayer(A))).json.objectiveKinds?.find(
      (k) => k.kind === 'deliver_material',
    )?.summary.startsWith('Deliver'),
    true,
  );

  // Category constraint: combat cannot use deliver_material, custom always fits.
  const badCategory = await req(
    'POST',
    '/api/missions/validate',
    {
      mission: {
        title: 'Combat livraison',
        category: 'combat',
        objectives: [{ type: 'deliver_material', title: 'x', params: { itemId: 'ore' } }],
      },
    },
    asPlayer(A),
  );
  check('category rejects objective kind', [badCategory.status, badCategory.json.error], [400, 'OBJECTIVE_NOT_IN_CATEGORY']);
  const okCategory = await req(
    'POST',
    '/api/missions/validate',
    { mission: { title: 'Combat ok', category: 'combat', objectives: [{ type: 'custom', title: 'x' }] } },
    asPlayer(A),
  );
  check('category accepts matching kind', [okCategory.status, okCategory.json.valid], [200, true]);
  const genericBypass = await req(
    'POST',
    '/api/missions/validate',
    {
      mission: {
        title: 'Generic libre',
        category: 'generic',
        objectives: [{ type: 'deliver_material', title: 'x', params: { itemId: 'ore' } }],
      },
    },
    asPlayer(A),
  );
  check('generic category bypasses the constraint', [genericBypass.status, genericBypass.json.valid], [200, true]);
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
        {
          type: 'deliver_material',
          title: 'Minerai livré',
          targetQuantity: 10,
          unit: 'units',
          params: { itemId: 'iron_ore' },
        },
      ],
    },
    internal,
  );
  check('create mission', [created.status, created.json.mission?.status, created.json.objectives?.length], [201, 'available', 1]);
  deliveryMissionId = created.json.mission?.id;
  deliveryObjectiveId = created.json.objectives?.[0]?.id;
  check('objective kind params stored', created.json.objectives?.[0]?.params, { itemId: 'iron_ore' });

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

  const verify = await req('POST', `/api/missions/${deliveryMissionId}/verify`, undefined, asPlayer(A));
  check('verify endpoint (game kinds skipped)', [verify.status, verify.json.allComplete], [409, 'ASSIGNMENT_NOT_ACTIVE']);
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

// ── Manual (issuer) objectives ───────────────────────────────────────────────

console.log('\n# manual objective (issuer confirmation)');
{
  const created = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Contrat libre',
      objectives: [{ type: 'manual', title: 'Construire une maison' }],
    },
    internal,
  );
  const missionId = created.json.mission?.id;
  const objectiveId = created.json.objectives?.[0]?.id;
  check('manual objective target normalized', created.json.objectives?.[0]?.targetQuantity, 1);

  await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(A));

  const push = await req(
    'POST',
    `/api/missions/${missionId}/objectives/${objectiveId}/progress`,
    { quantity: 1 },
    asPlayer(A),
  );
  check('push on a non-game kind is rejected', [push.status, push.json.error], [409, 'OBJECTIVE_NOT_PUSHABLE']);

  const confirmed = await req(
    'POST',
    `/api/internal/missions/${missionId}/objectives/${objectiveId}/confirm`,
    {},
    internal,
  );
  check('issuer confirmation completes the objective', [confirmed.status, confirmed.json.allComplete], [200, true]);

  const again = await req(
    'POST',
    `/api/internal/missions/${missionId}/objectives/${objectiveId}/confirm`,
    {},
    internal,
  );
  check('confirmation is idempotent', [again.status, again.json.objective?.status], [200, 'completed']);

  const completed = await req('POST', `/api/missions/${missionId}/complete`, undefined, asPlayer(A));
  check('manual mission completes', [completed.status, completed.json.assignment?.status], [200, 'completed']);
}

// ── Zones (availability by position) ─────────────────────────────────────────

console.log('\n# zones');
let zSystemId;
let zSceneId;
let zAreaId;
{
  const mk = async (title, zones) => {
    const created = await req(
      'POST',
      '/api/internal/missions',
      { title, zones, objectives: [{ type: 'visit', title: 'Exploration' }] },
      internal,
    );
    return created.json.mission?.id;
  };
  zSystemId = await mk('Zone système', [{ kind: 'system', system: 'tarsis' }]);
  zSceneId = await mk('Zone scène', [{ kind: 'scene', system: 'tarsis', scene: 'tarsis_1/new-paris' }]);
  zAreaId = await mk('Zone area', [{ kind: 'area', system: 'tarsis', center: { x: 0, y: 0, z: 0 }, radiusM: 500 }]);
  check('zoned missions created', [zSystemId, zSceneId, zAreaId].every(Boolean), true);

  const zoned = [zSystemId, zSceneId, zAreaId];
  const listed = async (playerId, name) =>
    (await req('GET', '/api/missions', undefined, asPlayer(playerId, name))).json.missions ?? [];

  if (SOCIAL_BASE) {
    // Deterministic start: profile first (presence requires it), then clear any presence
    // left by a previous run.
    const profile = await req('PUT', `/api/internal/players/${D}`, { displayName: 'dave' }, socialAuth, SOCIAL_BASE);
    check('social profile upsert (zone block)', profile.status, 200);
    const cleared = await putPresence(D, { status: 'offline', location: null });
    check('social presence cleared', cleared.status, 200);
  }

  // Without a location (or with Social unreachable), only global missions are listed.
  const hidden = await listed(D, 'dave');
  check(
    'zoned missions hidden without a location',
    zoned.every((id) => !hidden.some((m) => m.id === id)),
    true,
  );

  // The game can re-zone a live mission through the internal PATCH.
  const patched = await req('PATCH', `/api/internal/missions/${zSystemId}`, { zones: [{ kind: 'system', system: 'vega' }] }, internal);
  check('internal re-zone', [patched.status, patched.json.zones?.[0]?.system], [200, 'vega']);
  await req('PATCH', `/api/internal/missions/${zSystemId}`, { zones: [{ kind: 'system', system: 'tarsis' }] }, internal);

  if (SOCIAL_BASE) {
    const inside = await putPresence(D, {
      status: 'online',
      location: { system: 'tarsis', scene: 'tarsis_1/new-paris', position: { x: 100, y: 0, z: 0 } },
    });
    check('presence written inside the zone', inside.status, 200);
    const visible = await listed(D, 'dave');
    check(
      'zoned missions visible inside the zone (system, scene prefix, area radius)',
      zoned.every((id) => visible.some((m) => m.id === id)),
      true,
    );

    await putPresence(D, { status: 'online', location: { system: 'other', scene: 'elsewhere' } });
    const outside = await listed(D, 'dave');
    check(
      'zoned missions hidden outside the zone',
      zoned.every((id) => !outside.some((m) => m.id === id)),
      true,
    );

    const rejected = await req('POST', `/api/missions/${zSceneId}/accept`, undefined, asPlayer(D, 'dave'));
    check('accept outside the zone', [rejected.status, rejected.json.error], [403, 'OUT_OF_ZONE']);

    await putPresence(D, {
      status: 'online',
      location: { system: 'tarsis', scene: 'tarsis_1/new-paris', position: { x: 0, y: 0, z: 0 } },
    });
    const accepted = await req('POST', `/api/missions/${zSystemId}/accept`, undefined, asPlayer(D, 'dave'));
    check('accept inside the zone', [accepted.status, accepted.json.assignment?.status], [201, 'active']);
  } else {
    console.log('  skip zone visibility with Social (set SOCIAL_BASE + SOCIAL_KEY, mission needs SOCIAL_API_URL)');
  }
}

// ── Capacity and cancellation ────────────────────────────────────────────────

console.log('\n# capacity, listing and cancellation');
{
  const created = await req(
    'POST',
    '/api/internal/missions',
    { title: 'Mission unique', maxAssignees: 1, objectives: [{ type: 'visit', title: 'Unique' }] },
    internal,
  );
  const missionId = created.json.mission?.id;

  await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(C, 'carol'));

  // A full mission disappears from the available list.
  const list = await req('GET', '/api/missions', undefined, asPlayer(D, 'dave'));
  check('full mission hidden from the list', list.json.missions?.some((m) => m.id === missionId), false);

  const full = await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(D, 'dave'));
  check('mission at capacity', [full.status, full.json.error], [409, 'MISSION_FULL']);

  const fullFr = await req('POST', `/api/missions/${missionId}/accept`, undefined, {
    ...asPlayer(D, 'dave'),
    'Accept-Language': 'fr',
  });
  check('error message in French', [fullFr.status, fullFr.json.message], [409, "La mission n'a plus de place libre"]);
  const fullEn = await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(D, 'dave'));
  check('error message in English by default', [fullEn.status, fullEn.json.message], [409, 'Mission has no free slot']);

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

console.log('\n# economic reward and prerequisites');
if (ECONOMY_BASE) {
  const reward = 750;
  const created = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Prime économique',
      category: 'transport',
      rewards: [{ type: 'credits', currency: 'credits', amount: reward }],
      objectives: [{ type: 'transport', title: 'Livrer la cargaison', params: { itemId: 'iron_ore' } }],
    },
    internal,
  );
  const missionId = created.json.mission?.id;
  const objectiveId = created.json.objectives?.[0]?.id;
  check('mission rewards stored as components', created.json.mission?.rewards?.[0]?.type, 'credits');

  const before = await playerBalance(C);
  await req('POST', `/api/missions/${missionId}/accept`, undefined, asPlayer(C, 'carol'));
  await req('POST', `/api/missions/${missionId}/objectives/${objectiveId}/progress`, { quantity: 1 }, asPlayer(C, 'carol'));
  const completed = await req('POST', `/api/missions/${missionId}/complete`, undefined, asPlayer(C, 'carol'));
  check('economic reward settled', [completed.status, completed.json.settlement?.reason], [200, 'settled']);
  check('economy wallet credited', (await playerBalance(C)) - before, reward);
  check('credits share frozen on the assignment', completed.json.assignment?.rewardShares?.[0]?.amount, reward);

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
      rewards: [{ type: 'credits', currency: 'credits', amount: total }],
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
      rewards: [{ type: 'credits', currency: 'credits', amount: escrow }],
      objectives: [{ type: 'deliver_material', title: 'Apporter la marchandise', targetQuantity: 5, params: { itemId: 'iron_ore' } }],
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
      rewards: [{ type: 'credits', currency: 'credits', amount: 1_000_000 }],
      objectives: [{ type: 'visit', title: 'Impossible' }],
    },
    asPlayer(A, 'alice'),
  );
  check('escrow refused without funds', [broke.status, broke.json.error], [409, 'INSUFFICIENT_FUNDS']);

  // ── Prerequisites (has_credits) ─────────────────────────────────────────────
  const balance = await playerBalance(A);
  const poor = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Prérequis riche',
      prerequisites: [{ kind: 'has_credits', params: { amount: Math.max(balance + 1_000_000, 2_000_000_000) } }],
      objectives: [{ type: 'visit', title: 'Réservé aux riches' }],
    },
    internal,
  );
  const prereqFail = await req('POST', `/api/missions/${poor.json.mission?.id}/accept`, undefined, asPlayer(A, 'alice'));
  check('prerequisite failure', [prereqFail.status, prereqFail.json.error], [403, 'PREREQ_FAILED']);

  const rich = await req(
    'POST',
    '/api/internal/missions',
    {
      title: 'Prérequis ok',
      prerequisites: [{ kind: 'has_credits', params: { amount: 1 } }],
      objectives: [{ type: 'visit', title: 'Accessible' }],
    },
    internal,
  );
  const richAccept = await req('POST', `/api/missions/${rich.json.mission?.id}/accept`, undefined, asPlayer(A, 'alice'));
  check('prerequisite satisfied', [richAccept.status, richAccept.json.assignment?.status], [201, 'active']);
} else {
  console.log('  skip real Economy credit (set ECONOMY_BASE to assert the wallet movement and prereqs)');
}

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
