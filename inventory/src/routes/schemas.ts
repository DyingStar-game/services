/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import { GOOD_KINDS, HOLD_KINDS, HOLD_REF_TYPES, HOLDER_TYPES, INSTANCE_STATUSES } from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const holderParams = z.object({
  holderType: z.enum(HOLDER_TYPES),
  holderId: uuidSchema,
});

export const holderStackParams = holderParams.extend({
  goodType: z.string().trim().min(1).max(128),
});

export const corporationParams = z.object({ corporationId: uuidSchema });

export const corporationStackParams = corporationParams.extend({
  goodType: z.string().trim().min(1).max(128),
});

export const instanceParams = holderParams.extend({
  instanceId: uuidSchema,
});

/** Good type key; opaque string defined by the game/market catalog. */
const goodType = z.string().trim().min(1).max(128);

/** Quantities are positive integers. */
const quantity = z.number().int().min(1).max(10_000_000_000_000);

export const creditBody = z.object({
  goodType,
  quantity,
});

export const registerInstanceBody = z.object({
  instanceId: uuidSchema,
  goodType,
  metadata: z.record(z.string(), z.unknown()).optional(),
});

export const instanceStatusBody = z.object({
  status: z.enum(INSTANCE_STATUSES),
});

export const transferStackBody = z.object({
  from: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  to: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  goodType,
  quantity,
});

export const transferInstanceBody = z.object({
  from: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  to: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  instanceId: uuidSchema,
});

export const createHoldBody = z
  .object({
    kind: z.enum(HOLD_KINDS),
    goodType,
    quantity: quantity.optional(),
    instanceId: uuidSchema.optional(),
    refType: z.enum(HOLD_REF_TYPES),
    refId: z.string().trim().max(128).optional(),
    expiresAt: z.coerce.date().optional(),
    details: z.record(z.string(), z.unknown()).optional(),
  })
  .refine((v) => (v.kind === 'stack' ? v.quantity !== undefined : v.instanceId !== undefined), {
    message: 'stack holds need quantity; instance holds need instanceId',
  });

export const consumeHoldBody = z.object({
  to: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
});

export const holdIdParams = z.object({ holdId: uuidSchema });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/** Bodies reused by player routes (holder derived from the JWT). */
export const playerCreditBody = creditBody;
export const playerRegisterInstanceBody = registerInstanceBody;
export const playerHoldBody = createHoldBody;

export type CreateHoldBody = z.infer<typeof createHoldBody>;
export { GOOD_KINDS };
