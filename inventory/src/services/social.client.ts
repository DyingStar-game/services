/**
 * Outbound client for the Social service internal API: membership reads (inventory and
 * POI listings) plus the central ACL (`POST /api/internal/authorize`) that gates every
 * organization-scoped write. Authentication mirrors mission's clients: this service's own
 * Keycloak service account (`client_credentials`) with a dev-only `X-Internal-Key` fallback.
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

/** Headers used to authenticate against the Social internal API. */
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

/** Calls a `/api/internal/players/:playerId/...` mapping endpoint and parses its body. */
async function fetchPlayerMapping<T>(playerId: string, path: string): Promise<T> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const res = await fetch(`${env.social.apiUrl}${path}`, {
    headers: { 'Accept-Language': currentLang(), ...(await authHeaders()) },
  });
  if (res.ok) return (await res.json()) as T;
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`, {
    status: res.status,
    body: text,
  });
}

/** Corporation membership of a player, with their rank (permissions authoritative in Social). */
export interface CorporationMembership {
  rank: {
    id: string;
    name: string;
    priority: number;
    permissions: string[];
    isCeo: boolean;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

/** Political membership of a player, with their office (permissions authoritative in Social). */
export interface PoliticalMembership {
  office: {
    id: number;
    name: string;
    priority: number;
    permissions: string[];
    isHead: boolean;
    isDefault: boolean;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

/**
 * Whether a player is a member of a corporation (delegated to Social).
 * @param playerId - Player id.
 * @param corporationId - Corporation id.
 * @returns True when the player belongs to that corporation.
 */
export async function isCorporationMember(playerId: string, corporationId: string): Promise<boolean> {
  return (await getCorporationMembership(playerId, corporationId)) !== null;
}

/**
 * A player's corporation membership (with their rank), or null.
 * @param playerId - Player id.
 * @param corporationId - Corporation id.
 * @returns Membership, or null when the player is not a member.
 */
export async function getCorporationMembership(
  playerId: string,
  corporationId: string,
): Promise<CorporationMembership | null> {
  return fetchPlayerMapping<CorporationMembership | null>(
    playerId,
    `/api/internal/players/${playerId}/corporation?corporationId=${encodeURIComponent(corporationId)}`,
  );
}

/**
 * A player's political membership (with their office), or null.
 * @param playerId - Player id.
 * @param entityId - Political entity id.
 * @returns Membership, or null when the player is not a member.
 */
export async function getPoliticalMembership(
  playerId: string,
  entityId: string,
): Promise<PoliticalMembership | null> {
  return fetchPlayerMapping<PoliticalMembership | null>(
    playerId,
    `/api/internal/players/${playerId}/politics?entityId=${encodeURIComponent(entityId)}`,
  );
}

// ── Central authorization (Social is the policy decision point) ─────────────

/** Holder kinds understood by Social's authorization endpoint. */
export type AuthorizeHolderType = 'corporation' | 'political';

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
 * Asks Social whether a player holds an action on an organisation. Inventory never
 * interprets permissions itself: it maps `reason` onto its own error codes.
 * @param input - Holder, player and catalogued/free action.
 * @returns The decision.
 */
export async function authorizeAction(input: {
  holderType: AuthorizeHolderType;
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
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`, {
    status: res.status,
    body: text,
  });
}
