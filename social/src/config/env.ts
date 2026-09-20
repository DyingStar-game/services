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
    /** Optional expected `aud` claim; empty disables the audience check. */
    audience: process.env.OIDC_AUDIENCE || undefined,
  },
  /** Shared secret expected in `X-Internal-Key` on `/api/internal/*`. */
  internalApiKey: process.env.INTERNAL_API_KEY ?? '',
  /** Accept `X-Player-Id` instead of a JWT (never honoured in production). */
  authDevBypass: process.env.AUTH_DEV_BYPASS === 'true' && nodeEnv !== 'production',
};
