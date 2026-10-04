/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import {
  ASSIGNEE_HOLDER_TYPES,
  ASSIGNMENT_STATUSES,
  MISSION_CATEGORIES,
  MISSION_ISSUER_TYPES,
  MISSION_KINDS,
  MISSION_STATUSES,
  MISSION_VISIBILITIES,
} from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const missionIdParams = z.object({ missionId: uuidSchema });

export const objectiveParams = z.object({
  missionId: uuidSchema,
  objectiveId: uuidSchema,
});

export const playerIdParams = z.object({ playerId: uuidSchema });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/** Boolean query filter (`?isEvent=true|false`). */
const booleanQuery = z
  .enum(['true', 'false'])
  .transform((v) => v === 'true');

/** Optional filter on a mission listing. */
export const missionListQuery = limitQuery.extend({
  status: z.enum(MISSION_STATUSES).optional(),
  kind: z.enum(MISSION_KINDS).optional(),
  category: z.enum(MISSION_CATEGORIES).optional(),
  issuerType: z.enum(MISSION_ISSUER_TYPES).optional(),
  issuerId: uuidSchema.optional(),
  visibility: z.enum(MISSION_VISIBILITIES).optional(),
  groupId: uuidSchema.optional(),
  groupClaimable: booleanQuery.optional(),
  isEvent: booleanQuery.optional(),
});

export const assignmentListQuery = limitQuery.extend({
  status: z.enum(ASSIGNMENT_STATUSES).optional(),
});

/** Free-form location / payload objects (opaque to this service). */
const jsonObject = z.record(z.string(), z.unknown());

/** One reward component: credits (at most one per mission) or an item (distinct items). */
export const rewardComponentSchema = z.union([
  z
    .object({
      type: z.literal('credits'),
      currency: z.string().trim().min(1).max(16).default('credits'),
      amount: z.number().int().min(1).max(10_000_000_000_000),
    })
    .strict(),
  z
    .object({
      type: z.literal('item'),
      itemId: z.string().trim().min(1).max(128),
      quantity: z.number().int().min(1).max(1_000_000),
      /** Unique-instance reward (single assignee); omitted for fungible goods. */
      instanceId: uuidSchema.optional(),
    })
    .strict(),
]);

/** Reward list: ≤ 1 credits component, distinct item components. */
export const rewardsSchema = z
  .array(rewardComponentSchema)
  .max(20)
  .superRefine((components, ctx) => {
    const credits = components.filter((c) => c.type === 'credits');
    if (credits.length > 1) {
      ctx.addIssue({ code: z.ZodIssueCode.custom, message: 'At most one credits component is allowed', path: [0] });
    }
    const itemIds = components.filter((c) => c.type === 'item').map((c) => (c.type === 'item' ? c.itemId : ''));
    const duplicates = itemIds.filter((id, i) => itemIds.indexOf(id) !== i);
    if (duplicates.length > 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: `Duplicate item reward component: ${duplicates[0]}`,
        path: [0],
      });
    }
  });

/** One prerequisite gate: kind name + structured params (registry-validated). */
export const prerequisiteSchema = z
  .object({
    kind: z.string().trim().min(1).max(64),
    params: jsonObject.optional(),
  })
  .strict();

// ── Zones ────────────────────────────────────────────────────────────────────

const zoneSystem = z.string().trim().min(1).max(64);
const zoneScene = z.string().trim().min(1).max(128);
const zoneCenter = z
  .object({ x: z.number(), y: z.number(), z: z.number() })
  .strict();

/** One availability zone, matched against the player's presence location. */
export const missionZoneSchema = z.union([
  z.object({ kind: z.literal('system'), system: zoneSystem }).strict(),
  z
    .object({ kind: z.literal('scene'), system: zoneSystem.optional(), scene: zoneScene })
    .strict(),
  z
    .object({
      kind: z.literal('area'),
      system: zoneSystem.optional(),
      center: zoneCenter,
      radiusM: z.number().int().min(1).max(1_000_000),
    })
    .strict(),
]);

/** Zone list: empty = global (visible everywhere), otherwise at least one must match. */
const zonesArray = z.array(missionZoneSchema).max(20);
export const zonesSchema = zonesArray.default([]);

export const objectiveInput = z.object({
  /** Objective kind name (validated against the kind registry). */
  type: z.string().trim().min(1).max(64),
  title: z.string().trim().min(1).max(200),
  description: z.string().trim().max(2000).optional(),
  targetQuantity: z.number().int().min(1).max(1_000_000_000).optional(),
  unit: z.string().trim().max(32).optional(),
  locationFrom: jsonObject.optional(),
  locationTo: jsonObject.optional(),
  order: z.number().int().min(0).max(10_000).optional(),
  /** Kind-specific structured params (validated by the kind's params schema). */
  params: jsonObject.optional(),
  payload: jsonObject.optional(),
});

export const createMissionBody = z.object({
  title: z.string().trim().min(1).max(200),
  description: z.string().trim().max(4000).optional(),
  kind: z.enum(MISSION_KINDS).default('dynamic'),
  category: z.enum(MISSION_CATEGORIES).default('generic'),
  issuerType: z.enum(MISSION_ISSUER_TYPES).default('system'),
  issuerId: uuidSchema.nullish(),
  /** Reward components; empty/omitted for reputation-only missions. */
  rewards: rewardsSchema.optional(),
  /** Prerequisite gates checked at acceptance (kind registry). */
  prerequisites: z.array(prerequisiteSchema).max(20).default([]),
  maxAssignees: z.number().int().min(1).max(1000).default(1),
  /** Limited to a single group: the first group to accept claims it. */
  groupClaimable: z.boolean().default(false),
  /** Big event mission (open multijoueur, declared by the game server or a corporation). */
  isEvent: z.boolean().default(false),
  /** Availability zones (empty = global). */
  zones: zonesSchema,
  scriptId: z.string().trim().min(1).max(128).nullish(),
  expiresAt: z.coerce.date().nullish(),
  objectives: z.array(objectiveInput).min(1).max(100),
});

export const createPlayerMissionBody = z
  .object({
    title: z.string().trim().min(1).max(200),
    description: z.string().trim().max(4000).optional(),
    category: z.enum(MISSION_CATEGORIES).default('generic'),
    visibility: z.enum(MISSION_VISIBILITIES).default('public'),
    /** Required when `visibility` is `corporation`. */
    corporationId: uuidSchema.nullish(),
    /** Alternative issuer: a political entity (commune … federation, owned by Social). */
    politicalEntityId: uuidSchema.nullish(),
    /**
     * Who funds the escrow: `creator` pays from their own wallet (default), `issuer` debits
     * the issuing organization's treasury (corporation or political entity).
     */
    escrowSource: z.enum(['creator', 'issuer']).default('creator'),
    /** Reward components, escrowed from the creator (≥ 1 required). */
    rewards: rewardsSchema.min(1),
    prerequisites: z.array(prerequisiteSchema).max(20).default([]),
    maxAssignees: z.number().int().min(1).max(100).default(1),
    /** Limited to a single group: the first group to accept claims it. */
    groupClaimable: z.boolean().default(false),
    /** Availability zones (empty = global). */
    zones: zonesSchema,
    expiresAt: z.coerce.date().nullish(),
    objectives: z.array(objectiveInput).min(1).max(100),
  })
  .refine((v) => v.visibility !== 'corporation' || !!v.corporationId, {
    message: 'corporationId is required for corporation missions',
    path: ['corporationId'],
  })
  .refine((v) => !(v.corporationId && v.politicalEntityId), {
    message: 'corporationId and politicalEntityId are mutually exclusive',
    path: ['politicalEntityId'],
  })
  .refine((v) => v.escrowSource !== 'issuer' || !!(v.corporationId || v.politicalEntityId), {
    message: 'escrowSource "issuer" requires corporationId or politicalEntityId',
    path: ['escrowSource'],
  });

/**
 * Mutable mission fields. The spec (objectives, prerequisites, rewards, capacity) is
 * immutable after creation: the escrow is taken against it at creation time.
 */
export const updateMissionBody = z
  .object({
    title: z.string().trim().min(1).max(200).optional(),
    description: z.string().trim().max(4000).nullish(),
    expiresAt: z.coerce.date().nullish(),
    /** Declare/lift the "big event" flag. */
    isEvent: z.boolean().optional(),
    /** Re-zone a mission (internal API: the game moves live events). No escrow impact. */
    zones: zonesArray.optional(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

/** Dry-run of a mission spec (`POST /api/missions/validate`). */
export const validateMissionBody = z.object({
  mode: z.enum(['player', 'internal']).default('internal'),
  mission: createMissionBody,
});

/** Share a mission with a group. */
export const shareBody = z.object({ groupId: uuidSchema });

/** Declare (or lift) the "big event" flag on a mission. */
export const eventBody = z.object({
  enabled: z.boolean(),
  maxAssignees: z.number().int().min(1).max(1000).optional(),
});

export const progressBody = z.object({
  quantity: z.number().int().min(1).max(1_000_000_000),
});

export const internalProgressBody = progressBody.extend({ playerId: uuidSchema });

export const internalCompleteBody = z.object({
  playerId: uuidSchema,
  force: z.boolean().optional(),
  settle: z.boolean().optional(),
});

export const settleBody = z.object({ playerId: uuidSchema });

/** Verify the service objectives of a holder's assignment (internal). */
export const internalVerifyBody = z.object({ playerId: uuidSchema });

/** Assign a mission to a holder (player or NPC) on the game server's behalf. */
export const internalAssignBody = z.object({
  playerId: uuidSchema,
  holderType: z.enum(ASSIGNEE_HOLDER_TYPES).default('player'),
});

export type CreateMissionBody = z.infer<typeof createMissionBody>;
export type UpdateMissionBody = z.infer<typeof updateMissionBody>;
