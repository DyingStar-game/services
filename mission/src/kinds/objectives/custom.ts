import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/** Free-form objective: the game server reports whatever it verifies, details in `payload`. */
export const customKind: ObjectiveKind = {
  kind: 'custom',
  evaluation: 'game',
  quantity: true,
  paramsSchema: z.object({}).strict(),
  paramsJsonSchema: { type: 'object', properties: {}, additionalProperties: false },
  summary: 'Anything the game server can verify; free-form details in `payload` (game server reports progress)',
};
