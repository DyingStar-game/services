/**
 * Outbound client for the Economy service internal API. Used to credit economic mission
 * rewards. Authentication prefers the mission's own Keycloak service account
 * (`client_credentials`, `svc-mission`); in dev, when no client secret is configured, the
 * legacy `X-Internal-Key` is sent instead (requires Economy's `INTERNAL_DEV_BYPASS=true`).
 */
import { env } from '../config/env.js';
import { HttpError } from '../lib/httpError.js';

/** Result of an Economy credit call. */
export interface EconomyMovement {
  transaction: { id: number; externalId?: string | null; [key: string]: unknown };
  amount: number;
  toBalance?: number;
}

interface TokenCache {
  value: string;
  expiresAt: number;
}

let tokenCache: TokenCache | null = null;

/** Fetches (and caches) a service-account access token for the configured client. */
async function getServiceToken(): Promise<string> {
  const now = Date.now();
  if (tokenCache && tokenCache.expiresAt > now) return tokenCache.value;

  const res = await fetch(env.oidc.tokenUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'client_credentials',
      client_id: env.economy.serviceClientId,
      client_secret: env.economy.serviceClientSecret,
    }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'ECONOMY_AUTH_FAILED', `Economy token request failed (${res.status}): ${text}`);
  }
  const json = (await res.json()) as { access_token: string; expires_in?: number };
  const ttl = Math.max(30, (json.expires_in ?? 60) - 30);
  tokenCache = { value: json.access_token, expiresAt: now + ttl * 1000 };
  return json.access_token;
}

/** Headers used to authenticate against the Economy internal API. */
async function authHeaders(): Promise<Record<string, string>> {
  if (env.economy.serviceClientSecret) {
    return { Authorization: `Bearer ${await getServiceToken()}` };
  }
  if (env.economy.internalApiKey) {
    return { 'X-Internal-Key': env.economy.internalApiKey };
  }
  throw new HttpError(
    503,
    'ECONOMY_NOT_CONFIGURED',
    'No Economy credentials configured (set ECONOMY_SERVICE_CLIENT_SECRET or ECONOMY_INTERNAL_API_KEY)',
  );
}

/** Movement parameters accepted by the Economy internal credit/debit routes. */
export interface PlayerMovement {
  amount: number;
  currency?: string;
  reference?: string;
  externalId: string;
}

/** Shared credit/debit call to the Economy internal API. */
async function playerMovement(
  direction: 'credit' | 'debit',
  playerId: string,
  opts: PlayerMovement,
): Promise<EconomyMovement | null> {
  const res = await fetch(`${env.economy.apiUrl}/api/internal/players/${playerId}/wallet/${direction}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...(await authHeaders()) },
    body: JSON.stringify({
      amount: opts.amount,
      currency: opts.currency,
      reference: opts.reference,
      externalId: opts.externalId,
      type: direction === 'credit' ? 'mission_reward' : 'withdrawal',
    }),
  });

  if (res.status === 201) return (await res.json()) as EconomyMovement;

  const text = await res.text().catch(() => '');
  let parsed: { error?: string; message?: string } | null = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = null;
  }
  // The idempotency key was already recorded: the movement is considered done.
  if (res.status === 409 && parsed?.error === 'DUPLICATE_EXTERNAL_ID') return null;

  const code = direction === 'credit' ? 'ECONOMY_CREDIT_FAILED' : 'ECONOMY_DEBIT_FAILED';
  // Insufficient funds is a caller error, not an upstream failure.
  if (res.status === 409 && parsed?.error === 'INSUFFICIENT_FUNDS') {
    throw new HttpError(409, 'INSUFFICIENT_FUNDS', parsed.message ?? 'Insufficient balance for the mission escrow');
  }
  throw new HttpError(502, code, `Economy ${direction} failed (${res.status}): ${parsed?.message ?? text}`);
}

/**
 * Credits a player wallet through the Economy internal API.
 * @param playerId - Player id.
 * @param opts - Amount, currency, reference and idempotency key.
 * @returns The Economy movement, or `null` when it was already recorded (idempotent replay).
 */
export function creditPlayer(playerId: string, opts: PlayerMovement): Promise<EconomyMovement | null> {
  return playerMovement('credit', playerId, opts);
}

/**
 * Debits a player wallet through the Economy internal API (mission escrow).
 * @param playerId - Player id.
 * @param opts - Amount, currency, reference and idempotency key.
 * @returns The Economy movement, or `null` when already recorded.
 * @throws 409 `INSUFFICIENT_FUNDS` when the player cannot fund the escrow.
 */
export function debitPlayer(playerId: string, opts: PlayerMovement): Promise<EconomyMovement | null> {
  return playerMovement('debit', playerId, opts);
}
