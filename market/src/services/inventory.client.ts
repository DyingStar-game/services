/**
 * Outbound client for the Inventory service internal API. The market uses it to move
 * ownership of traded goods at settlement. Authentication mirrors the other services:
 * market's own Keycloak service account (`client_credentials`) with a dev-only
 * `X-Internal-Key` fallback.
 */
import { env } from '../config/env.js';
import { currentLang } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

/** Holder reference shared with the inventory service. */
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
    throw new HttpError(502, 'INVENTORY_AUTH_FAILED', `Inventory token request failed (${res.status}): ${text}`, { status: res.status, body: text });
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

/** Generic JSON call to the Inventory internal API; turns non-2xx into a 502/409. */
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
  const code = (parsed as { error?: string } | null)?.error;
  const message = (parsed as { message?: string } | null)?.message ?? text;
  // Insufficient goods is a caller-visible conflict, not an upstream failure.
  if (res.status === 409 && code === 'INSUFFICIENT_GOODS') {
    throw new HttpError(409, 'INSUFFICIENT_GOODS', message ?? 'Insufficient goods');
  }
  throw new HttpError(502, 'INVENTORY_CALL_FAILED', `Inventory ${method} ${path} failed (${res.status}): ${message}`, { method, path, status: res.status, message });
}

/** Moves available fungible goods between holders. */
export function transferStack(from: Holder, to: Holder, goodType: string, quantity: number): Promise<unknown> {
  return call('POST', '/api/internal/transfers', { from, to, goodType, quantity });
}

/** Moves a unique instance between holders. */
export function transferInstance(from: Holder, to: Holder, instanceId: string): Promise<unknown> {
  return call('POST', '/api/internal/transfers/instance', { from, to, instanceId });
}

/** Reads a holder's inventory (used to validate availability before matching). */
export function getHolderInventory(holder: Holder): Promise<unknown> {
  return call('GET', `/api/internal/holders/${holder.holderType}/${holder.holderId}`);
}
