/**
 * Outbound client for the Social service internal API. Used to verify a player's
 * corporation membership when creating or accepting a corporation-scoped mission.
 * Authentication mirrors `economy.client`: mission's own Keycloak service account
 * (`client_credentials`) with a dev-only `X-Internal-Key` fallback.
 */
import { env } from '../config/env.js';
import { currentLang } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

/** Corporation and rank of a player, as returned by Social's internal API. */
export interface PlayerCorporation {
  id: string;
  name: string;
  ticker: string;
  rank: { id: string; name: string; priority: number; permissions: string[]; isCeo: boolean };
  [key: string]: unknown;
}

/** Group reference, as returned by Social's internal API. */
export interface PlayerGroup {
  id: string;
  name: string;
  [key: string]: unknown;
}

/** Membership of a player in a group, as returned by Social's internal API. */
export interface PlayerGroupMembership {
  group: PlayerGroup;
  member: { groupId: string; playerId: string; joinedAt: string; [key: string]: unknown };
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
      client_id: env.social.serviceClientId,
      client_secret: env.social.serviceClientSecret,
    }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'SOCIAL_AUTH_FAILED', `Social token request failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
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

/** Calls a Social internal endpoint (path starting with `/api/internal/...`) and returns the parsed body. */
async function fetchInternal<T>(path: string): Promise<T> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const res = await fetch(`${env.social.apiUrl}${path}`, {
    headers: { 'Accept-Language': currentLang(), ...(await authHeaders()) },
  });
  if (res.ok) {
    return (await res.json()) as T;
  }
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
}

/** Calls the Social internal corporation endpoint and returns the parsed body. */
async function fetchMappings<T>(playerId: string, corporationId?: string): Promise<T> {
  const query = corporationId ? `?corporationId=${encodeURIComponent(corporationId)}` : '';
  return fetchInternal<T>(`/api/internal/players/${playerId}/corporation${query}`);
}

/**
 * Every corporation membership of a player (a player may belong to several corporations).
 * @param playerId - Player id.
 * @returns Corporation contexts.
 */
export async function listPlayerCorporations(playerId: string): Promise<PlayerCorporation[]> {
  return (await fetchMappings<PlayerCorporation[] | null>(playerId)) ?? [];
}

/**
 * A player's membership in one corporation, or null.
 * @param playerId - Player id.
 * @param corporationId - Corporation id.
 * @returns Corporation context, or null.
 */
export async function getPlayerCorporationIn(
  playerId: string,
  corporationId: string,
): Promise<PlayerCorporation | null> {
  return fetchMappings<PlayerCorporation | null>(playerId, corporationId);
}

/**
 * Whether a player is a member of the given corporation.
 * @param playerId - Player id.
 * @param corporationId - Corporation id.
 * @returns True when the player belongs to that corporation.
 */
export async function isCorporationMember(playerId: string, corporationId: string): Promise<boolean> {
  return (await getPlayerCorporationIn(playerId, corporationId)) !== null;
}

/**
 * The (single) group a player belongs to, or null. A player belongs to at most one group.
 * @param playerId - Player id.
 * @returns Membership, or null.
 */
export async function getPlayerGroup(playerId: string): Promise<PlayerGroupMembership | null> {
  return fetchInternal<PlayerGroupMembership | null>(`/api/internal/players/${playerId}/group`);
}

/**
 * Whether a player is a member of the given group.
 * @param playerId - Player id.
 * @param groupId - Group id.
 * @returns True when the player belongs to that group.
 */
export async function isGroupMember(playerId: string, groupId: string): Promise<boolean> {
  const membership = await fetchInternal<PlayerGroupMembership | null>(
    `/api/internal/players/${playerId}/group?groupId=${encodeURIComponent(groupId)}`,
  );
  return membership !== null;
}

/**
 * A group reference, or null when it does not exist (used to validate a share target).
 * @param groupId - Group id.
 * @returns Group, or null.
 */
export async function getGroup(groupId: string): Promise<PlayerGroup | null> {
  return fetchInternal<PlayerGroup | null>(`/api/internal/groups/${encodeURIComponent(groupId)}`);
}

/** Public profile fields mission checks care about (reputation gate). */
export interface PlayerProfileInfo {
  playerId: string;
  displayName?: string;
  reputation: number;
  [key: string]: unknown;
}

/**
 * A player's profile (with reputation), or null when it does not exist.
 * @param playerId - Player id.
 * @returns Profile, or null.
 */
export async function getPlayerProfile(playerId: string): Promise<PlayerProfileInfo | null> {
  return fetchInternal<PlayerProfileInfo | null>(`/api/internal/players/${playerId}`);
}

/** Presence of a player as reported by the game server (Social internal API). */
export interface PlayerPresence {
  playerId: string;
  status: string;
  location: { system?: string; scene?: string; position?: { x: number; y: number; z: number } } | null;
  updatedAt?: string;
  [key: string]: unknown;
}

/**
 * The player's presence (status + location). Defaults to offline/null when the player
 * was never seen — used for mission zone matching.
 * @param playerId - Player id.
 * @returns Presence.
 */
export async function getPlayerPresence(playerId: string): Promise<PlayerPresence> {
  return fetchInternal<PlayerPresence>(`/api/internal/players/${playerId}/presence`);
}
