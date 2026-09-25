/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import {
  CORPORATION_ROLES,
  TRANSACTION_TYPES,
} from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const playerIdParams = z.object({ playerId: uuidSchema });

export const corporationIdParams = z.object({ corporationId: uuidSchema });

export const corporationMemberParams = corporationIdParams.extend({ playerId: uuidSchema });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/** Amounts are integer minor units. */
const amount = z.number().int().min(1).max(10_000_000_000_000);

/** Optional currency code; defaults to `credits` in the services. */
const currency = z.string().trim().min(1).max(16).optional();

/** Types trusted callers may report on credits/debits. */
const INTERNAL_TYPES = TRANSACTION_TYPES.filter((t) => !['transfer', 'donation', 'tax'].includes(t));

export const transferBody = z.object({
  toPlayerId: uuidSchema,
  amount,
  memo: z.string().trim().max(256).optional(),
});

export const movementBody = z.object({
  amount,
  currency,
  reference: z.string().trim().max(128).optional(),
  externalId: z.string().trim().min(1).max(128).optional(),
  type: z.enum(INTERNAL_TYPES).optional(),
});
export type MovementBody = z.infer<typeof movementBody>;

export const donationBody = z.object({
  amount,
  memo: z.string().trim().max(256).optional(),
});

export const memberRoleBody = z.object({
  role: z.enum(CORPORATION_ROLES),
});

export const corporationSettingsBody = z
  .object({
    taxRateBps: z.number().int().min(0).max(10_000).optional(),
    allowDonations: z.boolean().optional(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

/** Optional report period (ISO date/time, inclusive bounds). */
export const reportQuery = z.object({
  from: z.coerce.date().optional(),
  to: z.coerce.date().optional(),
});