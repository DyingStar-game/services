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
    serviceClients: (process.env.INTERNAL_SERVICE_CLIENTS ?? 'svc-game,svc-universesimulator')
      .split(',')
      .map((client) => client.trim())
      .filter(Boolean),
  },
  /** Valkey cache: live presence (TTL) and playtime accumulator. Empty URL disables it. */
  valkey: {
    url: process.env.VALKEY_URL ?? '',
    /** Presence TTL: a player without a heartbeat for this long reads back as offline. */
    presenceTtlSeconds: int('PRESENCE_TTL_SECONDS', 90),
    /** How often cached playtime deltas are flushed to PostgreSQL. */
    playtimeFlushIntervalMinutes: int('PLAYTIME_FLUSH_INTERVAL_MINUTES', 5),
    /** Maximum players accepted by one `PUT /players/presence/batch` heartbeat. */
    presenceBatchMax: int('PRESENCE_BATCH_MAX', 5000),
  },
  /** MQTT broker for ephemeral player notifications; empty URL disables the publisher. */
  mqtt: {
    url: process.env.MOSQUITTO_URL ?? '',
    /** Service client authenticating on the broker (its token is the MQTT password). */
    clientId: process.env.MQTT_CLIENT_ID ?? 'svc-social',
    /** Empty secret connects anonymously (dev broker without the go-auth plugin only). */
    clientSecret: process.env.MQTT_CLIENT_SECRET ?? '',
    /** Defaults to `${OIDC_ISSUER}/protocol/openid-connect/token` (client_credentials). */
    tokenUrl: process.env.MQTT_TOKEN_URL || `${issuer}/protocol/openid-connect/token`,
    /** Notifications are published to `{topicRoot}/{playerId}`. */
    topicRoot: process.env.MQTT_TOPIC_ROOT || 'notify',
  },
  /** In-game bug reports relayed to a Discord webhook; an empty URL turns the feature off. */
  bugReport: {
    /** The webhook URL — a SECRET (whoever holds it can post): from a Kubernetes Secret, never logged. */
    webhookUrl: process.env.DISCORD_BUG_WEBHOOK_URL ?? '',
    /**
     * Token of the Discord bot that READS the forum (the open bugs a report can be added to) — a
     * SECRET, never logged. Empty: no list, every report opens its own post.
     */
    botToken: process.env.DISCORD_BUG_BOT_TOKEN ?? '',
    /** The webhook's channel is a forum: each report opens a post named after its title. */
    forum: process.env.DISCORD_BUG_WEBHOOK_FORUM === 'true',
    /** Forum tag ids applied to each post (CSV), for a forum that requires a tag. */
    forumTagIds: (process.env.DISCORD_BUG_WEBHOOK_TAG_IDS ?? '')
      .split(',')
      .map((id) => id.trim())
      .filter(Boolean),
    /** Label of this environment in a report's title (`[BUG][PREPROD]…`); empty leaves it out. */
    environment: process.env.BUG_REPORT_ENVIRONMENT ?? '',
    /** Shortest time between two reports of one player. */
    minIntervalSeconds: int('BUG_REPORT_MIN_INTERVAL_SECONDS', 60),
    /** Reports one player may send in 24 hours. */
    dailyMax: int('BUG_REPORT_DAILY_MAX', 20),
  },
  /** Temporary groups (squads): one nobody came back to is disbanded by itself. */
  groups: {
    /** Hours with no member online after which a group is disbanded; 0 keeps groups for ever. */
    idleExpiryHours: int('GROUP_IDLE_EXPIRY_HOURS', 24),
    /** In-process expiry pass interval in minutes; 0 disables the pass. */
    expiryIntervalMinutes: int('GROUP_EXPIRY_INTERVAL_MINUTES', 15),
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
