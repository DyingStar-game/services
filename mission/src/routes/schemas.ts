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
  OBJECTIVE_TYPES,
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

/** Optional filter on a mission listing. */
export const missionListQuery = limitQuery.extend({
  status: z.enum(MISSION_STATUSES).optional(),
  kind: z.enum(MISSION_KINDS).optional(),
  category: z.enum(MISSION_CATEGORIES).optional(),
  issuerType: z.enum(MISSION_ISSUER_TYPES).optional(),
  issuerId: uuidSchema.optional(),
  visibility: z.enum(MISSION_VISIBILITIES).optional(),
});

export const assignmentListQuery = limitQuery.extend({
  status: z.enum(ASSIGNMENT_STATUSES).optional(),
});

/** Free-form location / payload objects (opaque to this service). */
const jsonObject = z.record(z.string(), z.unknown());

/** Reward: at least one of `economic` or `item`. */
export const rewardSchema = z
  .object({
    economic: z
      .object({
        currency: z.string().trim().min(1).max(16).default('credits'),
        amount: z.number().int().min(1).max(10_000_000_000_000),
      })
      .optional(),
    item: z
      .object({
        itemId: z.string().trim().min(1).max(128),
        quantity: z.number().int().min(1).max(1_000_000),
        /** Unique-instance reward (single assignee); omitted for fungible goods. */
        instanceId: uuidSchema.optional(),
      })
      .optional(),
  })
  .refine((v) => v.economic !== undefined || v.item !== undefined, {
    message: 'A reward requires an economic or item component',
  });

export const objectiveInput = z.object({
  type: z.enum(OBJECTIVE_TYPES),
  title: z.string().trim().min(1).max(200),
  description: z.string().trim().max(2000).optional(),
  targetQuantity: z.number().int().min(1).max(1_000_000_000).optional(),
  unit: z.string().trim().max(32).optional(),
  locationFrom: jsonObject.optional(),
  locationTo: jsonObject.optional(),
  order: z.number().int().min(0).max(10_000).optional(),
  payload: jsonObject.optional(),
});

export const createMissionBody = z.object({
  title: z.string().trim().min(1).max(200),
  description: z.string().trim().max(4000).optional(),
  kind: z.enum(MISSION_KINDS).default('dynamic'),
  category: z.enum(MISSION_CATEGORIES).default('generic'),
  issuerType: z.enum(MISSION_ISSUER_TYPES).default('system'),
  issuerId: uuidSchema.nullish(),
  reward: rewardSchema.nullish(),
  maxAssignees: z.number().int().min(1).max(1000).default(1),
  scriptId: z.string().trim().min(1).max(128).nullish(),
  expiresAt: z.coerce.date().nullish(),
  objectives: z.array(objectiveInput).min(1).max(100),
});

/** Player-sponsored reward: economic and/or item (the item is escrowed from the creator). */
export const playerRewardSchema = z
  .object({
    economic: z
      .object({
        currency: z.string().trim().min(1).max(16).default('credits'),
        amount: z.number().int().min(1).max(10_000_000_000_000),
      })
      .optional(),
    item: z
      .object({
        itemId: z.string().trim().min(1).max(128),
        quantity: z.number().int().min(1).max(1_000_000),
        instanceId: uuidSchema.optional(),
      })
      .optional(),
  })
  .refine((v) => v.economic !== undefined || v.item !== undefined, {
    message: 'A reward requires an economic or item component',
  });

export const createPlayerMissionBody = z
  .object({
    title: z.string().trim().min(1).max(200),
    description: z.string().trim().max(4000).optional(),
    category: z.enum(MISSION_CATEGORIES).default('generic'),
    visibility: z.enum(MISSION_VISIBILITIES).default('public'),
    /** Required when `visibility` is `corporation`. */
    corporationId: uuidSchema.nullish(),
    reward: playerRewardSchema,
    maxAssignees: z.number().int().min(1).max(100).default(1),
    expiresAt: z.coerce.date().nullish(),
    objectives: z.array(objectiveInput).min(1).max(100),
  })
  .refine((v) => v.visibility !== 'corporation' || !!v.corporationId, {
    message: 'corporationId is required for corporation missions',
    path: ['corporationId'],
  });

export const updateMissionBody = z
  .object({
    title: z.string().trim().min(1).max(200).optional(),
    description: z.string().trim().max(4000).nullish(),
    reward: rewardSchema.nullish(),
    maxAssignees: z.number().int().min(1).max(1000).optional(),
    expiresAt: z.coerce.date().nullish(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

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

/** Assign a mission to a holder (player or NPC) on the game server's behalf. */
export const internalAssignBody = z.object({
  playerId: uuidSchema,
  holderType: z.enum(ASSIGNEE_HOLDER_TYPES).default('player'),
});

export type CreateMissionBody = z.infer<typeof createMissionBody>;
export type UpdateMissionBody = z.infer<typeof updateMissionBody>;
