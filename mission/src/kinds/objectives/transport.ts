import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/** Move a quantity of an item from `locationFrom` to `locationTo`: reported by the game server. */
export const transportKind: ObjectiveKind = {
  kind: 'transport',
  evaluation: 'game',
  quantity: true,
  paramsSchema: z.object({ itemId: z.string().trim().min(1).max(128) }).strict(),
  paramsJsonSchema: {
    type: 'object',
    required: ['itemId'],
    properties: { itemId: { type: 'string', maxLength: 128, description: 'Inventory good type to move' } },
    additionalProperties: false,
  },
  summary: { en: 'Transport a quantity of {itemId} from {locationFrom} to {locationTo} (game server)', fr: 'Transporter une quantité de {itemId} de {locationFrom} à {locationTo} (serveur de jeu)' },
  categories: ['transport', 'delivery', 'trading', 'reception'],
};
