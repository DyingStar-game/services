/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import { PRESENCE_STATUSES } from '../db/schema/index.js';

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
