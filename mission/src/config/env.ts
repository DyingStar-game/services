/**
 * Environment configuration loaded once at startup (see `.env.example`).
 */
import dotenv from 'dotenv';

dotenv.config();

function int(name: string, fallback: number): number {
  const raw = process.env[name];
  const value = raw === undefined || raw === '' ? NaN : Number(raw);
  return Number.isFinite(value) ? value : fallback;
}

function bool(name: string, fallback: boolean): boolean {
  const raw = process.env[name];
  if (raw === undefined || raw === '') return fallback;
  return raw === 'true' || raw === '1';
}

const issuer = process.env.OIDC_ISSUER ?? 'http://keycloak:8080/realms/dyingstar';
const nodeEnv = process.env.NODE_ENV ?? 'development';

export const env = {
  port: parseInt(process.env.PORT ?? '3000', 10),
  nodeEnv,
  corsOrigin: process.env.CORS_ORIGIN ?? 'http://localhost:5173',
  databaseUrl: process.env.DATABASE_URL ?? '',
  oidc: {
    issuer,
    jwksUrl: process.env.OIDC_JWKS_URL || `${issuer}/protocol/openid-connect/certs`,
    /** Defaults to `${OIDC_ISSUER}/protocol/openid-connect/token` (client_credentials). */
    tokenUrl: process.env.OIDC_TOKEN_URL || `${issuer}/protocol/openid-connect/token`,
    /** Optional expected `aud` claim for player tokens; empty disables the audience check. */
    audience: process.env.OIDC_AUDIENCE || undefined,
    /** Expected `aud` of service-account tokens (client_credentials) on `/api/internal/*`. */
    serviceAudience: process.env.OIDC_SERVICE_AUDIENCE || 'mission-api',
  },
  /** Legacy shared secret; only honoured when `internalDevBypass` is on. */
  internalApiKey: process.env.INTERNAL_API_KEY ?? '',
  /** Accept the legacy `X-Internal-Key` on `/api/internal/*` (never honoured in production). */
  internalDevBypass: process.env.INTERNAL_DEV_BYPASS === 'true' && nodeEnv !== 'production',
  /** Accept `X-Player-Id` instead of a JWT (never honoured in production). */
  authDevBypass: process.env.AUTH_DEV_BYPASS === 'true' && nodeEnv !== 'production',
  /** Trusted Keycloak service clients (`azp`) allowed on `/api/internal/*`. */
  internal: {
    serviceClients: (process.env.INTERNAL_SERVICE_CLIENTS ?? 'svc-game')
      .split(',')
      .map((client) => client.trim())
      .filter(Boolean),
  },
  /** Economy integration used to escrow and pay economic rewards. */
  economy: {
    apiUrl: (process.env.ECONOMY_API_URL ?? 'http://localhost:3000').replace(/\/$/, ''),
    /** Mission's own Keycloak service-account client, used for `client_credentials`. */
    serviceClientId: process.env.ECONOMY_SERVICE_CLIENT_ID ?? 'svc-mission',
    serviceClientSecret: process.env.ECONOMY_SERVICE_CLIENT_SECRET ?? '',
    /** Dev-only fallback: shared key sent as `X-Internal-Key` when no secret is set. */
    internalApiKey: process.env.ECONOMY_INTERNAL_API_KEY ?? '',
  },
  /** Social integration used to verify corporation membership. Empty URL disables it. */
  social: {
    apiUrl: (process.env.SOCIAL_API_URL ?? '').replace(/\/$/, ''),
    /** Service account used for `client_credentials` (defaults to the mission service client). */
    serviceClientId: process.env.SOCIAL_SERVICE_CLIENT_ID ?? process.env.ECONOMY_SERVICE_CLIENT_ID ?? 'svc-mission',
    serviceClientSecret: process.env.SOCIAL_SERVICE_CLIENT_SECRET ?? '',
    /** Dev-only fallback: shared key sent as `X-Internal-Key` when no secret is set. */
    internalApiKey: process.env.SOCIAL_INTERNAL_API_KEY ?? '',
  },
  /** Mission rules. */
  mission: {
    /** Pay economic rewards automatically on completion. */
    rewardAutoSettle: bool('MISSION_REWARD_AUTO_SETTLE', true),
    /** Default lifetime of a mission, in hours (0 = never expires). */
    defaultTtlHours: int('MISSION_DEFAULT_TTL_HOURS', 0),
  },
};
