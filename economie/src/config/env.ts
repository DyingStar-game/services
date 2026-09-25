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
    /** Optional expected `aud` claim; empty disables the audience check. */
    audience: process.env.OIDC_AUDIENCE || undefined,
  },
  /** Shared secret expected in `X-Internal-Key` on `/api/internal/*`. */
  internalApiKey: process.env.INTERNAL_API_KEY ?? '',
  /** Accept `X-Player-Id` instead of a JWT (never honoured in production). */
  authDevBypass: process.env.AUTH_DEV_BYPASS === 'true' && nodeEnv !== 'production',
  /** Economy rules (amounts are integer minor units, rates in basis points). */
  economy: {
    /** Automatic tax on player-to-player transfers (basis points, 500 = 5%). */
    transferTaxBps: int('ECONOMY_TRANSFER_TAX_BPS', 500),
    /** Absolute ceiling of the tax amount per transfer; 0 = no ceiling. */
    transferTaxCeiling: int('ECONOMY_TRANSFER_TAX_CEILING', 0),
    /** Smallest allowed transfer amount. */
    minTransfer: int('ECONOMY_MIN_TRANSFER', 1),
    /** Largest allowed transfer amount; 0 = no cap. */
    maxTransfer: int('ECONOMY_MAX_TRANSFER', 0),
    /** Upsert key of the reserved system account that collects automatic taxes. */
    taxVaultUuid: process.env.ECONOMY_TAX_VAULT_UUID ?? '00000000-0000-0000-0000-000000000001',
  },
};