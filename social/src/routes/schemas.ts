/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import {
  CORPORATION_PERMISSIONS,
  CORPORATION_RECRUITMENT_MODES,
  ENTITY_TYPES,
  ESCALATION_LEVELS,
  PRESENCE_STATUSES,
  REPORT_REASONS,
  REPORT_STATUSES,
  REPORT_TARGET_TYPES,
  SANCTION_TYPES,
} from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const playerIdParams = z.object({ playerId: uuidSchema });

export const requestIdParams = z.object({ id: z.coerce.number().int().positive() });

export const targetPlayerBody = z.object({ playerId: uuidSchema });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

export const searchQuery = limitQuery.extend({
  search: z.string().trim().max(64).default(''),
});

/** Profile search with an optional profile-kind filter (`all` = both kinds). */
export const profileSearchQuery = searchQuery.extend({
  entityType: z.enum(ENTITY_TYPES).optional(),
});

const nullableText = (max: number) => z.string().trim().max(max).nullable().optional();

export const rpSheetSchema = z.object({
  characterName: z.string().trim().max(64).optional(),
  story: z.string().trim().max(4000).optional(),
  alignment: z.string().trim().max(32).optional(),
});

export const profilePatchBody = z
  .object({
    displayName: z.string().trim().min(2).max(32).optional(),
    avatarUrl: z.string().trim().url().max(512).nullable().optional(),
    faction: nullableText(64),
    biography: nullableText(2000),
    rpSheet: rpSheetSchema.nullable().optional(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

export const locationSchema = z.object({
  system: z.string().trim().max(64).optional(),
  scene: z.string().trim().max(128).optional(),
  position: z.object({ x: z.number(), y: z.number(), z: z.number() }).optional(),
});

export const presenceBody = z.object({
  status: z.enum(PRESENCE_STATUSES),
  location: locationSchema.nullable().optional(),
});

export const statsBody = z
  .object({
    playtimeSecondsDelta: z.number().int().min(0).optional(),
    reputationDelta: z.number().int().optional(),
    reputationReason: z.string().trim().max(128).optional(),
    level: z.number().int().min(0).optional(),
    role: nullableText(64),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

export const activityBody = z.object({
  type: z.string().trim().min(1).max(64),
  details: z.record(z.string(), z.unknown()).optional(),
});

export const encounterBody = z
  .object({ playerId: uuidSchema, otherPlayerId: uuidSchema })
  .refine((v) => v.playerId !== v.otherPlayerId, { message: 'playerId and otherPlayerId must differ' });

export const upsertPlayerBody = z.object({
  displayName: z.string().trim().min(2).max(32),
});

// ── NPCs (internal API, game server) ────────────────────────────────────────

export const npcProfileBody = z.object({
  displayName: z.string().trim().min(2).max(32),
  avatarUrl: z.string().trim().url().max(512).nullable().optional(),
  faction: nullableText(64),
  biography: nullableText(2000),
  role: nullableText(64),
  level: z.number().int().min(0).optional(),
});

export const npcCorporationBody = z.object({
  corporationId: uuidSchema,
  rankId: z.number().int().positive().optional(),
});

// ── Corporations ────────────────────────────────────────────────────────────

export const corporationIdParams = z.object({ corporationId: uuidSchema });
export const corporationMemberParams = corporationIdParams.extend({ playerId: uuidSchema });
export const corporationRankParams = corporationIdParams.extend({ rankId: z.coerce.number().int().positive() });
export const corporationRequestParams = corporationIdParams.extend({ id: z.coerce.number().int().positive() });

const corporationName = z.string().trim().min(3).max(48);
const corporationTicker = z
  .string()
  .trim()
  .min(2)
  .max(5)
  .regex(/^[A-Za-z0-9]+$/, 'Ticker must be alphanumeric')
  .transform((t) => t.toUpperCase());

export const createCorporationBody = z.object({
  name: corporationName,
  ticker: corporationTicker,
  description: nullableText(2000),
  logoUrl: z.string().trim().url().max(512).nullable().optional(),
  recruitment: z.enum(CORPORATION_RECRUITMENT_MODES).optional(),
});

export const corporationPatchBody = z
  .object({
    name: corporationName.optional(),
    ticker: corporationTicker.optional(),
    description: nullableText(2000),
    logoUrl: z.string().trim().url().max(512).nullable().optional(),
    recruitment: z.enum(CORPORATION_RECRUITMENT_MODES).optional(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

export const rankBody = z.object({
  name: z.string().trim().min(1).max(32),
  priority: z.number().int().min(0).max(99),
  permissions: z.array(z.enum(CORPORATION_PERMISSIONS)).default([]),
  isDefault: z.boolean().optional(),
});

// Not `rankBody.partial()`: the `permissions` default would turn every patch into a permissions change.
export const rankPatchBody = z
  .object({
    name: z.string().trim().min(1).max(32).optional(),
    priority: z.number().int().min(0).max(99).optional(),
    permissions: z.array(z.enum(CORPORATION_PERMISSIONS)).optional(),
    isDefault: z.boolean().optional(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

export const memberRankBody = z.object({ rankId: z.number().int().positive() });

export const joinCorporationBody = z.object({ message: z.string().trim().max(500).optional() });

// ── Reputation & moderation ─────────────────────────────────────────────────

export const createReportBody = z.object({
  targetType: z.enum(REPORT_TARGET_TYPES),
  targetId: uuidSchema,
  reason: z.enum(REPORT_REASONS),
  message: z.string().trim().max(1000).optional(),
});

export const reportIdParams = z.object({ id: z.coerce.number().int().positive() });

export const reportsQuery = limitQuery.extend({
  status: z.enum(REPORT_STATUSES).optional(),
  escalation: z.enum(ESCALATION_LEVELS).optional(),
  targetPlayerId: uuidSchema.optional(),
});

export const reportStatusBody = z.object({
  status: z.enum(['reviewing', 'resolved', 'dismissed']),
  note: z.string().trim().max(1000).optional(),
});

export const reputationAdjustBody = z.object({
  delta: z.number().int().min(-100).max(100).refine((d) => d !== 0, { message: 'delta must not be 0' }),
  reason: z.string().trim().min(1).max(128),
});

export const sanctionBody = z.object({
  type: z.enum(SANCTION_TYPES),
  reason: z.string().trim().min(1).max(256),
  durationHours: z.number().int().positive().max(24 * 365).nullable().optional(),
});

export const sanctionIdParams = z.object({ id: z.coerce.number().int().positive() });

export const sanctionsQuery = limitQuery.extend({
  playerId: uuidSchema.optional(),
  active: z
    .enum(['true', 'false'])
    .default('true')
    .transform((v) => v === 'true'),
});
