// Outbound client for the Social service internal API (`/api/internal/*`).
// Authentication mirrors the other services: our own Keycloak service account
// (`client_credentials`); the token is cached until near expiry.

const DEFAULT_TOKEN_TTL_MARGIN_S = 30;

let tokenCache = null;

function env(name) {
  return process.env[name] || "";
}

/** Whether the Social integration is configured (URL + secret). */
export function isSocialConfigured() {
  return Boolean(env("CHAT_SOCIAL_API_URL") && env("CHAT_SOCIAL_SERVICE_CLIENT_SECRET"));
}

function tokenUrl() {
  const issuer = env("OIDC_ISSUER") || "http://keycloak:8080/realms/dyingstar";
  return env("OIDC_TOKEN_URL") || `${issuer}/protocol/openid-connect/token`;
}

async function getServiceToken() {
  const now = Date.now();
  if (tokenCache && tokenCache.expiresAt > now) return tokenCache.value;

  const res = await fetch(tokenUrl(), {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "client_credentials",
      client_id: env("CHAT_SOCIAL_SERVICE_CLIENT_ID") || "svc-chat",
      client_secret: env("CHAT_SOCIAL_SERVICE_CLIENT_SECRET"),
    }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`social token request failed (${res.status}): ${text}`);
  }
  const json = await res.json();
  const ttl = Math.max(30, (json.expires_in ?? 60) - DEFAULT_TOKEN_TTL_MARGIN_S);
  tokenCache = { value: json.access_token, expiresAt: now + ttl * 1000 };
  return tokenCache.value;
}

async function getJson(path) {
  const base = env("CHAT_SOCIAL_API_URL").replace(/\/$/, "");
  const res = await fetch(`${base}/api/internal${path}`, {
    headers: { Authorization: `Bearer ${await getServiceToken()}` },
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`social ${path} failed (${res.status}): ${text}`);
  }
  return res.json();
}

/**
 * Builds the membership probes used by `topics.js`, all backed by Social's
 * internal API. Transport errors throw (the caller fails closed); `isMuted`
 * is wrapped so a Social outage never blocks reading chat.
 *
 * @param cache TTL cache shared by all probes (see `cache.js`).
 */
export function createSocialChecks(cache) {
  return {
    async isFriend(a, b) {
      const [first, second] = [a, b].sort();
      return cache.getOrSet(`friend:${first}:${second}`, async () => {
        const data = await getJson(`/players/${a}/friendship/${b}`);
        return Boolean(data && data.friends);
      });
    },

    async inGroup(playerId, groupId) {
      return cache.getOrSet(`group:${playerId}:${groupId}`, async () => {
        const data = await getJson(
          `/players/${playerId}/group?groupId=${encodeURIComponent(groupId)}`,
        );
        return Boolean(data && data.member);
      });
    },

    async inCorp(playerId, corporationId) {
      return cache.getOrSet(`corp:${playerId}:${corporationId}`, async () => {
        const data = await getJson(
          `/players/${playerId}/corporation?corporationId=${encodeURIComponent(corporationId)}`,
        );
        return Boolean(data);
      });
    },

    async isMuted(playerId) {
      try {
        const sanctions = await cache.getOrSet(`sanctions:${playerId}`, async () => {
          const data = await getJson(`/players/${playerId}/sanctions`);
          return Array.isArray(data) ? data.map((s) => s.type) : [];
        });
        return sanctions.includes("mute");
      } catch {
        // Social down: mutes are enforced again on the next successful check;
        // never block chat delivery on a moderation-cache outage.
        return false;
      }
    },

    /** Active suspension/ban, if Social is reachable (used at connect time). */
    async isBlocking(playerId) {
      const sanctions = await cache.getOrSet(`sanctions:${playerId}`, async () => {
        const data = await getJson(`/players/${playerId}/sanctions`);
        return Array.isArray(data) ? data.map((s) => s.type) : [];
      });
      return sanctions.includes("suspension") || sanctions.includes("ban");
    },
  };
}
