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
    /** Optional expected `aud` claim for player tokens; empty disables the audience check. */
    audience: process.env.OIDC_AUDIENCE || undefined,
    /** Expected `aud` of service-account tokens (client_credentials) on `/api/internal/*`. */
    serviceAudience: process.env.OIDC_SERVICE_AUDIENCE || 'social-api',
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
  /** Reputation deltas and automatic sanction thresholds (all thresholds are negative scores). */
  reputation: {
    blockPenalty: int('REPUTATION_BLOCK_PENALTY', 1),
    reportPenalty: int('REPUTATION_REPORT_PENALTY', 2),
    upheldReportPenalty: int('REPUTATION_UPHELD_REPORT_PENALTY', 10),
    warnAt: int('REPUTATION_WARN_AT', -10),
    muteAt: int('REPUTATION_MUTE_AT', -25),
    muteHours: int('REPUTATION_MUTE_HOURS', 24),
    suspendAt: int('REPUTATION_SUSPEND_AT', -50),
    suspendHours: int('REPUTATION_SUSPEND_HOURS', 168),
    escalateAt: int('REPUTATION_ESCALATE_AT', -75),
    /** Players with no reputation event for this many days regain `rehabStep` points. */
    rehabAfterDays: int('REPUTATION_REHAB_AFTER_DAYS', 7),
    rehabStep: int('REPUTATION_REHAB_STEP', 1),
    /** In-process rehabilitation pass interval in minutes; 0 disables (call the internal route instead). */
    rehabIntervalMinutes: int('REPUTATION_REHAB_INTERVAL_MINUTES', 60),
  },
};
