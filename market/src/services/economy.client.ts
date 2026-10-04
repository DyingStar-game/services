/**
 * Outbound client for the Economy service internal API. The market uses it to move credits
 * at settlement. Authentication mirrors the other services: market's own Keycloak service
 * account (`client_credentials`) with a dev-only `X-Internal-Key` fallback.
 */
import { env } from '../config/env.js';
import { currentLang } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

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
    throw new HttpError(502, 'ECONOMY_AUTH_FAILED', `Economy token request failed (${res.status}): ${text}`, { status: res.status, body: text });
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

/** Whether the Economy integration is configured. */
export function isEconomyConfigured(): boolean {
  return Boolean(env.economy.apiUrl && (env.economy.serviceClientSecret || env.economy.internalApiKey));
}

/** Movement parameters accepted by the Economy internal credit/debit routes. */
export interface PlayerMovement {
  amount: number;
  currency?: string;
  reference?: string;
  externalId: string;
}

/** Holder kinds whose wallets the economy service exposes on its internal API. */
export type SettleableHolderType = 'player' | 'npc' | 'corporation';

/** Shared credit/debit call to the Economy internal API (returns null on idempotent replay). */
async function holderMovement(
  direction: 'credit' | 'debit',
  holderType: SettleableHolderType,
  holderId: string,
  opts: PlayerMovement,
): Promise<unknown | null> {
  if (!env.economy.apiUrl) {
    throw new HttpError(503, 'ECONOMY_NOT_CONFIGURED', 'ECONOMY_API_URL is not configured');
  }
  const segment = holderType === 'player' ? 'players' : holderType === 'npc' ? 'npcs' : 'corporations';
  const base = `/api/internal/${segment}/${holderId}`;
  const res = await fetch(`${env.economy.apiUrl}${base}/wallet/${direction}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'Accept-Language': currentLang(), ...(await authHeaders()) },
    body: JSON.stringify({
      amount: opts.amount,
      currency: opts.currency,
      reference: opts.reference,
      externalId: opts.externalId,
      type: direction === 'credit' ? 'transfer' : 'withdrawal',
    }),
  });
  if (res.status === 201) return res.json();

  const text = await res.text().catch(() => '');
  let parsed: { error?: string; message?: string } | null = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = null;
  }
  if (res.status === 409 && parsed?.error === 'DUPLICATE_EXTERNAL_ID') return null;
  if (res.status === 409 && parsed?.error === 'INSUFFICIENT_FUNDS') {
    throw new HttpError(409, 'INSUFFICIENT_FUNDS', parsed.message ?? 'Insufficient balance');
  }
  const code = direction === 'credit' ? 'ECONOMY_CREDIT_FAILED' : 'ECONOMY_DEBIT_FAILED';
  throw new HttpError(502, code, `Economy ${direction} failed (${res.status}): ${parsed?.message ?? text}`);
}

/** Debits a holder's wallet (buyer pays for a trade). */
export function debitHolder(
  holderType: SettleableHolderType,
  holderId: string,
  opts: PlayerMovement,
): Promise<unknown | null> {
  return holderMovement('debit', holderType, holderId, opts);
}

/** Credits a holder's wallet (seller gets paid for a trade). */
export function creditHolder(
  holderType: SettleableHolderType,
  holderId: string,
  opts: PlayerMovement,
): Promise<unknown | null> {
  return holderMovement('credit', holderType, holderId, opts);
}
