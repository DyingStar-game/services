import { z } from 'zod';

import { getStack } from '../../services/inventory.client.js';
import type { ObjectiveKind } from '../types.js';

const paramsSchema = z
  .object({
    itemId: z.string().trim().min(1).max(128),
    /** `total` counts reserved goods, `available` ignores holds. */
    scope: z.enum(['total', 'available']).default('total'),
  })
  .strict();

/**
 * Snapshot of what the holder owns: measured against Inventory, so the progression can
 * regress if the holder spends the goods before completion (completion re-verifies).
 */
export const ownsItemsKind: ObjectiveKind = {
  kind: 'owns_items',
  evaluation: 'service',
  quantity: true,
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['itemId'],
    properties: {
      itemId: { type: 'string', maxLength: 128 },
      scope: { type: 'string', enum: ['total', 'available'], default: 'total' },
    },
    additionalProperties: false,
  },
  summary: { en: 'Holder owns {targetQuantity} of {itemId} (measured against Inventory, snapshot)', fr: 'Le détenteur possède {targetQuantity} de {itemId} (mesuré sur Inventory, instantané)' },
  categories: ['mining', 'farming', 'crafting', 'construction', 'trading', 'salvage'],
  async measure({ playerId, holderType, objective }) {
    const { itemId, scope } = paramsSchema.parse(objective.params ?? {});
    const stack = await getStack({ holderType, holderId: playerId }, itemId);
    return scope === 'available' ? (stack?.available ?? 0) : (stack?.quantity ?? 0);
  },
};
