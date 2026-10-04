import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/** Deliver a material at a place: reported by the game server. */
export const deliverMaterialKind: ObjectiveKind = {
  kind: 'deliver_material',
  evaluation: 'game',
  quantity: true,
  paramsSchema: z.object({ itemId: z.string().trim().min(1).max(128) }).strict(),
  paramsJsonSchema: {
    type: 'object',
    required: ['itemId'],
    properties: { itemId: { type: 'string', maxLength: 128, description: 'Inventory good type to deliver' } },
    additionalProperties: false,
  },
  summary: { en: 'Deliver a quantity of {itemId} at {locationTo} (progress reported by the game server)', fr: 'Livrer une quantité de {itemId} à {locationTo} (progression rapportée par le serveur de jeu)' },
  categories: ['delivery', 'transport', 'mining', 'farming', 'crafting', 'trading', 'salvage', 'reception'],
};
