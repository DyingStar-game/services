import { z } from 'zod';
import { t } from '../../i18n/index.js';

import { getStack } from '../../services/inventory.client.js';
import type { PrerequisiteKind } from '../types.js';

const paramsSchema = z
  .object({
    itemId: z.string().trim().min(1).max(128),
    quantity: z.number().int().min(1).max(1_000_000_000),
    /** `available` ignores reserved goods (holds), `total` counts them. */
    scope: z.enum(['available', 'total']).default('available'),
  })
  .strict();

/** Inventory gate: the player must hold at least `quantity` of `itemId`. */
export const ownsItemsPrerequisite: PrerequisiteKind = {
  kind: 'owns_items',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['itemId', 'quantity'],
    properties: {
      itemId: { type: 'string', maxLength: 128 },
      quantity: { type: 'integer', minimum: 1 },
      scope: { type: 'string', enum: ['available', 'total'], default: 'available' },
    },
    additionalProperties: false,
  },
  name: { en: 'Required items', fr: 'Objets requis' },
  summary: { en: 'Player inventory holds at least {quantity} of {itemId}', fr: "L'inventaire du joueur détient au moins {quantity} de {itemId}" },
  categories: ['mining', 'farming', 'crafting', 'construction', 'trading', 'salvage', 'delivery'],
  async check({ playerId, params }) {
    const { itemId, quantity, scope } = paramsSchema.parse(params);
    const stack = await getStack({ holderType: 'player', holderId: playerId }, itemId);
    const held = scope === 'total' ? (stack?.quantity ?? 0) : (stack?.available ?? 0);
    if (held < quantity) {
      return {
      ok: false,
      detail: t('prereq.detail.owns', {
        scope: scope === 'available' ? 'disponible' : 'total',
        itemId,
        held,
        quantity,
      }),
    };
    }
    return { ok: true };
  },
};
