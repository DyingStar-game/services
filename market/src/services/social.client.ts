/**
 * Outbound client for the Social service internal API: the central ACL
 * (`POST /api/internal/authorize`) that decides who may trade for a corporation.
 */
import { env } from '../config/env.js';
import { currentLang } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

interface TokenCache {
  value: string;
  expiresAt: number;
}

let tokenCache: TokenCache | null = null;

async function getServiceToken(): Promise<string> {
  const now = Date.now();
  if (tokenCache && tokenCache.expiresAt > now) return tokenCache.value;

  const res = await fetch(env.oidc.tokenUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'client_credentials',
      client_id: env.social.serviceClientId,
      client_secret: env.social.serviceClientSecret,
    }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'SOCIAL_AUTH_FAILED', `Social token request failed (${res.status}): ${text}`, { status: res.status, body: text });
  }
  const json = (await res.json()) as { access_token: string; expires_in?: number };
  const ttl = Math.max(30, (json.expires_in ?? 60) - 30);
  tokenCache = { value: json.access_token, expiresAt: now + ttl * 1000 };
  return json.access_token;
}

async function authHeaders(): Promise<Record<string, string>> {
  if (env.social.serviceClientSecret) {
    return { Authorization: `Bearer ${await getServiceToken()}` };
  }
  if (env.social.internalApiKey) {
    return { 'X-Internal-Key': env.social.internalApiKey };
  }
  throw new HttpError(
    503,
    'SOCIAL_NOT_CONFIGURED',
    'No Social credentials configured (set SOCIAL_SERVICE_CLIENT_SECRET or SOCIAL_INTERNAL_API_KEY)',
  );
}

/** Whether the Social integration is configured. */
export function isSocialConfigured(): boolean {
  return Boolean(env.social.apiUrl && (env.social.serviceClientSecret || env.social.internalApiKey));
}

// ── Central authorization (Social is the policy decision point) ─────────────

/** Why Social refused an action. */
export type AuthorizeReason = 'allowed' | 'not_member' | 'missing_permission';

/** Outcome of `POST /api/internal/authorize` — always a result, never an HTTP error. */
export interface AuthorizeResult {
  allowed: boolean;
  reason: AuthorizeReason;
  /** Rank/office name held, or null when the player is not a member. */
  role: string | null;
  /** True when the rank is the CEO / the office is the head. */
  leader: boolean;
  permissions: string[];
}

/**
 * Asks Social whether a player holds an action on an organisation. The market never
 * interprets permissions itself: it maps `reason` onto its own error codes.
 * @param input - Holder, player and catalogued/free action.
 * @returns The decision.
 */
export async function authorizeAction(input: {
  holderType: 'corporation' | 'political';
  holderId: string;
  playerId: string;
  action: string;
}): Promise<AuthorizeResult> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const res = await fetch(`${env.social.apiUrl}/api/internal/authorize`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'Accept-Language': currentLang(), ...(await authHeaders()) },
    body: JSON.stringify(input),
  });
  if (res.ok) return (await res.json()) as AuthorizeResult;
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`, { status: res.status, body: text });
}

// ── Player notifications ─────────────────────────────────────────────────────

/** A best-effort push relayed by Social to the player's MQTT channel. */
export interface PlayerNotification {
  type: string;
  title?: string;
  body?: string;
  data?: Record<string, unknown>;
}

/**
 * Asks Social to notify a player (only while they are online; the message is
 * ephemeral — no offline storage, so `{ delivered: false }` is not an error).
 * @param playerId - Target player.
 * @param notification - Content (`type` drives client-side rendering).
 * @returns Whether the broker accepted the message.
 */
export async function notifyPlayer(
  playerId: string,
  notification: PlayerNotification,
): Promise<{ delivered: boolean; reason?: string }> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const res = await fetch(`${env.social.apiUrl}/api/internal/players/${playerId}/notifications`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'Accept-Language': currentLang(), ...(await authHeaders()) },
    body: JSON.stringify(notification),
  });
  if (res.ok) return (await res.json()) as { delivered: boolean; reason?: string };
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`, { status: res.status, body: text });
}
