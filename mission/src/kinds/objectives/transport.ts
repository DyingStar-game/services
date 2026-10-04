import { z } from 'zod';

import { d } from '../schemaI18n.js';
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
    properties: { itemId: { type: 'string', maxLength: 128, description: d('Inventory good type to move', 'Type de bien Inventory à transporter') } },
    additionalProperties: false,
  },
  summary: { en: 'Transport a quantity of {itemId} from {locationFrom} to {locationTo} (game server)', fr: 'Transporter une quantité de {itemId} de {locationFrom} à {locationTo} (serveur de jeu)' },
  categories: ['transport', 'delivery', 'trading', 'reception'],
};
