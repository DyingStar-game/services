/**
 * Reputation history, reports, sanctions and the moderation audit log.
 */
import { boolean, index, integer, jsonb, pgTable, text, timestamp, uuid } from 'drizzle-orm/pg-core';

import { guilds } from './guilds.js';
import { playerProfiles } from './profiles.js';

/** Where a reputation change comes from. */
export const REPUTATION_SOURCES = ['game', 'block', 'report', 'sanction', 'moderation', 'rehabilitation'] as const;
export type ReputationSource = (typeof REPUTATION_SOURCES)[number];

/** Every change to `player_profiles.reputation` is recorded here. */
export const reputationEvents = pgTable(
  'reputation_events',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    delta: integer('delta').notNull(),
    /** Score after applying the delta. */
    balance: integer('balance').notNull(),
    source: text('source').$type<ReputationSource>().notNull(),
    reason: text('reason').notNull(),
    actorId: uuid('actor_id').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('reputation_events_player_created_idx').on(t.playerId, t.createdAt.desc())],
);

export type ReputationEvent = typeof reputationEvents.$inferSelect;

export const REPORT_TARGET_TYPES = ['player', 'guild'] as const;
export type ReportTargetType = (typeof REPORT_TARGET_TYPES)[number];

export const REPORT_REASONS = ['harassment', 'cheating', 'griefing', 'offensive_name', 'scam', 'other'] as const;
export type ReportReason = (typeof REPORT_REASONS)[number];

export const REPORT_STATUSES = ['open', 'reviewing', 'resolved', 'dismissed'] as const;
export type ReportStatus = (typeof REPORT_STATUSES)[number];

/** Escalation levels map to moderation roles. */
export const ESCALATION_LEVELS = ['moderator', 'admin', 'supervisor'] as const;
export type EscalationLevel = (typeof ESCALATION_LEVELS)[number];

export const reports = pgTable(
  'reports',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    /** Null for reports raised automatically by the system. */
    reporterId: uuid('reporter_id').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    targetType: text('target_type').$type<ReportTargetType>().notNull(),
    targetPlayerId: uuid('target_player_id').references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    targetGuildId: uuid('target_guild_id').references(() => guilds.id, { onDelete: 'cascade' }),
    reason: text('reason').$type<ReportReason | 'reputation_threshold'>().notNull(),
    message: text('message'),
    status: text('status').$type<ReportStatus>().notNull().default('open'),
    escalation: text('escalation').$type<EscalationLevel>().notNull().default('moderator'),
    resolvedBy: uuid('resolved_by'),
    resolutionNote: text('resolution_note'),
    resolvedAt: timestamp('resolved_at', { withTimezone: true }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index('reports_status_created_idx').on(t.status, t.createdAt.desc()),
    index('reports_target_player_idx').on(t.targetPlayerId),
  ],
);

export type Report = typeof reports.$inferSelect;

export const SANCTION_TYPES = ['warning', 'mute', 'suspension', 'ban'] as const;
export type SanctionType = (typeof SANCTION_TYPES)[number];

export const sanctions = pgTable(
  'sanctions',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    type: text('type').$type<SanctionType>().notNull(),
    reason: text('reason').notNull(),
    /** True when issued by the reputation thresholds rather than a moderator. */
    automatic: boolean('automatic').notNull().default(false),
    issuedBy: uuid('issued_by'),
    /** Null = permanent (or instantaneous for warnings). */
    expiresAt: timestamp('expires_at', { withTimezone: true }),
    revokedAt: timestamp('revoked_at', { withTimezone: true }),
    revokedBy: uuid('revoked_by'),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('sanctions_player_created_idx').on(t.playerId, t.createdAt.desc())],
);

export type Sanction = typeof sanctions.$inferSelect;

/** Audit trail of moderator actions. */
export const moderationLog = pgTable(
  'moderation_log',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    actorId: uuid('actor_id'),
    action: text('action').notNull(),
    targetPlayerId: uuid('target_player_id'),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('moderation_log_created_idx').on(t.createdAt.desc())],
);

export type ModerationLogEntry = typeof moderationLog.$inferSelect;
