/**
 * End-to-end smoke test for the Inventory API.
 *
 * Exercises the ownership registry: fungible stacks (credit, hold, consume, transfer,
 * debit), unique instances (register, hold, consume), the hold availability invariant and
 * the access rules.
 *
 *   node scripts/smoke.mjs
 *   BASE=http://host.docker.internal:3000 KEY=test-internal-key node scripts/smoke.mjs
 *
 * Exits with code 1 as soon as one assertion failed.
 */

const BASE = process.env.BASE ?? 'http://localhost:3000';
const KEY = process.env.KEY ?? 'test-internal-key';

const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const INSTANCE = '99999999-9999-4999-8999-999999999999';
const runId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
const GOOD = `ore-${runId}`;
const TRUCK = `truck-${runId}`;
const asPlayer = (id, name = 'alice') => ({ 'X-Player-Id': id, 'X-Player-Name': name });
const internal = { 'X-Internal-Key': KEY };

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

async function req(method, path, body, headers = {}) {
  const res = await fetch(BASE + path, {
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

const holder = (type, id) => `/api/internal/holders/${type}/${id}`;

console.log(`\n# inventory smoke (run ${runId})`);

// ── Health & auth ─────────────────────────────────────────────────────────────
console.log('\n# health and auth');
{
  const health = await req('GET', '/api/health');
  check('GET /api/health', [health.status, health.json.service], [200, 'inventory']);

  const noAuth = await req('GET', holder('player', A));
  check('internal route without credentials', [noAuth.status, noAuth.json.error], [401, 'UNAUTHORIZED']);

  const wrong = await req('GET', holder('player', A), undefined, { 'X-Internal-Key': 'wrong' });
  check('internal route with wrong key', [wrong.status, wrong.json.error], [401, 'UNAUTHORIZED']);

  const me = await req('GET', '/api/me/inventory');
  check('player route without credentials', [me.status, me.json.error], [401, 'UNAUTHORIZED']);
  const meFr = await req('GET', '/api/me/inventory', undefined, { 'Accept-Language': 'fr' });
  check('error message in French', [meFr.status, meFr.json.message], [401, 'Token Bearer manquant']);
}

// ── Fungible stacks: credit, availability, hold, consume ─────────────────────
console.log('\n# fungible stacks');
{
  const c1 = await req('POST', `${holder('player', A)}/credit`, { goodType: GOOD, quantity: 1000 }, internal);
  check('credit 1000', [c1.status, c1.json.quantity], [201, 1000]);

  const c2 = await req('POST', `${holder('player', A)}/credit`, { goodType: GOOD, quantity: 500 }, internal);
  check('credit accumulates', c2.json.quantity, 1500);

  const held = await req(
    'POST',
    `${holder('player', A)}/holds`,
    { kind: 'stack', goodType: GOOD, quantity: 400, refType: 'market_order', refId: `order-${runId}` },
    internal,
  );
  check('create stack hold', [held.status, held.json.quantity, held.json.status], [201, 400, 'active']);

  const stack = await req('GET', `${holder('player', A)}/stacks/${GOOD}`, undefined, internal);
  check('held quantity is reserved', [stack.json.quantity, stack.json.held, stack.json.available], [1500, 400, 1100]);

  // Over-hold: only 1100 is available.
  const overHold = await req(
    'POST',
    `${holder('player', A)}/holds`,
    { kind: 'stack', goodType: GOOD, quantity: 1200, refType: 'manual' },
    internal,
  );
  check('over-hold rejected', [overHold.status, overHold.json.error], [409, 'INSUFFICIENT_GOODS']);

  // Debit cannot touch held quantity.
  const overDebit = await req('POST', `${holder('player', A)}/debit`, { goodType: GOOD, quantity: 1200 }, internal);
  check('debit beyond available rejected', [overDebit.status, overDebit.json.error], [409, 'INSUFFICIENT_GOODS']);

  // Consume the hold: 400 move to B.
  const consumed = await req('POST', `/api/internal/holds/${held.json.id}/consume`, { to: { holderType: 'player', holderId: B } }, internal);
  check('hold consumed', consumed.status, 200);

  const aStack = await req('GET', `${holder('player', A)}/stacks/${GOOD}`, undefined, internal);
  const bStack = await req('GET', `${holder('player', B)}/stacks/${GOOD}`, undefined, internal);
  check('sender debited the held quantity', [aStack.json.quantity, aStack.json.held], [1100, 0]);
  check('receiver credited the held quantity', bStack.json.quantity, 400);
}

// ── Direct transfer (no hold) ─────────────────────────────────────────────────
console.log('\n# direct transfer');
{
  const res = await req(
    'POST',
    '/api/internal/transfers',
    { from: { holderType: 'player', holderId: A }, to: { holderType: 'npc', holderId: B }, goodType: GOOD, quantity: 100 },
    internal,
  );
  check('transfer accepted', [res.status, res.json.quantity], [201, 100]);

  const aStack = await req('GET', `${holder('player', A)}/stacks/${GOOD}`, undefined, internal);
  check('source reduced', aStack.json.quantity, 1000);
}

// ── Unique instances: register, hold, consume ─────────────────────────────────
console.log('\n# unique instances');
{
  const reg = await req(
    'POST',
    `${holder('player', A)}/instances`,
    { instanceId: INSTANCE, goodType: TRUCK, metadata: { plate: 'DS-001' } },
    internal,
  );
  check('register instance', [reg.status, reg.json.id, reg.json.goodType, reg.json.status], [201, INSTANCE, TRUCK, 'stored']);

  const wrongKind = await req('POST', `${holder('player', A)}/credit`, { goodType: TRUCK, quantity: 1 }, internal);
  check('good type nature is stable', [wrongKind.status, wrongKind.json.error], [409, 'CONFLICT']);

  const hold = await req(
    'POST',
    `${holder('player', A)}/holds`,
    { kind: 'instance', goodType: TRUCK, instanceId: INSTANCE, refType: 'mission_escrow', refId: `mission-${runId}` },
    internal,
  );
  check('hold instance', [hold.status, hold.json.kind], [201, 'instance']);

  const doubleHold = await req(
    'POST',
    `${holder('player', A)}/holds`,
    { kind: 'instance', goodType: TRUCK, instanceId: INSTANCE, refType: 'manual' },
    internal,
  );
  check('instance double-hold rejected', [doubleHold.status, doubleHold.json.error], [409, 'CONFLICT']);

  const transferHeld = await req(
    'POST',
    '/api/internal/transfers/instance',
    { from: { holderType: 'player', holderId: A }, to: { holderType: 'player', holderId: B }, instanceId: INSTANCE },
    internal,
  );
  check('transfer of a held instance rejected', [transferHeld.status, transferHeld.json.error], [409, 'CONFLICT']);

  const consumed = await req(
    'POST',
    `/api/internal/holds/${hold.json.id}/consume`,
    { to: { holderType: 'player', holderId: B } },
    internal,
  );
  check('instance hold consumed', [consumed.status, consumed.json.instance.holderId], [200, B]);
}

// ── Access rules & validation ─────────────────────────────────────────────────
console.log('\n# access rules and validation');
{
  const me = await req('GET', '/api/me/inventory', undefined, asPlayer(A));
  check('player reads own inventory', me.status, 200);
  check('own instances are listed', me.json.instances.length, 0);

  const meB = await req('GET', '/api/me/inventory', undefined, asPlayer(B));
  check('recipient now owns the instance', meB.json.instances[0]?.id, INSTANCE);

  const badUuid = await req('GET', holder('player', 'not-a-uuid'), undefined, internal);
  check('non-uuid holder id', [badUuid.status, badUuid.json.error], [400, 'VALIDATION_ERROR']);

  const badType = await req('GET', holder('planet', A), undefined, internal);
  check('invalid holder type', [badType.status, badType.json.error], [400, 'VALIDATION_ERROR']);

  const negQty = await req('POST', `${holder('player', A)}/credit`, { goodType: GOOD, quantity: 0 }, internal);
  check('non-positive quantity', [negQty.status, negQty.json.error], [400, 'VALIDATION_ERROR']);
}

// ── POIs: CRUD, shares, transfer, resolution ─────────────────────────────────
console.log('\n# POIs');
const SYSTEM = '00000000-0000-4000-8000-000000000001';
{
  const created = await req(
    'POST',
    '/api/me/pois',
    { name: `Entrepôt ${runId}`, system: 'tarsis', scene: 'tarsis_1/new-paris', x: 10, y: 0, z: -5, radiusM: 250 },
    asPlayer(A),
  );
  check('create POI', [created.status, created.json.radiusM, created.json.visibility, created.json.ownerType], [201, 250, 'private', 'player']);
  const poiId = created.json.id;

  const listA = await req('GET', '/api/me/pois', undefined, asPlayer(A));
  check('owner lists the POI', listA.json.some((p) => p.id === poiId), true);
  const listB0 = await req('GET', '/api/me/pois', undefined, asPlayer(B));
  check('private POI hidden from others', listB0.json.some((p) => p.id === poiId), false);

  const share = await req('POST', `/api/me/pois/${poiId}/shares`, { granteeType: 'player', granteeId: B }, asPlayer(A));
  check('share with a player', share.status, 201);
  const dup = await req('POST', `/api/me/pois/${poiId}/shares`, { granteeType: 'player', granteeId: B }, asPlayer(A));
  check('duplicate share rejected', [dup.status, dup.json.error], [409, 'POI_ALREADY_SHARED']);

  const listB = await req('GET', '/api/me/pois', undefined, asPlayer(B));
  check('grantee lists the shared POI', listB.json.some((p) => p.id === poiId), true);
  const detailB = await req('GET', `/api/me/pois/${poiId}`, undefined, asPlayer(B));
  check('grantee reads detail with its grants', [detailB.status, detailB.json.shares?.length], [200, 1]);
  const editB = await req('PATCH', `/api/me/pois/${poiId}`, { name: 'pirate' }, asPlayer(B));
  check('grantee cannot edit (read-only share)', [editB.status, editB.json.error], [403, 'NOT_POI_OWNER']);

  const revoke = await req('DELETE', `/api/me/pois/${poiId}/shares/player/${B}`, undefined, asPlayer(A));
  check('revoke share', revoke.status, 204);
  const revokeAgain = await req('DELETE', `/api/me/pois/${poiId}/shares/player/${B}`, undefined, asPlayer(A));
  check('revoke twice rejected', [revokeAgain.status, revokeAgain.json.error], [404, 'POI_NOT_SHARED']);
  const listB2 = await req('GET', '/api/me/pois', undefined, asPlayer(B));
  check('revoked grantee no longer sees it', listB2.json.some((p) => p.id === poiId), false);

  const pub = await req(
    'POST',
    '/api/me/pois',
    { name: `Balise publique ${runId}`, x: 0, y: 0, z: 0, visibility: 'public' },
    asPlayer(A),
  );
  check('public POI created', [pub.status, pub.json.visibility], [201, 'public']);
  const listB3 = await req('GET', '/api/me/pois', undefined, asPlayer(B));
  check('public POI readable by anyone', listB3.json.some((p) => p.id === pub.json.id), true);

  const transferred = await req('POST', `/api/me/pois/${poiId}/transfer`, { toType: 'player', toId: B }, asPlayer(A));
  check('transfer ownership', [transferred.status, transferred.json.ownerType, transferred.json.ownerId], [200, 'player', B]);
  const oldOwnerEdit = await req('PATCH', `/api/me/pois/${poiId}`, { name: 'nope' }, asPlayer(A));
  check('previous owner lost control', [oldOwnerEdit.status, oldOwnerEdit.json.error], [403, 'NOT_POI_OWNER']);
  const selfTransfer = await req('POST', `/api/me/pois/${poiId}/transfer`, { toType: 'player', toId: B }, asPlayer(B));
  check('self transfer rejected', [selfTransfer.status, selfTransfer.json.error], [409, 'POI_TRANSFER_SELF']);
  const newOwnerEdit = await req('PATCH', `/api/me/pois/${poiId}`, { name: `Renommé ${runId}` }, asPlayer(B));
  check('new owner can edit', [newOwnerEdit.status, newOwnerEdit.json.name], [200, `Renommé ${runId}`]);

  // Internal API: system POI + batch resolution (the mission zone path).
  const sys = await req(
    'POST',
    '/api/internal/pois',
    { owner: { type: 'system', id: SYSTEM }, name: `Zone jeu ${runId}`, system: 'vega', x: 1, y: 2, z: 3, radiusM: 500 },
    internal,
  );
  check('system POI via internal API', [sys.status, sys.json.ownerType], [201, 'system']);

  const noAuth = await req('POST', '/api/internal/pois/resolve', { ids: [sys.json.id] });
  check('resolve without credentials', [noAuth.status, noAuth.json.error], [401, 'UNAUTHORIZED']);

  const resolved = await req(
    'POST',
    '/api/internal/pois/resolve',
    { ids: [sys.json.id, 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'] },
    internal,
  );
  check('batch resolve returns known POIs only', [resolved.status, resolved.json.pois.length], [200, 1]);
  check('resolved geometry', [resolved.json.pois[0].x, resolved.json.pois[0].radiusM], [1, 500]);

  const holderPois = await req('GET', `/api/internal/holders/system/${SYSTEM}/pois`, undefined, internal);
  check('POIs of a holder', holderPois.json.some((p) => p.id === sys.json.id), true);

  const missing = await req('GET', `/api/internal/pois/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa`, undefined, internal);
  check('unknown POI is 404', [missing.status, missing.json.error], [404, 'NOT_FOUND']);

  const noName = await req('POST', '/api/me/pois', { x: 1, y: 2, z: 3 }, asPlayer(A));
  check('POI requires a name', [noName.status, noName.json.error], [400, 'VALIDATION_ERROR']);
  const negRadius = await req('POST', '/api/me/pois', { name: 'bad', x: 1, y: 2, z: 3, radiusM: -1 }, asPlayer(A));
  check('negative radius rejected', [negRadius.status, negRadius.json.error], [400, 'VALIDATION_ERROR']);
  const emptyPatch = await req('PATCH', `/api/me/pois/${pub.json.id}`, {}, asPlayer(A));
  check('empty patch rejected', [emptyPatch.status, emptyPatch.json.error], [400, 'VALIDATION_ERROR']);

  const deleted = await req('DELETE', `/api/me/pois/${pub.json.id}`, undefined, asPlayer(A));
  check('delete POI', deleted.status, 204);
  const gone = await req('GET', `/api/me/pois/${pub.json.id}`, undefined, asPlayer(A));
  check('deleted POI is gone', [gone.status, gone.json.error], [404, 'NOT_FOUND']);
}

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
