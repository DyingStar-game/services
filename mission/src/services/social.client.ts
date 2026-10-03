/**
 * Outbound client for the Social service internal API. Used to verify a player's
 * corporation membership when creating or accepting a corporation-scoped mission.
 * Authentication mirrors `economy.client`: mission's own Keycloak service account
 * (`client_credentials`) with a dev-only `X-Internal-Key` fallback.
 */
import { env } from '../config/env.js';
import { HttpError } from '../lib/httpError.js';

/** Corporation and rank of a player, as returned by Social's internal API. */
export interface PlayerCorporation {
  id: string;
  name: string;
  ticker: string;
  rank: { id: string; name: string; priority: number; permissions: string[]; isCeo: boolean };
  [key: string]: unknown;
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
    throw new HttpError(502, 'SOCIAL_AUTH_FAILED', `Social token request failed (${res.status}): ${text}`);
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

/** Calls the Social internal corporation endpoint and returns the parsed body. */
async function fetchMappings<T>(playerId: string, corporationId?: string): Promise<T> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const query = corporationId ? `?corporationId=${encodeURIComponent(corporationId)}` : '';
  const res = await fetch(`${env.social.apiUrl}/api/internal/players/${playerId}/corporation${query}`, {
    headers: { ...(await authHeaders()) },
  });
  if (res.ok) {
    return (await res.json()) as T;
  }
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`);
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
