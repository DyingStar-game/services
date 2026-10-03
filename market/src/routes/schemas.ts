/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import { GOOD_KINDS, HOLDER_TYPES, ORDER_SIDES, ORDER_STATUSES, DEMAND_STATUSES, TRADE_STATUSES } from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const orderIdParams = z.object({ id: uuidSchema });
export const tradeIdParams = z.object({ id: uuidSchema });
export const goodTypeParams = z.object({ goodType: z.string().trim().min(1).max(128) });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/** Quantities are positive integers (instance orders use 1). */
const quantity = z.number().int().min(1).max(10_000_000_000_000);
/** Prices are non-negative integer credits. */
const price = z.number().int().min(0).max(10_000_000_000_000);
const currency = z.string().trim().min(1).max(16).optional();

/** Optional corporation context; when set, the route requires membership in Social. */
const corporationId = uuidSchema.optional();

export const catalogBody = z.object({
  kind: z.enum(GOOD_KINDS),
  unit: z.string().trim().max(32).nullish(),
  displayName: z.string().trim().max(128).nullish(),
  enabled: z.boolean().optional(),
});

export const catalogEnabledBody = z.object({ enabled: z.boolean() });

export const orderBody = z
  .object({
    side: z.enum(ORDER_SIDES),
    goodType: z.string().trim().min(1).max(128),
    kind: z.enum(GOOD_KINDS),
    instanceId: uuidSchema.optional(),
    quantity,
    price,
    currency,
    corporationId,
  })
  .refine((v) => v.kind !== 'instance' || v.instanceId !== undefined, {
    message: 'instanceId is required for instance orders',
  })
  .refine((v) => !(v.kind === 'instance' && v.side === 'buy'), {
    message: 'buy orders are only supported for fungible goods',
  });

export const demandBody = z
  .object({
    goodType: z.string().trim().min(1).max(128),
    kind: z.enum(GOOD_KINDS),
    instanceId: uuidSchema.optional(),
    quantity,
    maxPrice: price,
    currency,
    message: z.string().trim().max(500).optional(),
    corporationId,
  })
  .refine((v) => v.kind !== 'instance' || v.instanceId !== undefined, {
    message: 'instanceId is required for instance demands',
  });

export const fulfillBody = z.object({
  unitPrice: price,
  corporationId,
});

export const ordersQuery = z.object({
  goodType: z.string().trim().max(128).optional(),
  side: z.enum(ORDER_SIDES).optional(),
  status: z.enum(ORDER_STATUSES).optional(),
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

export const demandsQuery = z.object({
  goodType: z.string().trim().max(128).optional(),
  status: z.enum(DEMAND_STATUSES).optional(),
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

export const tradesQuery = z.object({
  status: z.enum(TRADE_STATUSES).optional(),
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

export const bookQuery = z.object({
  goodType: z.string().trim().min(1).max(128),
});

/** Internal filters accept an explicit holder. */
export const internalOrdersQuery = ordersQuery.extend({
  holderType: z.enum(HOLDER_TYPES).optional(),
  holderId: uuidSchema.optional(),
});
export const internalDemandsQuery = demandsQuery.extend({
  holderType: z.enum(HOLDER_TYPES).optional(),
  holderId: uuidSchema.optional(),
});
export const internalTradesQuery = tradesQuery.extend({
  holderType: z.enum(HOLDER_TYPES).optional(),
  holderId: uuidSchema.optional(),
});
