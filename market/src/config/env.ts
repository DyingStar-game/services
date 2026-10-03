/**
 * Environment configuration loaded once at startup (see `.env.example`).
 */
import dotenv from 'dotenv';

dotenv.config();

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
    serviceAudience: process.env.OIDC_SERVICE_AUDIENCE || 'market-api',
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
  /** Inventory integration: holds and transfers the traded goods (empty URL disables it). */
  inventory: {
    apiUrl: (process.env.INVENTORY_API_URL ?? 'http://localhost:3002').replace(/\/$/, ''),
    serviceClientId: process.env.INVENTORY_SERVICE_CLIENT_ID ?? 'svc-market',
    serviceClientSecret: process.env.INVENTORY_SERVICE_CLIENT_SECRET ?? '',
    internalApiKey: process.env.INVENTORY_INTERNAL_API_KEY ?? '',
  },
  /** Economy integration: holds and moves the credits (empty URL disables it). */
  economy: {
    apiUrl: (process.env.ECONOMY_API_URL ?? 'http://localhost:3000').replace(/\/$/, ''),
    serviceClientId: process.env.ECONOMY_SERVICE_CLIENT_ID ?? 'svc-market',
    serviceClientSecret: process.env.ECONOMY_SERVICE_CLIENT_SECRET ?? '',
    internalApiKey: process.env.ECONOMY_INTERNAL_API_KEY ?? '',
  },
  /** Social integration: authorizes corporation orders (empty URL disables it). */
  social: {
    apiUrl: (process.env.SOCIAL_API_URL ?? '').replace(/\/$/, ''),
    serviceClientId: process.env.SOCIAL_SERVICE_CLIENT_ID ?? 'svc-market',
    serviceClientSecret: process.env.SOCIAL_SERVICE_CLIENT_SECRET ?? '',
    internalApiKey: process.env.SOCIAL_INTERNAL_API_KEY ?? '',
  },
  /** Market rules. */
  market: {
    /** Default lifetime of an open order, in hours (0 = never expires). */
    orderTtlHours: parseInt(process.env.MARKET_ORDER_TTL_HOURS ?? '0', 10),
    /** Default page size for listings. */
    defaultLimit: parseInt(process.env.MARKET_DEFAULT_LIMIT ?? '20', 10),
  },
};
