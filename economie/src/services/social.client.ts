/**
 * Outbound client for the Social service internal API. Used by the admin dashboard to
 * resolve player pseudonyms (Economy only stores opaque holder ids). Authentication
 * mirrors mission's clients: Economy's own Keycloak service account (`client_credentials`)
 * with a dev-only `X-Internal-Key` fallback.
 */
import { env } from '../config/env.js';
import { currentLang } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

/** Minimal profile identity returned by Social's internal resolver. */
export interface ProfileIdentity {
  playerId: string;
  displayName: string;
  entityType: string;
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

/** GET helper against Social's internal API. */
async function fetchProfiles(params: URLSearchParams): Promise<ProfileIdentity[]> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const res = await fetch(`${env.social.apiUrl}/api/internal/players?${params.toString()}`, {
    headers: { 'Accept-Language': currentLang(), ...(await authHeaders()) },
  });
  if (res.ok) return (await res.json()) as ProfileIdentity[];
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`, { status: res.status, body: text });
}

/**
 * Searches profiles by display-name substring.
 * @param search - Substring to match.
 * @param limit - Max results.
 * @returns Matching profiles.
 */
export function searchProfiles(search: string, limit: number): Promise<ProfileIdentity[]> {
  return fetchProfiles(new URLSearchParams({ search, limit: String(limit) }));
}

/**
 * Resolves several player ids to their display names.
 * @param playerIds - Player ids to resolve.
 * @returns Known profiles among the given ids.
 */
export function resolveProfiles(playerIds: string[]): Promise<ProfileIdentity[]> {
  if (playerIds.length === 0) return Promise.resolve([]);
  return fetchProfiles(new URLSearchParams({ playerIds: playerIds.join(',') }));
}
