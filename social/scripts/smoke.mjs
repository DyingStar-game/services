/**
 * End-to-end smoke test for the Social API — focused on the ACL (central permission PDP).
 *
 * Exercises `POST /api/internal/authorize` and `GET /api/internal/permissions/catalog`
 * (plus the internal political hierarchy routes used by system-governed bootstraps)
 * against a running server: leader allowance, rank permissions, catalog `satisfiedBy`,
 * catalog `defaultMember`, free actions (exact match only), batch evaluation and the
 * validation/auth boundaries. Player-facing routes rely on `AUTH_DEV_BYPASS=true`
 * (`X-Player-Id`), internal routes on `INTERNAL_DEV_BYPASS=true` (`X-Internal-Key`) or a
 * Keycloak service account holding `social:authorize`:
 *
 *   node scripts/smoke.mjs
 *   BASE=http://host.docker.internal:3001 KEY=test-internal-key node scripts/smoke.mjs
 *
 * Against production-style auth (Keycloak service accounts), provide:
 *   KC_BASE=http://keycloak:8080 KC_CLIENT_ID=svc-mission KC_CLIENT_SECRET=... node scripts/smoke.mjs
 * and drop KEY; without Keycloak the legacy X-Internal-Key is used (INTERNAL_DEV_BYPASS=true).
 *
 * Exits with code 1 as soon as one assertion failed.
 */

const BASE = process.env.BASE ?? 'http://localhost:3001';
const KEY = process.env.KEY ?? 'test-internal-key';

const KC_BASE = process.env.KC_BASE ?? '';
const KC_REALM = process.env.KC_REALM ?? 'dyingstar';
const KC_CLIENT_ID = process.env.KC_CLIENT_ID ?? '';
const KC_CLIENT_SECRET = process.env.KC_CLIENT_SECRET ?? '';

/** Test fixtures. `A` is the CEO / head, `B` a plain corporation member, `D` a plain citizen. */
const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const D = '44444444-4444-4444-8444-444444444444';
/** Holder that does not exist, to assert fail-closed evaluation. */
const UNKNOWN_HOLDER = '99999999-9999-4999-8999-999999999999';

/** Makes fixture names unique per run so the script can be replayed. */
const suffix = Date.now().toString(36).toUpperCase().slice(-6);
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

/** Real service token when Keycloak is configured, else the dev-only shared key. */
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

/** Sends one authorization check and returns `[status, allowed, reason]`. */
async function authorize(checkBody, headers = internal) {
  const { status, json } = await req('POST', '/api/internal/authorize', checkBody, headers);
  return [status, json.allowed, json.reason];
}

const corpCheck = (playerId, action, holderId) => ({
  holderType: 'corporation',
  holderId,
  playerId,
  action,
});
const polCheck = (playerId, action, holderId) => ({
  holderType: 'political',
  holderId,
  playerId,
  action,
});

console.log('# health');
{
  const { status, json } = await req('GET', '/api/health');
  check('GET /api/health', [status, json.status], [200, 'ok']);
}

// ── Fixtures: one corporation (open recruitment) and one commune ─────────────

console.log('\n# fixtures');
const corpName = `Acl ${suffix}`;
const polName = `Aclville ${suffix}`;
let corpId = null;
let entityId = null;
{
  for (const [id, displayName] of [
    [A, 'Alice'],
    [B, 'Bob'],
    [D, 'Dave'],
  ]) {
    const profile = await req('PUT', `/api/internal/players/${id}`, { displayName }, internal);
    check(`profile ensured for ${displayName}`, profile.status, 200);
  }

  const corp = await req(
    'POST',
    '/api/internal/corporations',
    { ceoId: A, name: corpName, ticker: `S${suffix.slice(-4)}`, recruitment: 'open' },
    internal,
  );
  check('corporation created (CEO = A, open recruitment)', [corp.status, corp.json.ceoId], [201, A]);
  corpId = corp.json.id;

  const joined = await req(
    'POST',
    `/api/corporations/${corpId}/join`,
    { message: 'smoke' },
    asPlayer(B, 'bob'),
  );
  check('B joined the corporation (default rank)', [joined.status, joined.json.joined], [200, true]);

  const members = await req('GET', `/api/corporations/${corpId}/members`, undefined, asPlayer(A, 'alice'));
  check('members list is a page', [members.status, members.json.items.length >= 2, typeof members.json.total, typeof members.json.limit, typeof members.json.offset], [200, true, 'number', 'number', 'number']);
  const membersPage2 = await req('GET', `/api/corporations/${corpId}/members?limit=1&offset=2`, undefined, asPlayer(A, 'alice'));
  check('second members page is empty', [membersPage2.json.items.length, membersPage2.json.total >= 2], [0, true]);

  const entity = await req(
    'POST',
    '/api/internal/politics',
    { headId: A, type: 'commune', name: polName },
    internal,
  );
  check('political entity created (head = A)', [entity.status, entity.json.headId], [201, A]);
  entityId = entity.json.id;

  const seated = await req(
    'POST',
    `/api/politics/${entityId}/members`,
    { playerId: D },
    asPlayer(A, 'alice'),
  );
  check('D seated in the default office', seated.status, 201);
}

// ── Corporation checks ───────────────────────────────────────────────────────

console.log('\n# authorize (corporation)');
{
  const full = await req('POST', '/api/internal/authorize', corpCheck(A, 'manage_corporation', corpId), internal);
  check(
    'CEO holds a legacy permission',
    [full.status, full.json.allowed, full.json.reason, full.json.leader, full.json.role],
    [200, true, 'allowed', true, 'CEO'],
  );
  check('CEO permissions listed', full.json.permissions.length > 0, true);

  check(
    'CEO satisfies a cross-service action (inventory:poi:manage)',
    await authorize(corpCheck(A, 'inventory:poi:manage', corpId)),
    [200, true, 'allowed'],
  );
  check(
    'CEO holds an explicit action (economie:treasury:manage)',
    await authorize(corpCheck(A, 'economie:treasury:manage', corpId)),
    [200, true, 'allowed'],
  );

  const member = await req('POST', '/api/internal/authorize', corpCheck(B, 'manage_corporation', corpId), internal);
  check(
    'plain member lacks a legacy permission',
    [member.status, member.json.allowed, member.json.reason, member.json.role, member.json.leader],
    [200, false, 'missing_permission', 'Member', false],
  );
  check(
    'plain member lacks the explicit treasury action',
    await authorize(corpCheck(B, 'economie:treasury:manage', corpId)),
    [200, false, 'missing_permission'],
  );
  check(
    'every member may trade (market:trade, defaultMember)',
    await authorize(corpCheck(B, 'market:trade', corpId)),
    [200, true, 'allowed'],
  );
  check(
    'mission:treasury:commit needs manage_corporation (satisfiedBy)',
    await authorize(corpCheck(A, 'mission:treasury:commit', corpId)),
    [200, true, 'allowed'],
  );
  check(
    'plain member cannot commit the mission treasury',
    await authorize(corpCheck(B, 'mission:treasury:commit', corpId)),
    [200, false, 'missing_permission'],
  );

  const outsider = await req('POST', '/api/internal/authorize', corpCheck(D, 'manage_corporation', corpId), internal);
  check(
    'non-member is refused before any permission',
    [outsider.status, outsider.json.allowed, outsider.json.reason, outsider.json.role],
    [200, false, 'not_member', null],
  );
}

// ── Political checks ─────────────────────────────────────────────────────────

console.log('\n# authorize (political entity)');
{
  const head = await req('POST', '/api/internal/authorize', polCheck(A, 'manage_entity', entityId), internal);
  check(
    'head holds a legacy permission',
    [head.status, head.json.allowed, head.json.reason, head.json.leader, head.json.role],
    [200, true, 'allowed', true, 'Mayor'],
  );
  check(
    'head satisfies the POI action (satisfiedBy: manage_entity)',
    await authorize(polCheck(A, 'inventory:poi:manage', entityId)),
    [200, true, 'allowed'],
  );

  const citizen = await req('POST', '/api/internal/authorize', polCheck(D, 'manage_treasury', entityId), internal);
  check(
    'citizen office holds no permission',
    [citizen.status, citizen.json.allowed, citizen.json.reason, citizen.json.role],
    [200, false, 'missing_permission', 'Citizen'],
  );
  check(
    'citizen cannot manage the entity POIs',
    await authorize(polCheck(D, 'inventory:poi:manage', entityId)),
    [200, false, 'missing_permission'],
  );
  check(
    'non-member of the entity',
    await authorize(polCheck(B, 'manage_entity', entityId)),
    [200, false, 'not_member'],
  );
}

// ── Free actions: exact match only ───────────────────────────────────────────

console.log('\n# free actions');
{
  const ranks = await req('GET', `/api/corporations/${corpId}/ranks`, undefined, asPlayer(A, 'alice'));
  const memberRank = ranks.json.find((r) => r.isDefault);
  check('default rank starts with no permission', memberRank.permissions, []);

  const patched = await req(
    'PATCH',
    `/api/corporations/${corpId}/ranks/${memberRank.id}`,
    { permissions: ['game:custom_flag'] },
    asPlayer(A, 'alice'),
  );
  check('free action stored on a rank', [patched.status, patched.json.permissions], [200, ['game:custom_flag']]);

  check('free action matches exactly', await authorize(corpCheck(B, 'game:custom_flag', corpId)), [
    200,
    true,
    'allowed',
  ]);
  check('unrelated free action is refused', await authorize(corpCheck(B, 'game:other_flag', corpId)), [
    200,
    false,
    'missing_permission',
  ]);

  await req(
    'PATCH',
    `/api/corporations/${corpId}/ranks/${memberRank.id}`,
    { permissions: [] },
    asPlayer(A, 'alice'),
  );
}

// ── Batch evaluation ─────────────────────────────────────────────────────────

console.log('\n# batch');
{
  const { status, json } = await req(
    'POST',
    '/api/internal/authorize',
    {
      checks: [
        corpCheck(A, 'manage_corporation', corpId),
        corpCheck(D, 'manage_corporation', corpId),
        polCheck(D, 'manage_treasury', entityId),
      ],
    },
    internal,
  );
  check('batch answers 200 with one result per check', [status, json.results?.length], [200, 3]);
  check(
    'batch preserves order and reasons',
    json.results.map((r) => [r.allowed, r.reason]),
    [
      [true, 'allowed'],
      [false, 'not_member'],
      [false, 'missing_permission'],
    ],
  );

  const tooMany = await req(
    'POST',
    '/api/internal/authorize',
    {
      checks: Array.from({ length: 51 }, () => corpCheck(A, 'manage_corporation', corpId)),
    },
    internal,
  );
  check('more than 50 checks rejected', [tooMany.status, tooMany.json.error], [400, 'VALIDATION_ERROR']);
}

// ── Catalog ──────────────────────────────────────────────────────────────────

console.log('\n# permission catalog');
{
  const { status, json } = await req('GET', '/api/internal/permissions/catalog', undefined, internal);
  check('GET /api/internal/permissions/catalog', status, 200);
  check('every row is complete', json.actions.every((row) => row.action && row.holder && row.description), true);

  const poiCorp = json.actions.find((r) => r.action === 'inventory:poi:manage' && r.holder === 'corporation');
  check('POI action maps to manage_corporation', poiCorp.satisfiedBy, ['manage_corporation']);
  const poiPol = json.actions.find((r) => r.action === 'inventory:poi:manage' && r.holder === 'political');
  check('POI action maps to manage_entity (political)', poiPol.satisfiedBy, ['manage_entity']);
  const trade = json.actions.find((r) => r.action === 'market:trade');
  check('market:trade is open to every member', trade.defaultMember, true);
  const treasury = json.actions.find((r) => r.action === 'economie:treasury:manage');
  check('treasury action needs an explicit grant', [treasury.satisfiedBy ?? null, treasury.defaultMember ?? null], [
    null,
    null,
  ]);
  const legacy = json.actions.filter((r) => r.legacy);
  check('legacy permissions are flagged', legacy.length > 0, true);

  const fr = await req('GET', '/api/internal/permissions/catalog', undefined, {
    ...internal,
    'Accept-Language': 'fr',
  });
  const frTrade = fr.json.actions.find((r) => r.action === 'market:trade');
  check('descriptions are localized (fr)', frTrade.description, 'Placer et régler des échanges de marché pour la corporation');
  check('descriptions are localized (en)', trade.description, 'Place and settle market trades for the corporation');

  // Player-facing twin of the same catalog (JWT, or X-Player-Id in dev-bypass).
  const mine = await req('GET', '/api/me/permissions/catalog', undefined, asPlayer(A, 'alice'));
  check('GET /api/me/permissions/catalog', [mine.status, mine.json.actions?.length], [200, json.actions.length]);
  const mineFr = await req('GET', '/api/me/permissions/catalog', undefined, {
    ...asPlayer(A, 'alice'),
    'Accept-Language': 'fr',
  });
  check(
    'player catalog is localized (fr)',
    mineFr.json.actions.find((r) => r.action === 'market:trade').description,
    'Placer et régler des échanges de marché pour la corporation',
  );
  const anonymous = await req('GET', '/api/me/permissions/catalog');
  check('player catalog without credentials', [anonymous.status, anonymous.json.error], [401, 'UNAUTHORIZED']);
}

// ── Political hierarchy (internal, system-governed bootstrap) ────────────────

console.log('\n# politics hierarchy (internal)');
{
  const fedName = `Ares ${suffix}`;
  const fed = await req('POST', '/api/internal/politics', { type: 'federation', name: fedName }, internal);
  check('headless federation created', [fed.status, fed.json.headId, fed.json.parentId], [201, null, null]);
  const fedId = fed.json.id;

  const nullHead = await req(
    'POST',
    '/api/internal/politics',
    { headId: null, type: 'country', name: `Ares Prime ${suffix}` },
    internal,
  );
  check('headless country created (headId: null)', [nullHead.status, nullHead.json.headId], [201, null]);
  const countryId = nullHead.json.id;

  const region = await req('POST', '/api/internal/politics', { type: 'region', name: `Ares North ${suffix}` }, internal);
  check('headless region created', region.status, 201);
  const regionId = region.json.id;

  const fedOffices = await req('GET', `/api/politics/${fedId}/offices`, undefined, asPlayer(A, 'alice'));
  check(
    'headless entity still gets its default offices (head office vacant)',
    [fedOffices.status, fedOffices.json.some((o) => o.isHead), fedOffices.json.some((o) => o.isDefault)],
    [200, true, true],
  );
  const fedMembers = await req('GET', `/api/politics/${fedId}/members`, undefined, asPlayer(A, 'alice'));
  check('headless entity has no member', [fedMembers.status, fedMembers.json.total], [200, 0]);
  const fedPage = await req('GET', `/api/politics/${fedId}`, undefined, asPlayer(A, 'alice'));
  check('player page tolerates a null head', [fedPage.status, fedPage.json.headId], [200, null]);

  const got = await req('GET', `/api/internal/politics/${fedId}`, undefined, internal);
  check(
    'GET /internal/politics/:id returns the entity view',
    [got.status, got.json.id, got.json.type, got.json.name, got.json.headId, 'memberCount' in got.json],
    [200, fedId, 'federation', fedName, null, false],
  );
  const missing = await req('GET', `/api/internal/politics/${UNKNOWN_HOLDER}`, undefined, internal);
  check('GET /internal/politics/:id 404 when missing', [missing.status, missing.json.error], [404, 'NOT_FOUND']);
  const badId = await req('GET', '/api/internal/politics/not-a-uuid', undefined, internal);
  check('GET /internal/politics/:id rejects a non-uuid', [badId.status, badId.json.error], [400, 'VALIDATION_ERROR']);

  const byName = await req('GET', `/api/internal/politics?name=${encodeURIComponent(fedName)}`, undefined, internal);
  check(
    'lookup by exact name',
    [byName.status, byName.json.total, byName.json.items[0]?.id, byName.json.limit, byName.json.offset],
    [200, 1, fedId, 20, 0],
  );
  const partial = await req('GET', `/api/internal/politics?name=${encodeURIComponent(fedName.slice(0, -1))}`, undefined, internal);
  check('lookup by name is exact (no substring match)', [partial.status, partial.json.total], [200, 0]);
  const byType = await req('GET', `/api/internal/politics?type=federation&limit=100`, undefined, internal);
  check(
    'lookup by type filters the level',
    [byType.status, byType.json.items.every((e) => e.type === 'federation'), byType.json.items.some((e) => e.id === fedId)],
    [200, true, true],
  );
  const badType = await req('GET', '/api/internal/politics?type=empire', undefined, internal);
  check('lookup rejects an unknown type', [badType.status, badType.json.error], [400, 'VALIDATION_ERROR']);

  const linkCountry = await req('PUT', `/api/internal/politics/${countryId}/parent`, { parentId: fedId }, internal);
  check('country attached to the federation (system)', [linkCountry.status, linkCountry.json.parentId], [200, fedId]);
  const linkRegion = await req('PUT', `/api/internal/politics/${regionId}/parent`, { parentId: countryId }, internal);
  check('region attached to the country (system)', [linkRegion.status, linkRegion.json.parentId], [200, countryId]);

  const wrongLevel = await req('PUT', `/api/internal/politics/${fedId}/parent`, { parentId: regionId }, internal);
  check('parent of a lower level refused', [wrongLevel.status, wrongLevel.json.error], [400, 'INVALID_PARENT_LEVEL']);
  const self = await req('PUT', `/api/internal/politics/${fedId}/parent`, { parentId: fedId }, internal);
  check('self parent refused', [self.status, self.json.error], [400, 'INVALID_PARENT']);
  const unknownParent = await req('PUT', `/api/internal/politics/${regionId}/parent`, { parentId: UNKNOWN_HOLDER }, internal);
  check('unknown parent refused', unknownParent.status, 404);
  const noParentField = await req('PUT', `/api/internal/politics/${regionId}/parent`, {}, internal);
  check('parentId is required', [noParentField.status, noParentField.json.error], [400, 'VALIDATION_ERROR']);

  const byParent = await req('GET', `/api/internal/politics?parentId=${countryId}`, undefined, internal);
  check('lookup by parent', [byParent.status, byParent.json.total, byParent.json.items[0]?.id], [200, 1, regionId]);

  const children = await req('GET', `/api/internal/politics/${fedId}/children`, undefined, internal);
  check(
    'children of the federation',
    [children.status, children.json.total, children.json.items.map((e) => e.id)],
    [200, 1, [countryId]],
  );
  const noChildren = await req('GET', `/api/internal/politics/${UNKNOWN_HOLDER}/children`, undefined, internal);
  check('children of an unknown entity 404', noChildren.status, 404);

  const ancestors = await req('GET', `/api/internal/politics/${regionId}/ancestors`, undefined, internal);
  check(
    'ancestors from the direct parent up to the root',
    [ancestors.status, ancestors.json.map((e) => e.id)],
    [200, [countryId, fedId]],
  );
  const rootAncestors = await req('GET', `/api/internal/politics/${fedId}/ancestors`, undefined, internal);
  check('root has no ancestor', [rootAncestors.status, rootAncestors.json], [200, []]);
  const unknownAncestors = await req('GET', `/api/internal/politics/${UNKNOWN_HOLDER}/ancestors`, undefined, internal);
  check('ancestors of an unknown entity 404', unknownAncestors.status, 404);

  // The player path keeps its permission check: D is a plain citizen of the commune, A its head.
  const playerDenied = await req('PUT', `/api/politics/${entityId}/parent`, { parentId: regionId }, asPlayer(D, 'dave'));
  check('player without manage_hierarchy still refused', playerDenied.status, 403);
  const playerOk = await req('PUT', `/api/politics/${entityId}/parent`, { parentId: regionId }, asPlayer(A, 'alice'));
  check('head still attaches through the player route', [playerOk.status, playerOk.json.parentId], [200, regionId]);
  const deep = await req('GET', `/api/internal/politics/${entityId}/ancestors`, undefined, internal);
  check('commune ancestors span the whole chain', deep.json.map((e) => e.id), [regionId, countryId, fedId]);

  const journal = await req('GET', `/api/politics/${entityId}/activity`, undefined, asPlayer(A, 'alice'));
  check(
    'player link journaled with the acting head',
    [journal.status, journal.json.items[0]?.type, journal.json.items[0]?.actorId],
    [200, 'parent_set', A],
  );

  const detach = await req('PUT', `/api/internal/politics/${entityId}/parent`, { parentId: null }, internal);
  check('system detach', [detach.status, detach.json.parentId], [200, null]);
  const journalAfter = await req('GET', `/api/politics/${entityId}/activity`, undefined, asPlayer(A, 'alice'));
  check(
    'system detach journaled with a null actor',
    [journalAfter.json.items[0]?.type, journalAfter.json.items[0]?.actorId],
    ['parent_cleared', null],
  );

  const patchHeadless = await req('PATCH', `/api/internal/politics/${fedId}`, { description: 'x' }, internal);
  check('PATCH a headless entity answers NO_HEAD', [patchHeadless.status, patchHeadless.json.error], [409, 'NO_HEAD']);
  const transferHeadless = await req('POST', `/api/internal/politics/${fedId}/transfer`, { playerId: A }, internal);
  check('transfer on a headless entity answers NO_HEAD', [transferHeadless.status, transferHeadless.json.error], [409, 'NO_HEAD']);
  const playerDisband = await req('DELETE', `/api/politics/${fedId}`, undefined, asPlayer(A, 'alice'));
  check('a player cannot disband a headless entity', playerDisband.status, 403);

  const duplicate = await req('POST', '/api/internal/politics', { type: 'federation', name: fedName }, internal);
  check('duplicate name still conflicts', duplicate.status, 409);
}

// ── Validation and auth boundaries ───────────────────────────────────────────

console.log('\n# validation & auth');
{
  const badAction = await req(
    'POST',
    '/api/internal/authorize',
    { ...corpCheck(A, 'Manage Corporation!', corpId) },
    internal,
  );
  check('malformed action rejected', [badAction.status, badAction.json.error], [400, 'VALIDATION_ERROR']);

  const badHolder = await req(
    'POST',
    '/api/internal/authorize',
    { ...corpCheck(A, 'manage_corporation', corpId), holderType: 'guild' },
    internal,
  );
  check('unknown holder kind rejected', [badHolder.status, badHolder.json.error], [400, 'VALIDATION_ERROR']);

  const empty = await req('POST', '/api/internal/authorize', {}, internal);
  check('empty body rejected', [empty.status, empty.json.error], [400, 'VALIDATION_ERROR']);

  const unknown = await req(
    'POST',
    '/api/internal/authorize',
    corpCheck(A, 'manage_corporation', UNKNOWN_HOLDER),
    internal,
  );
  check('unknown holder refuses (fail-closed, never errors)', [unknown.status, unknown.allowed, unknown.reason], [
    200,
    false,
    'not_member',
  ]);

  const wrongKey = await req(
    'POST',
    '/api/internal/authorize',
    corpCheck(A, 'manage_corporation', corpId),
    { 'X-Internal-Key': 'wrong-key' },
  );
  check('authorize without credentials', [wrongKey.status, wrongKey.json.error], [401, 'UNAUTHORIZED']);

  const catalogNoAuth = await req('GET', '/api/internal/permissions/catalog', undefined, {
    'X-Internal-Key': 'wrong-key',
  });
  check('catalog without credentials', [catalogNoAuth.status, catalogNoAuth.json.error], [401, 'UNAUTHORIZED']);
}

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
