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
 * To exercise `poi` zones against a running Inventory service (the mission service needs
 * INVENTORY_API_URL + credentials to resolve POI geometry):
 *   BASE=... KEY=... INVENTORY_BASE=http://host.docker.internal:3002 node scripts/smoke.mjs
 *
 * Exits with code 1 as soon as one assertion failed.
 */

const BASE = process.env.BASE ?? 'http://localhost:3000';
const KEY = process.env.KEY ?? 'test-internal-key';
const ECONOMY_BASE = process.env.ECONOMY_BASE ?? '';
const ECONOMY_KEY = process.env.ECONOMY_KEY ?? KEY;
const SOCIAL_BASE = process.env.SOCIAL_BASE ?? '';
const SOCIAL_KEY = process.env.SOCIAL_KEY ?? KEY;

// POI zones resolve their geometry through the Inventory service.
const INVENTORY_BASE = process.env.INVENTORY_BASE ?? '';
const INVENTORY_KEY = process.env.INVENTORY_KEY ?? KEY;

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
      catalog.json.objectiveKinds?.every((k) => typeof k.name === 'string' && k.name.length > 0),
      catalog.json.prerequisiteKinds?.every((k) => typeof k.name === 'string' && k.name.length > 0),
      Array.isArray(catalog.json.categories) && catalog.json.categories.length >= 10,
    ],
    [200, true, true, true, true, true, true],
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
    [
      frCatalog.json.objectiveKinds?.some((k) => k.kind === 'deliver_material' && k.summary.startsWith('Livrer')),
      frCatalog.json.objectiveKinds?.find((k) => k.kind === 'deliver_material')?.name,
      frCatalog.json.prerequisiteKinds?.find((k) => k.kind === 'owns_items')?.name,
    ],
    [true, 'Livraison de matériaux', 'Objets requis'],
  );
  const enCatalog = await req('GET', '/api/missions/kinds', undefined, asPlayer(A));
  check(
    'kind catalogue in English by default',
    [
      enCatalog.json.objectiveKinds?.find((k) => k.kind === 'deliver_material')?.summary.startsWith('Deliver'),
      enCatalog.json.objectiveKinds?.find((k) => k.kind === 'deliver_material')?.name,
    ],
    [true, 'Material delivery'],
  );

  // Category constraint: combat cannot use deliver_material, manual always fits ('all').
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
    { mission: { title: 'Combat ok', category: 'combat', objectives: [{ type: 'manual', title: 'x' }] } },
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
  check('browse carries page bounds', [typeof list.json.total, typeof list.json.limit, typeof list.json.offset], ['number', 'number', 'number']);

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
  check('my missions are a page', [mine.json.total >= 1, typeof mine.json.limit, typeof mine.json.offset], [true, 'number', 'number']);

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
  const inventoryAuth = { 'X-Internal-Key': INVENTORY_KEY };
  const SYSTEM_ID = '00000000-0000-4000-8000-000000000001';

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

  // The `poi` kinds need a running Inventory: mission resolves their geometry through
  // INVENTORY_API_URL (a missing POI zone never matches, and creation validation fails
  // open only when Inventory itself is unreachable).
  if (INVENTORY_BASE) {
    const scenePoi = await req(
      'POST',
      '/api/internal/pois',
      { owner: { type: 'system', id: SYSTEM_ID }, name: 'smoke zone scene', system: 'tarsis', scene: 'tarsis_1/new-paris', x: 0, y: 0, z: 0 },
      inventoryAuth,
      INVENTORY_BASE,
    );
    const areaPoi = await req(
      'POST',
      '/api/internal/pois',
      { owner: { type: 'system', id: SYSTEM_ID }, name: 'smoke zone area', system: 'tarsis', x: 100, y: 0, z: 0, radiusM: 500 },
      inventoryAuth,
      INVENTORY_BASE,
    );
    check('zone POIs created', [scenePoi.status, areaPoi.status], [201, 201]);

    zSceneId = await mk('Zone poi scène', [{ kind: 'poi', poiId: scenePoi.json.id }]);
    zAreaId = await mk('Zone poi aire', [{ kind: 'poi', poiId: areaPoi.json.id, radiusM: 500 }]);
    check('zoned missions created', [zSystemId, zSceneId, zAreaId].every(Boolean), true);

    const bogus = await req(
      'POST',
      '/api/internal/missions',
      { title: 'Zone poi fantôme', zones: [{ kind: 'poi', poiId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' }], objectives: [{ type: 'visit', title: 'x' }] },
      internal,
    );
    check('mission on an unknown POI', [bogus.status, bogus.json.error], [400, 'POI_NOT_FOUND']);

    const legacy = await req(
      'POST',
      '/api/internal/missions',
      { title: 'Zone legacy', zones: [{ kind: 'scene', system: 'tarsis', scene: 'tarsis_1/new-paris' }], objectives: [{ type: 'visit', title: 'x' }] },
      internal,
    );
    check('legacy scene zone rejected', [legacy.status, legacy.json.error], [400, 'VALIDATION_ERROR']);
  } else {
    console.log('  skip POI zones (set INVENTORY_BASE + INVENTORY_KEY, mission needs INVENTORY_API_URL)');
    check('zoned missions created', Boolean(zSystemId), true);
  }

  const zoned = [zSystemId, zSceneId, zAreaId].filter(Boolean);
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

    const rejected = await req('POST', `/api/missions/${zSceneId ?? zSystemId}/accept`, undefined, asPlayer(D, 'dave'));
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

  const issuerNoOrg = await req(
    'POST',
    '/api/missions',
    {
      title: 'Trésor sans émetteur',
      escrowSource: 'issuer',
      rewards: [{ type: 'credits', currency: 'credits', amount: 10 }],
      objectives: [{ type: 'manual', title: 'x' }],
    },
    asPlayer(A, 'alice'),
  );
  check('issuer escrow without organization rejected', [issuerNoOrg.status, issuerNoOrg.json.error], [400, 'VALIDATION_ERROR']);
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

// ── Organization treasuries: corporation & political entity funding ──────────

console.log('\n# organization treasuries (corporation / politics)');
if (ECONOMY_BASE && SOCIAL_BASE) {
  const suffix = Date.now().toString(36).toUpperCase().slice(-4);
  /** Credits balance of a treasury (corporation or political entity). */
  const treasury = async (kind, id) => {
    const { json } = await req(
      'GET',
      `${ECONOMY_BASE}/api/internal/${kind}/${id}/wallet`,
      undefined,
      { 'X-Internal-Key': ECONOMY_KEY },
      ECONOMY_BASE,
    );
    return json.accounts?.find((a) => a.currency === 'credits')?.balance ?? 0;
  };

  // ── Corporation: CEO A funds a mission from the corporation treasury ───────
  const corp = await req(
    'POST',
    `${SOCIAL_BASE}/api/internal/corporations`,
    { ceoId: A, name: `Tresor ${suffix}`, ticker: `T${suffix}` },
    socialAuth,
    SOCIAL_BASE,
  );
  check('corporation created (CEO = A)', [corp.status, corp.json.ceoId], [201, A]);
  const corpId = corp.json.id;
  const fund = await req(
    'POST',
    `${ECONOMY_BASE}/api/internal/corporations/${corpId}/wallet/credit`,
    { amount: 5000, currency: 'credits', reference: 'smoke_treasury', externalId: `smoke-treasury:${corpId}` },
    { 'X-Internal-Key': ECONOMY_KEY },
    ECONOMY_BASE,
  );
  check('corporation treasury funded', [fund.status, await treasury('corporations', corpId)], [201, 5000]);

  const corpMission = await req(
    'POST',
    '/api/missions',
    {
      title: `Contrat corpo ${suffix}`,
      escrowSource: 'issuer',
      corporationId: corpId,
      rewards: [{ type: 'credits', currency: 'credits', amount: 500 }],
      objectives: [{ type: 'manual', title: 'Travail' }],
    },
    asPlayer(A, 'alice'),
  );
  check(
    'mission funded by the corporation treasury',
    [corpMission.status, corpMission.json.mission?.escrowStatus, corpMission.json.mission?.escrowPayerType],
    [201, 'held', 'corporation'],
  );
  check('corporation treasury debited', 5000 - (await treasury('corporations', corpId)), 500);

  // ── Big event flag: `mission:event:manage` decided by Social (A is the CEO) ─
  const eventOn = await req(
    'POST',
    `/api/missions/${corpMission.json.mission.id}/event`,
    { enabled: true, maxAssignees: 500 },
    asPlayer(A, 'alice'),
  );
  check('CEO flags the mission as an event', [eventOn.status, eventOn.json.isEvent], [200, true]);
  const eventDenied = await req(
    'POST',
    `/api/missions/${corpMission.json.mission.id}/event`,
    { enabled: false },
    asPlayer(B, 'bob'),
  );
  check(
    'non-member cannot change the event flag',
    [eventDenied.status, eventDenied.json.error],
    [403, 'NOT_CORPORATION_MEMBER'],
  );

  const outsider = await req(
    'POST',
    '/api/missions',
    {
      title: `Contrat volé ${suffix}`,
      escrowSource: 'issuer',
      corporationId: corpId,
      rewards: [{ type: 'credits', currency: 'credits', amount: 100 }],
      objectives: [{ type: 'manual', title: 'x' }],
    },
    asPlayer(B, 'bob'),
  );
  check('non-member cannot commit the treasury', [outsider.status, outsider.json.error], [403, 'TREASURY_FORBIDDEN']);

  await req('POST', `/api/internal/missions/${corpMission.json.mission.id}/cancel`, undefined, internal);
  check('corporation treasury refunded on cancel', await treasury('corporations', corpId), 5000);

  // ── Political entity: head A funds a mission from the commune treasury ─────
  const entity = await req(
    'POST',
    `${SOCIAL_BASE}/api/internal/politics`,
    { headId: A, type: 'commune', name: `Smokeville ${suffix}` },
    socialAuth,
    SOCIAL_BASE,
  );
  check('political entity created (head = A)', [entity.status, entity.json.headId], [201, A]);
  const entityId = entity.json.id;
  const fundP = await req(
    'POST',
    `${ECONOMY_BASE}/api/internal/politics/${entityId}/wallet/credit`,
    { amount: 3000, currency: 'credits', reference: 'smoke_treasury', externalId: `smoke-treasury:${entityId}` },
    { 'X-Internal-Key': ECONOMY_KEY },
    ECONOMY_BASE,
  );
  check('political treasury funded', [fundP.status, await treasury('politics', entityId)], [201, 3000]);

  const polMission = await req(
    'POST',
    '/api/missions',
    {
      title: `Travaux communaux ${suffix}`,
      escrowSource: 'issuer',
      politicalEntityId: entityId,
      rewards: [{ type: 'credits', currency: 'credits', amount: 800 }],
      objectives: [{ type: 'manual', title: 'Travaux' }],
    },
    asPlayer(A, 'alice'),
  );
  check(
    'mission funded by the political treasury',
    [polMission.status, polMission.json.mission?.escrowStatus, polMission.json.mission?.escrowPayerType],
    [201, 'held', 'politics'],
  );
  check('political treasury debited', 3000 - (await treasury('politics', entityId)), 800);

  await req('POST', `/api/internal/missions/${polMission.json.mission.id}/cancel`, undefined, internal);
  check('political treasury refunded on cancel', await treasury('politics', entityId), 3000);
} else {
  console.log('  skip organization treasury flows (set ECONOMY_BASE + SOCIAL_BASE)');
}

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
