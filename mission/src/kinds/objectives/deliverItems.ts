import { z } from 'zod';

import { getStack, transferStack } from '../../services/inventory.client.js';
import { HttpError } from '../../lib/httpError.js';
import type { ObjectiveKind } from '../types.js';

const paramsSchema = z
  .object({
    itemId: z.string().trim().min(1).max(128),
    /** Where the goods are delivered (issuer's warehouse, system, another holder). */
    to: z
      .object({
        holderType: z.enum(['player', 'npc', 'corporation', 'system']),
        holderId: z.string().uuid(),
      })
      .strict(),
  })
  .strict();

/**
 * Delivery contract (Eco's "put items in container"): each measurement transfers the
 * holder's *available* goods up to the remaining quantity, directly to `to`. The
 * transfer happens at `verify` time — once moved, the delivery is terminal (a later
 * abandon does not bring the goods back).
 */
export const deliverItemsKind: ObjectiveKind = {
  kind: 'deliver_items',
  evaluation: 'service',
  quantity: true,
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['itemId', 'to'],
    properties: {
      itemId: { type: 'string', maxLength: 128 },
      to: {
        type: 'object',
        required: ['holderType', 'holderId'],
        properties: {
          holderType: { type: 'string', enum: ['player', 'npc', 'corporation', 'system'] },
          holderId: { type: 'string', format: 'uuid' },
        },
        additionalProperties: false,
      },
    },
    additionalProperties: false,
  },
  summary: 'Deliver `targetQuantity` of `itemId` to holder `to` (transferred at verify time)',
  async measure({ playerId, holderType, objective }) {
    const { itemId, to } = paramsSchema.parse(objective.params ?? {});
    const from = { holderType, holderId: playerId };
    if (from.holderType === to.holderType && from.holderId === to.holderId) {
      throw new HttpError(400, 'DELIVERY_TARGET_SELF', 'Delivery target must differ from the assignee');
    }

    const target = objective.targetQuantity;
    let progress = objective.currentProgress;
    if (progress >= target) return progress;

    const stack = await getStack(from, itemId);
    const quantity = Math.min(target - progress, stack?.available ?? 0);
    if (quantity <= 0) return progress;

    await transferStack(from, to, itemId, quantity);
    return progress + quantity;
  },
};
