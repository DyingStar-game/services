/**
 * Outbound client for the Economy service internal API. Used to credit economic mission
 * rewards. Authentication prefers the mission's own Keycloak service account
 * (`client_credentials`, `svc-mission`); in dev, when no client secret is configured, the
 * legacy `X-Internal-Key` is sent instead (requires Economy's `INTERNAL_DEV_BYPASS=true`).
 */
import { env } from '../config/env.js';
import { currentLang, t } from '../i18n/index.js';
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
    throw new HttpError(502, 'ECONOMY_AUTH_FAILED', `Economy token request failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
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

/** Holder kinds whose wallets Economy exposes on its internal API. */
export type WalletHolderType = 'player' | 'npc' | 'corporation';

/** Shared credit/debit call to the Economy internal API. */
async function holderMovement(
  direction: 'credit' | 'debit',
  holderType: WalletHolderType,
  holderId: string,
  opts: PlayerMovement,
): Promise<EconomyMovement | null> {
  const segment = holderType === 'player' ? 'players' : holderType === 'npc' ? 'npcs' : 'corporations';
  const res = await fetch(`${env.economy.apiUrl}/api/internal/${segment}/${holderId}/wallet/${direction}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'Accept-Language': currentLang(), ...(await authHeaders()) },
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
  const failure =
    direction === 'credit'
      ? t('economy.credit_failed', { status: res.status, message: parsed?.message ?? text })
      : t('economy.debit_failed', { status: res.status, message: parsed?.message ?? text });
  throw new HttpError(502, code, failure);
}

/**
 * Credits a wallet through the Economy internal API.
 * @param holderType - Player or NPC.
 * @param holderId - Holder id.
 * @param opts - Amount, currency, reference and idempotency key.
 * @returns The Economy movement, or `null` when it was already recorded (idempotent replay).
 */
export function creditHolder(
  holderType: WalletHolderType,
  holderId: string,
  opts: PlayerMovement,
): Promise<EconomyMovement | null> {
  return holderMovement('credit', holderType, holderId, opts);
}

/**
 * Debits a wallet through the Economy internal API (mission escrow).
 * @param holderType - Player or NPC.
 * @param holderId - Holder id.
 * @param opts - Amount, currency, reference and idempotency key.
 * @returns The Economy movement, or `null` when already recorded.
 * @throws 409 `INSUFFICIENT_FUNDS` when the holder cannot fund the escrow.
 */
export function debitHolder(
  holderType: WalletHolderType,
  holderId: string,
  opts: PlayerMovement,
): Promise<EconomyMovement | null> {
  return holderMovement('debit', holderType, holderId, opts);
}

/**
 * Credits a player wallet through the Economy internal API.
 * @param playerId - Player id.
 * @param opts - Amount, currency, reference and idempotency key.
 * @returns The Economy movement, or `null` when it was already recorded (idempotent replay).
 */
export function creditPlayer(playerId: string, opts: PlayerMovement): Promise<EconomyMovement | null> {
  return holderMovement('credit', 'player', playerId, opts);
}

/**
 * Debits a player wallet through the Economy internal API (mission escrow).
 * @param playerId - Player id.
 * @param opts - Amount, currency, reference and idempotency key.
 * @returns The Economy movement, or `null` when already recorded.
 * @throws 409 `INSUFFICIENT_FUNDS` when the player cannot fund the escrow.
 */
export function debitPlayer(playerId: string, opts: PlayerMovement): Promise<EconomyMovement | null> {
  return holderMovement('debit', 'player', playerId, opts);
}

/** One wallet account (currency + balance) as returned by the internal read API. */
export interface WalletAccount {
  currency: string;
  balance: number;
  status?: string;
  [key: string]: unknown;
}

/**
 * Reads a holder's wallet accounts (one per currency) through the Economy internal API.
 * Used by mission prerequisite checks (`has_credits`).
 * @param holderType - Player or NPC.
 * @param holderId - Holder id.
 * @returns The holder's accounts (empty when they have none).
 */
export async function getWalletAccounts(holderType: WalletHolderType, holderId: string): Promise<WalletAccount[]> {
  if (!env.economy.apiUrl) {
    throw new HttpError(503, 'ECONOMY_NOT_CONFIGURED', 'ECONOMY_API_URL is not configured');
  }
  const segment = holderType === 'player' ? 'players' : holderType === 'npc' ? 'npcs' : 'corporations';
  const res = await fetch(`${env.economy.apiUrl}/api/internal/${segment}/${holderId}/wallet`, {
    headers: { 'Accept-Language': currentLang(), ...(await authHeaders()) },
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'ECONOMY_READ_FAILED', `Economy wallet read failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
  }
  const json = (await res.json()) as { accounts?: WalletAccount[] };
  return json.accounts ?? [];
}
