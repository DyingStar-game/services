/**
 * End-to-end smoke test for the Economie API.
 *
 * Exercises the money paths against a running server: player wallets, idempotency,
 * transfers with the automatic system tax, corporation donations with the internal tax,
 * the treasury report and the access rules. A few invariants that no route exposes
 * (system tax vault balance, total money supply) are checked with a direct SQL query.
 *
 *   node scripts/smoke.mjs
 *   BASE=http://host.docker.internal:3000 KEY=test-internal-key \
 *   DATABASE_URL=postgresql://user:password@host.docker.internal:5432/economie node scripts/smoke.mjs
 *
 * The treasury ACL is decided by Social (`POST /api/internal/authorize`), so the access
 * rules below need a running Social service (Economy must have SOCIAL_API_URL set):
 *   BASE=... KEY=... SOCIAL_BASE=http://host.docker.internal:3001 node scripts/smoke.mjs
 *
 * Against production-style auth (Keycloak service accounts), provide:
 *   KC_BASE=http://keycloak:8080 KC_CLIENT_ID=svc-game KC_CLIENT_SECRET=... \
 *   KC_MARKET_CLIENT_ID=svc-market KC_MARKET_CLIENT_SECRET=... node scripts/smoke.mjs
 * and drop KEY; without Keycloak the legacy X-Internal-Key is used (INTERNAL_DEV_BYPASS=true).
 *
 * Exits with code 1 as soon as one assertion failed.
 */
import pg from 'pg';

const BASE = process.env.BASE ?? 'http://localhost:3000';
const KEY = process.env.KEY ?? 'test-internal-key';
const DATABASE_URL = process.env.DATABASE_URL ?? '';
const TAX_VAULT = process.env.ECONOMY_TAX_VAULT_UUID ?? '00000000-0000-0000-0000-000000000001';
/** Central permission decisions live in Social; unset to skip the treasury ACL block. */
const SOCIAL_BASE = process.env.SOCIAL_BASE ?? '';
const SOCIAL_KEY = process.env.SOCIAL_KEY ?? KEY;

const KC_BASE = process.env.KC_BASE ?? '';
const KC_REALM = process.env.KC_REALM ?? 'dyingstar';
const KC_CLIENT_ID = process.env.KC_CLIENT_ID ?? '';
const KC_CLIENT_SECRET = process.env.KC_CLIENT_SECRET ?? '';
const KC_MARKET_CLIENT_ID = process.env.KC_MARKET_CLIENT_ID ?? '';
const KC_MARKET_CLIENT_SECRET = process.env.KC_MARKET_CLIENT_SECRET ?? '';

/** Test fixtures. `D` is not a member of the corporation, `E` is a treasurer. */
const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const C = '33333333-3333-4333-8333-333333333333';
const D = '44444444-4444-4444-8444-444444444444';
const E = '55555555-5555-4555-8555-555555555555';
/** Never funded, only used to observe lazy account creation. */
const F = '66666666-6666-4666-8666-666666666666';

const START_BALANCE = 10_000;
const IDEMPOTENT_CREDIT = 7;
/** Makes idempotency keys unique per run so the script can be replayed. */
const runId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
const asPlayer = (id, name = 'alice') => ({ 'X-Player-Id': id, 'X-Player-Name': name });
const socialAuth = { 'X-Internal-Key': SOCIAL_KEY };
/** Ticker for the Social fixture corporation (alphanumeric, <= 5 chars). */
const socialTicker = `E${runId.replace(/[^a-z0-9]/gi, '').slice(0, 4).toUpperCase()}`;

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

/** Balance of a player or corporation account, as seen by the internal API. */
async function balanceOf(holderType, id) {
  const path =
    holderType === 'player'
      ? `/api/internal/players/${id}/wallet`
      : `/api/internal/corporations/${id}/wallet`;
  const { json } = await req('GET', path, undefined, internal);
  const account = json.accounts?.find((a) => a.currency === 'credits');
  return account ? account.balance : 0;
}

const playerBalance = (id) => balanceOf('player', id);
const corpBalance = (id) => balanceOf('corporation', id);

/** Optional SQL client for the invariants no route exposes. */
let sql = null;
if (DATABASE_URL) {
  const client = new pg.Client({ connectionString: DATABASE_URL });
  try {
    await client.connect();
    sql = {
      vaultBalance: async () => {
        const r = await client.query(
          'select coalesce(sum(balance), 0)::text as total from accounts where holder_type = $1 and holder_id = $2 and currency = $3',
          ['system', TAX_VAULT, 'credits'],
        );
        return Number(r.rows[0].total);
      },
      totalSupply: async () => {
        const r = await client.query('select coalesce(sum(balance), 0)::text as total from accounts');
        return Number(r.rows[0].total);
      },
      close: () => client.end(),
    };
  } catch (err) {
    console.log(`  warn SQL assertions disabled: ${err.message}`);
  }
} else {
  console.log('  note DATABASE_URL unset: SQL invariants (system vault, money supply) skipped');
}

// ── Setup: fund the fixtures and position the corporation ────────────────────

console.log('\n# setup');
// Baseline: the database may already hold credits from earlier runs, so the money
// supply invariant is asserted as a delta rather than an absolute total.
const supplyBaseline = sql ? await sql.totalSupply() : 0;
let firstCredit = null;
for (const id of [A, B, D, E]) {
  const res = await req('POST', `/api/internal/players/${id}/wallet/credit`, { amount: START_BALANCE }, internal);
  check(`credit ${START_BALANCE} to ${id.slice(0, 4)}`, res.status, 201);
  if (id === A) firstCredit = res;
}
if (serviceTokenValue) {
  check('service caller recorded in the ledger', firstCredit.json.transaction.caller, KC_CLIENT_ID);
}
// The treasury ACL asks Social who may operate a corporation's treasury, so the fixture
// corporation must exist there (CEO = A) to be reachable at all. Without Social the local
// id is kept and every treasury check falls back to the dev bypass.
let CORP = C;
if (SOCIAL_BASE) {
  for (const [id, displayName] of [
    [A, 'Alice'],
    [B, 'Bob'],
    [D, 'Dave'],
  ]) {
    const profile = await req('PUT', `/api/internal/players/${id}`, { displayName }, socialAuth, SOCIAL_BASE);
    check(`social profile ensured for ${displayName}`, profile.status, 200);
  }
  const socialCorp = await req(
    'POST',
    '/api/internal/corporations',
    { ceoId: A, name: `Acl ${runId}`, ticker: socialTicker, recruitment: 'open' },
    socialAuth,
    SOCIAL_BASE,
  );
  check('social corporation created (CEO = A)', [socialCorp.status, socialCorp.json.ceoId], [201, A]);
  CORP = socialCorp.json.id;
} else {
  console.log('  note SOCIAL_BASE unset: the treasury ACL falls back to the dev bypass');
}

await req('PUT', `/api/internal/corporations/${CORP}/members/${A}`, { role: 'member' }, internal);
await req('PUT', `/api/internal/corporations/${CORP}/members/${E}`, { role: 'treasurer' }, internal);
const settings = await req('PUT', `/api/internal/corporations/${CORP}/settings`, { taxRateBps: 500, allowDonations: true }, internal);
check('corporation settings: taxRateBps 500, donations allowed', [settings.json.taxRateBps, settings.json.allowDonations], [500, true]);

// ── Health and authentication ────────────────────────────────────────────────

console.log('\n# health and auth');
{
  const { status, json } = await req('GET', '/api/health');
  check('GET /api/health', [status, json.status], [200, 'ok']);

  const noAuth = await req('GET', '/api/me/wallet');
  check('wallet without credentials', [noAuth.status, noAuth.json.error], [401, 'UNAUTHORIZED']);
  const noAuthFr = await req('GET', '/api/me/wallet', undefined, { 'Accept-Language': 'fr' });
  check('error message in French', [noAuthFr.status, noAuthFr.json.message], [401, 'Token Bearer manquant']);

  const noCreds = await req('GET', `/api/internal/players/${A}/wallet`);
  check('internal route without credentials', [noCreds.status, noCreds.json.error], [401, 'UNAUTHORIZED']);

  if (serviceTokenValue) {
    const bogus = await req('GET', `/api/internal/players/${A}/wallet`, undefined, {
      Authorization: 'Bearer not-a-jwt',
    });
    check('internal route with a bogus bearer', [bogus.status, bogus.json.error], [401, 'UNAUTHORIZED']);

    const other = await req('GET', `/api/internal/players/${A}/wallet`, undefined, {
      Authorization: `Bearer ${serviceTokenValue}.tampered`,
    });
    check('internal route with a tampered bearer', [other.status, other.json.error], [401, 'UNAUTHORIZED']);

    if (KC_MARKET_CLIENT_ID && KC_MARKET_CLIENT_SECRET) {
      const marketToken = await serviceToken(KC_MARKET_CLIENT_ID, KC_MARKET_CLIENT_SECRET);
      const allowed = await req('GET', `/api/internal/players/${A}/wallet`, undefined, {
        Authorization: `Bearer ${marketToken}`,
      });
      check('svc-market can read wallets (has the role)', allowed.status, 200);

      const denied = await req('PUT', `/api/internal/corporations/${CORP}/settings`, { taxRateBps: 500 }, {
        Authorization: `Bearer ${marketToken}`,
      });
      check('svc-market cannot manage corporations (missing role)', [denied.status, denied.json.error], [403, 'FORBIDDEN']);
    } else {
      console.log('  skip capability-role split (set KC_MARKET_CLIENT_ID/KC_MARKET_CLIENT_SECRET)');
    }
  } else {
    const wrongKey = await req('GET', `/api/internal/players/${A}/wallet`, undefined, { 'X-Internal-Key': 'wrong' });
    check('internal route with wrong X-Internal-Key', [wrongKey.status, wrongKey.json.error], [401, 'UNAUTHORIZED']);
  }
}

// ── Wallets, idempotency, overdraft ──────────────────────────────────────────

console.log('\n# wallets');
{
  const { status, json } = await req('PUT', `/api/internal/players/${F}/wallet`, undefined, internal);
  check('PUT wallet creates the credits account', [status, json.currency, json.balance, json.status], [200, 'credits', 0, 'active']);

  // Run-scoped key: the first call must succeed even on a re-run, the replay must not.
  const key = `smoke-idem-${runId}`;
  const before = await playerBalance(B);
  const first = await req('POST', `/api/internal/players/${B}/wallet/credit`, { amount: IDEMPOTENT_CREDIT, externalId: key }, internal);
  const replay = await req('POST', `/api/internal/players/${B}/wallet/credit`, { amount: IDEMPOTENT_CREDIT, externalId: key }, internal);
  check('credit with externalId', first.status, 201);
  check('replaying the same externalId', [replay.status, replay.json.error], [409, 'DUPLICATE_EXTERNAL_ID']);
  check('replay did not credit twice', await playerBalance(B), before + IDEMPOTENT_CREDIT);

  // Balances accumulate across runs, so the overdraft is computed from the live one.
  const dBefore = await playerBalance(D);
  const overdraft = await req('POST', `/api/internal/players/${D}/wallet/debit`, { amount: dBefore + 1 }, internal);
  check('debit beyond the balance', [overdraft.status, overdraft.json.error], [409, 'INSUFFICIENT_FUNDS']);
  check('balance untouched after the refused debit', await playerBalance(D), dBefore);
}

// ── Player transfer: the tax goes to the system vault ────────────────────────

console.log('\n# transfer (tax -> system vault)');
const transferTax = 5;
{
  const aBefore = await playerBalance(A);
  const bBefore = await playerBalance(B);
  const vaultBefore = sql ? await sql.vaultBalance() : null;

  const { status, json } = await req('POST', '/api/transfers', { toPlayerId: B, amount: 100 }, asPlayer(A));
  check('transfer accepted', status, 201);
  check('transfer tax is 5%', [json.amount, json.taxAmount], [100, transferTax]);
  check('payer debited amount + tax', await playerBalance(A), aBefore - 100 - transferTax);
  check('receiver credited the amount', await playerBalance(B), bBefore + 100);

  if (sql) {
    check('system vault credited the tax', await sql.vaultBalance(), vaultBefore + transferTax);
  } else {
    console.log('  skip system vault balance (needs DATABASE_URL)');
  }
}

// ── Donation: the tax stays inside the corporation ───────────────────────────

console.log('\n# donation (tax stays in the corporation)');
{
  // Scoped with `from` so the assertion holds on a database with older donations.
  const from = new Date(Date.now() - 1000).toISOString();
  const aBefore = await playerBalance(A);
  const cBefore = await corpBalance(CORP);

  const { status, json } = await req('POST', `/api/corporations/${CORP}/donations`, { amount: 1000, memo: 'smoke' }, asPlayer(A));
  check('donation accepted', status, 201);
  check('donation tax is 5%', [json.amount, json.taxAmount], [1000, 50]);
  check('donor debited amount + tax', await playerBalance(A), aBefore - 1050);
  check('treasury credited amount + tax', await corpBalance(CORP), cBefore + 1050);
  check('reported receiver balance is truthful', json.toBalance, cBefore + 1050);
  check('tax destination recorded in details', json.transaction.details.taxTo, json.transaction.toAccountId);

  const report = await req('GET', `/api/corporations/${CORP}/report?from=${from}`, undefined, asPlayer(A));
  const donationRow = report.json.byType?.find((r) => r.type === 'donation');
  check('report counts the retained tax as inflow', donationRow?.inflow, 1050);
  check('report total matches the treasury inflow', report.json.totals?.[0]?.inflow, 1050);
}

// ── Donation tax boundaries ──────────────────────────────────────────────────

console.log('\n# donation tax boundaries');
{
  await req('PUT', `/api/internal/corporations/${CORP}/settings`, { taxRateBps: 0 }, internal);
  const cBefore = await corpBalance(CORP);
  const { json } = await req('POST', `/api/corporations/${CORP}/donations`, { amount: 1000 }, asPlayer(A));
  check('no internal tax', json.taxAmount, 0);
  check('treasury credited exactly the amount', await corpBalance(CORP), cBefore + 1000);

  await req('PUT', `/api/internal/corporations/${CORP}/settings`, { taxRateBps: 10000 }, internal);
  const cBefore2 = await corpBalance(CORP);
  const full = await req('POST', `/api/corporations/${CORP}/donations`, { amount: 1000 }, asPlayer(A));
  check('100% internal tax doubles the treasury', [full.json.taxAmount, await corpBalance(CORP)], [1000, cBefore2 + 2000]);

  await req('PUT', `/api/internal/corporations/${CORP}/settings`, { taxRateBps: 500 }, internal);
}

// ── Donation policy and membership ───────────────────────────────────────────

console.log('\n# donation access rules');
{
  await req('PUT', `/api/internal/corporations/${CORP}/settings`, { allowDonations: false }, internal);
  const disabled = await req('POST', `/api/corporations/${CORP}/donations`, { amount: 100 }, asPlayer(A));
  check('donations disabled', [disabled.status, disabled.json.error], [403, 'DONATIONS_DISABLED']);

  const outsider = await req('POST', `/api/corporations/${CORP}/donations`, { amount: 100 }, asPlayer(D, 'dave'));
  check('donation by a non-member', [outsider.status, outsider.json.error], [403, 'NOT_CORPORATION_MEMBER']);

  await req('PUT', `/api/internal/corporations/${CORP}/settings`, { allowDonations: true }, internal);
}

// ── Treasury ACL: `economie:treasury:manage` is decided by Social ───────────

console.log('\n# treasury ACL (central permission decision in Social)');
if (SOCIAL_BASE) {
  const report = (playerId, name) => req('GET', `/api/corporations/${CORP}/report`, undefined, asPlayer(playerId, name));

  // A is the CEO of the Social corporation: the leader holds everything implicitly.
  const ceoReport = await report(A, 'alice');
  check('CEO reads the treasury report', ceoReport.status, 200);

  // A plain member holds no permission: refused with the local error code.
  const join = await req('POST', `/api/corporations/${CORP}/join`, {}, asPlayer(B, 'bob'), SOCIAL_BASE);
  check('B joined the social corporation (default rank)', [join.status, join.json.joined], [200, true]);
  const memberReport = await report(B, 'bob');
  check('report refused to a plain member', [memberReport.status, memberReport.json.error], [403, 'FORBIDDEN']);

  // The free action can be granted on a rank, which is the only way in for a non-leader.
  const rank = await req(
    'POST',
    `/api/corporations/${CORP}/ranks`,
    { name: 'Treasurer', priority: 40, permissions: ['economie:treasury:manage'] },
    asPlayer(A, 'alice'),
    SOCIAL_BASE,
  );
  check('rank granted economie:treasury:manage', [rank.status, rank.json.permissions], [201, ['economie:treasury:manage']]);
  const seated = await req(
    'PATCH',
    `/api/corporations/${CORP}/members/${B}`,
    { rankId: rank.json.id },
    asPlayer(A, 'alice'),
    SOCIAL_BASE,
  );
  check('B seated in the treasurer rank', seated.status, 200);
  const grantedReport = await report(B, 'bob');
  check('granted member reads the treasury report', grantedReport.status, 200);

  // The local economie role does not grant anything by itself.
  const localTreasurer = await report(E, 'erin');
  check('local treasurer without a Social grant is refused', [localTreasurer.status, localTreasurer.json.error], [
    403,
    'NOT_CORPORATION_MEMBER',
  ]);
  const outsiderReport = await report(D, 'dave');
  check('non-member refused the report', [outsiderReport.status, outsiderReport.json.error], [
    403,
    'NOT_CORPORATION_MEMBER',
  ]);
} else {
  console.log('  skip treasury ACL checks (set SOCIAL_BASE, Economy needs SOCIAL_API_URL)');
}

// ── Validation ───────────────────────────────────────────────────────────────

console.log('\n# validation');
{
  const badTax = await req('PUT', `/api/internal/corporations/${CORP}/settings`, { taxRateBps: 10001 }, internal);
  check('taxRateBps above 10000', [badTax.status, badTax.json.error], [400, 'VALIDATION_ERROR']);

  const zeroDonation = await req('POST', `/api/corporations/${CORP}/donations`, { amount: 0 }, asPlayer(A));
  check('donation of zero', [zeroDonation.status, zeroDonation.json.error], [400, 'VALIDATION_ERROR']);

  const selfTransfer = await req('POST', '/api/transfers', { toPlayerId: A, amount: 10 }, asPlayer(A));
  check('transfer to oneself', [selfTransfer.status, selfTransfer.json.error], [400, 'INVALID_TARGET']);

  const badTarget = await req('GET', '/api/corporations/not-a-uuid/wallet', undefined, asPlayer(A));
  check('non-uuid corporation id', [badTarget.status, badTarget.json.error], [400, 'VALIDATION_ERROR']);
}

// ── Pagination ──────────────────────────────────────────────────────────────

console.log('\n# pagination');
{
  const ledger = await req('GET', '/api/me/wallet/transactions?limit=5', undefined, asPlayer(A));
  check('ledger is a page', [ledger.status, typeof ledger.json.total, typeof ledger.json.limit, typeof ledger.json.offset], [200, 'number', 'number', 'number']);

  const members = await req('GET', `/api/corporations/${CORP}/members`, undefined, asPlayer(A));
  check('members list is a page', [members.status, members.json.items.length, typeof members.json.total], [200, 2, 'number']);

  const taxes = await req('GET', '/api/me/taxes', undefined, asPlayer(A));
  check('tax debts are a page', [taxes.status, typeof taxes.json.total, typeof taxes.json.limit, typeof taxes.json.offset], [200, 'number', 'number', 'number']);

  // Treasury reads go through Social's ACL (or the dev bypass): only assert the shape
  // when the call is allowed, so a strict environment skips it instead of failing.
  const salaries = await req('GET', `/api/corporations/${CORP}/salaries`, undefined, asPlayer(A));
  if (salaries.status === 200) {
    check('salary overrides carry page bounds', [typeof salaries.json.memberOverridesTotal, typeof salaries.json.limit, typeof salaries.json.offset], ['number', 'number', 'number']);
  } else {
    console.log(`  note salaries page skipped (treasury ACL returned ${salaries.status})`);
  }
}

// ── Money supply invariant ───────────────────────────────────────────────────

console.log('\n# money supply');
if (sql) {
  // Transfers and donations only move credits around: the payer's debit is fully
  // credited to the receiver and to the tax destination. So the total supply may only
  // grow by what the internal API injected from outside — a destroyed tax (donation)
  // or a minted one would break this.
  const injected = START_BALANCE * 4 + IDEMPOTENT_CREDIT;
  check('total supply equals the externally injected credits', await sql.totalSupply(), supplyBaseline + injected);
  console.log('  info system vault holds the transfer tax:', await sql.vaultBalance());
} else {
  console.log('  skip money supply invariant (needs DATABASE_URL)');
}

if (sql) await sql.close();

console.log(`\n${failures.length === 0 ? 'PASS' : 'FAIL'} — ${passed} passed, ${failures.length} failed`);
if (failures.length) {
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
