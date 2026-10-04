/**
 * Smoke test for the Market API (market-only behaviours).
 *
 * Exercises the catalog push, order placement/cancellation, demands and validation. Matching
 * and settlement additionally require the Inventory and Economy services to be reachable and
 * configured (INVENTORY_API_URL / ECONOMY_API_URL); those paths are noted but not asserted here.
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
const runId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
const GOOD = `ore-${runId}`;
const internal = { 'X-Internal-Key': KEY };
const asPlayer = (id, name = 'alice') => ({ 'X-Player-Id': id, 'X-Player-Name': name });

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

console.log(`\n# market smoke (run ${runId})`);

// ── Health & auth ─────────────────────────────────────────────────────────────
console.log('\n# health and auth');
{
  const health = await req('GET', '/api/health');
  check('GET /api/health', [health.status, health.json.service], [200, 'market']);

  const noAuth = await req('GET', '/api/internal/catalog');
  check('internal route without credentials', [noAuth.status, noAuth.json.error], [401, 'UNAUTHORIZED']);

  const noPlayer = await req('GET', '/api/market/orders');
  check('player route without credentials', [noPlayer.status, noPlayer.json.error], [401, 'UNAUTHORIZED']);
  const noPlayerFr = await req('GET', '/api/market/orders', undefined, { 'Accept-Language': 'fr' });
  check('error message in French', [noPlayerFr.status, noPlayerFr.json.message], [401, 'Token Bearer manquant']);
}

// ── Catalog ───────────────────────────────────────────────────────────────────
console.log('\n# catalog');
{
  const put = await req('PUT', `/api/internal/catalog/${GOOD}`, { kind: 'stack', unit: 't', displayName: 'Ore' }, internal);
  check('upsert catalog entry', [put.status, put.json.goodType, put.json.kind, put.json.enabled], [200, GOOD, 'stack', true]);

  const list = await req('GET', '/api/internal/catalog', undefined, internal);
  check('catalog contains the entry', list.json.some((e) => e.goodType === GOOD), true);

  const badKind = await req('PUT', `/api/internal/catalog/${GOOD}`, { kind: 'nonsense' }, internal);
  check('invalid kind rejected', [badKind.status, badKind.json.error], [400, 'VALIDATION_ERROR']);
}

// ── Orders ────────────────────────────────────────────────────────────────────
console.log('\n# orders');
{
  // A sell order with no counterparty stays open (no settlement attempted).
  const sell = await req(
    'POST',
    '/api/market/orders',
    { side: 'sell', goodType: GOOD, kind: 'stack', quantity: 100, price: 10 },
    asPlayer(A),
  );
  check('sell order placed', [sell.status, sell.json.order.side, sell.json.order.status, sell.json.order.remaining], [201, 'sell', 'open', 100]);
  check('no match without counterparty', sell.json.trades.length, 0);

  const orderId = sell.json.order.id;
  const got = await req('GET', `/api/market/orders/${orderId}`, undefined, asPlayer(B));
  check('order is publicly readable', [got.status, got.json.id], [200, orderId]);

  const book = await req('GET', `/api/market/book?goodType=${GOOD}`, undefined, asPlayer(B));
  check('book depth counts the sell', [book.json.buy, book.json.sell], [0, 1]);

  const listed = await req('GET', `/api/market/orders?goodType=${GOOD}`, undefined, asPlayer(B));
  check('order appears in the book', listed.json.length, 1);

  const cancelByOther = await req('POST', `/api/market/orders/${orderId}/cancel`, {}, asPlayer(B));
  check('cannot cancel another player order', [cancelByOther.status, cancelByOther.json.error], [403, 'FORBIDDEN']);

  const cancel = await req('POST', `/api/market/orders/${orderId}/cancel`, {}, asPlayer(A));
  check('owner cancels the order', [cancel.status, cancel.json.status], [200, 'cancelled']);
}

// ── Demands ───────────────────────────────────────────────────────────────────
console.log('\n# demands');
{
  const demand = await req(
    'POST',
    '/api/market/demands',
    { goodType: GOOD, kind: 'stack', quantity: 50, maxPrice: 12, message: 'need ore' },
    asPlayer(B),
  );
  check('demand created', [demand.status, demand.json.status, demand.json.maxPrice], [201, 'open', 12]);

  const demandId = demand.json.id;
  const listed = await req('GET', `/api/market/demands?goodType=${GOOD}`, undefined, asPlayer(A));
  check('demand listed', listed.json.some((d) => d.id === demandId), true);

  const cancelByOther = await req('POST', `/api/market/demands/${demandId}/cancel`, {}, asPlayer(A));
  check('cannot cancel another player demand', [cancelByOther.status, cancelByOther.json.error], [403, 'FORBIDDEN']);

  const cancel = await req('POST', `/api/market/demands/${demandId}/cancel`, {}, asPlayer(B));
  check('owner cancels the demand', [cancel.status, cancel.json.status], [200, 'cancelled']);
}

// ── Validation ────────────────────────────────────────────────────────────────
console.log('\n# validation');
{
  const buyInstance = await req(
    'POST',
    '/api/market/orders',
    { side: 'buy', goodType: GOOD, kind: 'instance', quantity: 1, price: 100 },
    asPlayer(A),
  );
  check('buy instance order rejected', [buyInstance.status, buyInstance.json.error], [400, 'VALIDATION_ERROR']);

  const instanceNoId = await req(
    'POST',
    '/api/market/orders',
    { side: 'sell', goodType: GOOD, kind: 'instance', quantity: 1, price: 100 },
    asPlayer(A),
  );
  check('instance order without instanceId rejected', [instanceNoId.status, instanceNoId.json.error], [400, 'VALIDATION_ERROR']);

  const unknownGood = await req(
    'POST',
    '/api/market/orders',
    { side: 'sell', goodType: `nope-${runId}`, kind: 'stack', quantity: 1, price: 1 },
    asPlayer(A),
  );
  check('order on unknown good rejected', [unknownGood.status, unknownGood.json.error], [404, 'NOT_FOUND']);

  const zeroQty = await req(
    'POST',
    '/api/market/orders',
    { side: 'sell', goodType: GOOD, kind: 'stack', quantity: 0, price: 1 },
    asPlayer(A),
  );
  check('non-positive quantity', [zeroQty.status, zeroQty.json.error], [400, 'VALIDATION_ERROR']);

  const badId = await req('GET', '/api/market/orders/not-a-uuid', undefined, asPlayer(A));
  check('non-uuid order id', [badId.status, badId.json.error], [400, 'VALIDATION_ERROR']);
}

// ── Matching & settlement (best-effort: needs inventory + economy) ────────────
console.log('\n# matching and settlement');
{
  const sell = await req(
    'POST',
    '/api/market/orders',
    { side: 'sell', goodType: GOOD, kind: 'stack', quantity: 10, price: 5 },
    asPlayer(A),
  );
  const buy = await req(
    'POST',
    '/api/market/orders',
    { side: 'buy', goodType: GOOD, kind: 'stack', quantity: 10, price: 5 },
    asPlayer(B),
  );
  check('buy order crosses the sell', buy.json.trades?.length, 1);
  if (buy.json.trades?.length) {
    const trade = buy.json.trades[0];
    check('trade price is the resting price', [trade.unitPrice, trade.totalPrice, trade.quantity], [5, 50, 10]);
    if (trade.status === 'settled') {
      check('trade settled (inventory + economy reachable)', trade.status, 'settled');
    } else {
      console.log(`  note trade is "${trade.status}" (failure=${trade.failureReason}); inventory/economy may be unconfigured`);
    }
  }
}

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
