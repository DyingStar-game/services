import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/** Visit a place (`locationTo`): reported by the game server. */
export const visitKind: ObjectiveKind = {
  kind: 'visit',
  evaluation: 'game',
  quantity: true,
  paramsSchema: z.object({}).strict(),
  paramsJsonSchema: { type: 'object', properties: {}, additionalProperties: false },
  summary: 'Reach `locationTo` (targetQuantity visits; progress reported by the game server)',
};
