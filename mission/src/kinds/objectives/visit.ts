import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/** Visit a place (`locationTo`): reported by the game server. */
export const visitKind: ObjectiveKind = {
  kind: 'visit',
  evaluation: 'game',
  quantity: true,
  paramsSchema: z.object({}).strict(),
  paramsJsonSchema: { type: 'object', properties: {}, additionalProperties: false },
  name: { en: 'Visit', fr: 'Visite' },
  summary: { en: 'Reach {locationTo} ({targetQuantity} visits; progress reported by the game server)', fr: 'Atteindre {locationTo} ({targetQuantity} visites ; progression rapportée par le serveur de jeu)' },
  categories: ['exploration', 'combat', 'construction', 'salvage', 'delivery', 'reception'],
};
