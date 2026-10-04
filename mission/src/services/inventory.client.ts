/**
 * Outbound client for the Inventory service internal API. Used to escrow (hold) a
 * player-funded item reward at mission creation and to move ownership at settlement.
 * Authentication mirrors `economy.client`/`social.client`: mission's own Keycloak service
 * account (`client_credentials`) with a dev-only `X-Internal-Key` fallback.
 */
import { env } from '../config/env.js';
import { currentLang } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

/** Inventory holder reference. */
export interface Holder {
  holderType: 'player' | 'npc' | 'corporation' | 'system';
  holderId: string;
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
      client_id: env.inventory.serviceClientId,
      client_secret: env.inventory.serviceClientSecret,
    }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'INVENTORY_AUTH_FAILED', `Inventory token request failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
  }
  const json = (await res.json()) as { access_token: string; expires_in?: number };
  const ttl = Math.max(30, (json.expires_in ?? 60) - 30);
  tokenCache = { value: json.access_token, expiresAt: now + ttl * 1000 };
  return json.access_token;
}

/** Headers used to authenticate against the Inventory internal API. */
async function authHeaders(): Promise<Record<string, string>> {
  if (env.inventory.serviceClientSecret) {
    return { Authorization: `Bearer ${await getServiceToken()}` };
  }
  if (env.inventory.internalApiKey) {
    return { 'X-Internal-Key': env.inventory.internalApiKey };
  }
  throw new HttpError(
    503,
    'INVENTORY_NOT_CONFIGURED',
    'No Inventory credentials configured (set INVENTORY_SERVICE_CLIENT_SECRET or INVENTORY_INTERNAL_API_KEY)',
  );
}

/** Whether the Inventory integration is configured. */
export function isInventoryConfigured(): boolean {
  return Boolean(env.inventory.apiUrl && (env.inventory.serviceClientSecret || env.inventory.internalApiKey));
}

/** Generic JSON call to the Inventory internal API. */
async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  if (!env.inventory.apiUrl) {
    throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'INVENTORY_API_URL is not configured');
  }
  const res = await fetch(`${env.inventory.apiUrl}${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', 'Accept-Language': currentLang(), ...(await authHeaders()) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text().catch(() => '');
  const parsed = (() => {
    try {
      return text ? JSON.parse(text) : null;
    } catch {
      return null;
    }
  })();
  if (res.ok) return parsed as T;
  const error = parsed as { error?: string; message?: string } | null;
  // Insufficient goods is a caller-visible conflict.
  if (res.status === 409 && error?.error === 'INSUFFICIENT_GOODS') {
    throw new HttpError(409, 'INSUFFICIENT_GOODS', error.message ?? 'Insufficient goods');
  }
  throw new HttpError(502, 'INVENTORY_CALL_FAILED', `Inventory ${method} ${path} failed (${res.status}): ${error?.message ?? text}`, {
      method,
      path,
      status: res.status,
      message: error?.message ?? text,
    });
}

/** Hold parameters accepted by the Inventory service. */
export interface HoldParams {
  kind: 'stack' | 'instance';
  goodType: string;
  quantity?: number;
  instanceId?: string;
  refType: 'market_order' | 'mission_escrow' | 'manual';
  refId?: string;
}

/** Reserves goods of a holder for a mission escrow. */
export function createHold(holder: Holder, params: HoldParams): Promise<{ id: string }> {
  return call('POST', `/api/internal/holders/${holder.holderType}/${holder.holderId}/holds`, params);
}

/** Releases a mission escrow hold (goods become available again). */
export function releaseHold(holdId: string): Promise<unknown> {
  return call('POST', `/api/internal/holds/${holdId}/release`);
}

/** Consumes a mission escrow hold, transferring the reserved goods to a holder. */
export function consumeHold(holdId: string, to: Holder): Promise<unknown> {
  return call('POST', `/api/internal/holds/${holdId}/consume`, { to });
}

/** Moves available fungible goods between holders (system-funded rewards). */
export function transferStack(from: Holder, to: Holder, goodType: string, quantity: number): Promise<unknown> {
  return call('POST', '/api/internal/transfers', { from, to, goodType, quantity });
}

/** Moves a unique instance between holders. */
export function transferInstance(from: Holder, to: Holder, instanceId: string): Promise<unknown> {
  return call('POST', '/api/internal/transfers/instance', { from, to, instanceId });
}

/** The reserved system holder used as the item faucet for non-escrowed rewards. */
export function systemHolder(): Holder {
  return { holderType: 'system', holderId: env.inventory.systemHolderId };
}

/** One fungible stack with its reserved and available quantities. */
export interface StackView {
  goodType: string;
  quantity: number;
  held: number;
  available: number;
  [key: string]: unknown;
}

/**
 * Reads one holder's stack of a good type through the Inventory internal API.
 * Used by mission objective measurement (`owns_items`) and prerequisites.
 * @param holder - Holder (player or NPC).
 * @param goodType - Inventory good type.
 * @returns The stack, or null when the holder has none of that good.
 */
export async function getStack(holder: Holder, goodType: string): Promise<StackView | null> {
  if (!env.inventory.apiUrl) {
    throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'INVENTORY_API_URL is not configured');
  }
  const res = await fetch(
    `${env.inventory.apiUrl}/api/internal/holders/${holder.holderType}/${holder.holderId}/stacks/${encodeURIComponent(goodType)}`,
    { headers: { 'Accept-Language': currentLang(), ...(await authHeaders()) } },
  );
  if (res.status === 404) return null;
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'INVENTORY_READ_FAILED', `Inventory stack read failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
  }
  return (await res.json()) as StackView;
}
