/**
 * Outbound client for the Mission service internal API: the **POI in-use lock**. Before a
 * player may move a POI (`PATCH /api/me/pois/:poiId`), Inventory asks Mission which of
 * the POIs they reference are locked by a mission whose assignment is active. A locked POI
 * may not move, so the destination of an accepted contract never changes under the
 * acceptor's feet.
 *
 * Authentication mirrors `social.client`: this service's own Keycloak service account
 * (`client_credentials`, role `mission:read`) with a dev-only `X-Internal-Key` fallback.
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
      client_id: env.mission.serviceClientId,
      client_secret: env.mission.serviceClientSecret,
    }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(
      502,
      'MISSION_AUTH_FAILED',
      `Mission token request failed (${res.status}): ${text}`,
      { status: res.status, body: text },
    );
  }
  const json = (await res.json()) as { access_token: string; expires_in?: number };
  const ttl = Math.max(30, (json.expires_in ?? 60) - 30);
  tokenCache = { value: json.access_token, expiresAt: now + ttl * 1000 };
  return json.access_token;
}

/** Headers used to authenticate against the Mission internal API. */
async function authHeaders(): Promise<Record<string, string>> {
  if (env.mission.serviceClientSecret) {
    return { Authorization: `Bearer ${await getServiceToken()}` };
  }
  if (env.mission.internalApiKey) {
    return { 'X-Internal-Key': env.mission.internalApiKey };
  }
  throw new HttpError(
    503,
    'MISSION_NOT_CONFIGURED',
    'No Mission credentials configured (set MISSION_SERVICE_CLIENT_SECRET or MISSION_INTERNAL_API_KEY)',
  );
}

/** Whether the Mission integration is configured (empty URL disables the lock entirely). */
export function isMissionConfigured(): boolean {
  return Boolean(
    env.mission.apiUrl && (env.mission.serviceClientSecret || env.mission.internalApiKey),
  );
}

/**
 * Asks Mission which of the given POIs are locked by a mission with an active assignee.
 * @param poiIds - POI ids to check.
 * @returns Map of `poiId` → mission ids locking it (a POI absent from the map is free).
 * @throws 502 `MISSION_LOOKUP_FAILED` when Mission answers non-2xx.
 */
export async function findPoisInUse(poiIds: string[]): Promise<Map<string, string[]>> {
  const wanted = [...new Set(poiIds)];
  if (wanted.length === 0) return new Map();
  if (!env.mission.apiUrl) {
    throw new HttpError(503, 'MISSION_NOT_CONFIGURED', 'MISSION_API_URL is not configured');
  }

  const res = await fetch(`${env.mission.apiUrl}/api/internal/poi-usage`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Accept-Language': currentLang(),
      ...(await authHeaders()),
    },
    body: JSON.stringify({ poiIds: wanted }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new HttpError(502, 'MISSION_LOOKUP_FAILED', `Mission POI lookup failed (${res.status}): ${text}`, {
      status: res.status,
      body: text,
    });
  }
  const body = (await res.json()) as Record<string, string[]>;
  return new Map(Object.entries(body));
}

/** Whether a POI is locked by a mission with an active assignee. */
export async function isPoiInUse(poiId: string): Promise<boolean> {
  const usage = await findPoisInUse([poiId]);
  return usage.has(poiId);
}
