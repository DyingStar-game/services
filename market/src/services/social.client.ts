/**
 * Outbound client for the Social service internal API. The market uses it to authorize
 * corporation orders (membership is authoritative in Social).
 */
import { env } from '../config/env.js';
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
    throw new HttpError(502, 'SOCIAL_AUTH_FAILED', `Social token request failed (${res.status}): ${text}`);
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

/**
 * Whether a player is a member of a corporation (delegated to Social).
 * @param playerId - Player id.
 * @param corporationId - Corporation id.
 * @returns True when the player belongs to that corporation.
 */
export async function isCorporationMember(playerId: string, corporationId: string): Promise<boolean> {
  if (!env.social.apiUrl) {
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const url = `${env.social.apiUrl}/api/internal/players/${playerId}/corporation?corporationId=${encodeURIComponent(corporationId)}`;
  const res = await fetch(url, { headers: { ...(await authHeaders()) } });
  if (res.ok) {
    return (await res.json()) !== null;
  }
  const text = await res.text().catch(() => '');
  throw new HttpError(502, 'SOCIAL_LOOKUP_FAILED', `Social lookup failed (${res.status}): ${text}`);
}
