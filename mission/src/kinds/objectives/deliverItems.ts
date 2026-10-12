import { z } from 'zod';

import { getStack, transferStack } from '../../services/inventory.client.js';
import { HttpError } from '../../lib/httpError.js';
import type { ObjectiveKind } from '../types.js';

/** Inventory holders a delivery may target (`warehouse` id = persistence POI uuid). */
const DELIVERY_HOLDER_TYPES = ['player', 'npc', 'corporation', 'system', 'warehouse'] as const;

const paramsSchema = z
  .object({
    itemId: z.string().trim().min(1).max(128),
    /** Where the goods are delivered (issuer's warehouse, system, another holder). */
    to: z
      .object({
        holderType: z.enum(DELIVERY_HOLDER_TYPES),
        // Any hex UUID: warehouse ids are persistence POI UUIDs without RFC version bits.
        holderId: z.guid(),
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
          holderType: { type: 'string', enum: [...DELIVERY_HOLDER_TYPES] },
          holderId: { type: 'string', format: 'uuid' },
        },
        additionalProperties: false,
      },
    },
    additionalProperties: false,
  },
  name: { en: 'Item delivery', fr: "Livraison d'objets" },
  summary: { en: 'Deliver {targetQuantity} of {itemId} to holder {to} (transferred at verify time)', fr: 'Livrer {targetQuantity} de {itemId} au détenteur {to} (transféré au moment du verify)' },
  categories: ['delivery', 'trading', 'reception', 'mining', 'farming', 'salvage'],
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
