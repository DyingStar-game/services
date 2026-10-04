import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/** Free-form objective: the game server reports whatever it verifies, details in `payload`. */
export const customKind: ObjectiveKind = {
  kind: 'custom',
  evaluation: 'game',
  quantity: true,
  paramsSchema: z.object({}).strict(),
  paramsJsonSchema: { type: 'object', properties: {}, additionalProperties: false },
  summary: { en: 'Anything the game server can verify; free-form details in {payload} (game server reports progress)', fr: 'Tout ce que le serveur de jeu sait vérifier ; détails libres dans {payload} (le serveur de jeu rapporte la progression)' },
  categories: 'all',
};
