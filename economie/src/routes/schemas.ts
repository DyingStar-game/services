/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import {
  CORPORATION_MEMBER_HOLDER_TYPES,
  CORPORATION_ROLES,
  POLITICAL_MEMBER_HOLDER_TYPES,
  POLITICAL_MEMBER_ROLES,
  TRANSACTION_TYPES,
} from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const playerIdParams = z.object({ playerId: uuidSchema });

export const npcIdParams = z.object({ npcId: uuidSchema });

export const corporationIdParams = z.object({ corporationId: uuidSchema });

export const corporationMemberParams = corporationIdParams.extend({ playerId: uuidSchema });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/** Amounts are integer minor units. */
const amount = z.number().int().min(1).max(10_000_000_000_000);

/** Optional currency code; defaults to `credits` in the services. */
const currency = z.string().trim().min(1).max(16).optional();

/** Types trusted callers may report on credits/debits (`issuance` is mint-only). */
const INTERNAL_TYPES = TRANSACTION_TYPES.filter((t) => !['transfer', 'donation', 'tax', 'issuance'].includes(t));

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
  holderType: z.enum(CORPORATION_MEMBER_HOLDER_TYPES).optional(),
});

/** Salary amount: integer minor units, zero allowed (disables the payout). */
const salaryAmount = z.number().int().min(0).max(10_000_000_000_000);

export const corporationRoleParams = corporationIdParams.extend({
  role: z.enum(CORPORATION_ROLES),
});

export const roleSalaryBody = z.object({
  amount: salaryAmount,
  currency,
  enabled: z.boolean().optional(),
});

export const memberSalaryBody = z.object({
  amount: salaryAmount,
  currency,
  enabled: z.boolean().optional(),
});

export const primeBody = z.object({
  amount,
  currency,
  memo: z.string().trim().max(256).optional(),
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

/** Admin analytics window and ranking size. */
export const statsQuery = z.object({
  days: z.coerce.number().int().min(1).max(365).default(30),
  top: z.coerce.number().int().min(1).max(100).default(10),
});

/** Admin player search by pseudonym. */
export const playerSearchQuery = z.object({
  search: z.string().trim().min(1).max(64),
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

// ── Politics, taxes & minting ───────────────────────────────────────────────

export const politicalEntityIdParams = z.object({ entityId: uuidSchema });

export const politicalMemberParams = politicalEntityIdParams.extend({ playerId: uuidSchema });

export const politicalMemberRoleBody = z.object({
  role: z.enum(POLITICAL_MEMBER_ROLES),
  holderType: z.enum(POLITICAL_MEMBER_HOLDER_TYPES).optional(),
});

export const politicalSettingsBody = z
  .object({
    corporateTaxBps: z.number().int().min(0).max(10_000).optional(),
    incomeTaxBps: z.number().int().min(0).max(10_000).optional(),
    allowMinting: z.boolean().optional(),
    mintCeiling: z.number().int().min(0).max(10_000_000_000_000).optional(),
  })
  .refine((v) => Object.keys(v).length > 0, { message: 'At least one field is required' });

/** Trigger a tax assessment (books debts). */
export const assessBody = z.object({ currency });

/** Create currency and credit it to a political treasury. */
export const mintBody = z.object({
  amount,
  currency,
  reason: z.string().trim().max(128).optional(),
});

/** Pay a debtor's outstanding tax debts (all due by default). */
export const payTaxesBody = z.object({
  currency,
  entityId: uuidSchema.optional(),
});

/** Attach/detach a corporation to/from a political entity (fiscal home). */
export const affiliationBody = z.object({ politicalEntityId: uuidSchema.nullable() });