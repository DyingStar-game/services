import { z } from 'zod';

import { getPlayerProfile } from '../../services/social.client.js';
import type { PrerequisiteKind } from '../types.js';

const paramsSchema = z
  .object({ min: z.number().int().min(0).max(1_000_000_000) })
  .strict();

/** Reputation gate: the player's social reputation must reach `min`. */
export const minReputationKind: PrerequisiteKind = {
  kind: 'min_reputation',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['min'],
    properties: { min: { type: 'integer', minimum: 0 } },
    additionalProperties: false,
  },
  summary: 'Player reputation (Social) is at least `min`',
  async check({ playerId, params }) {
    const { min } = paramsSchema.parse(params);
    const profile = await getPlayerProfile(playerId);
    if (!profile) return { ok: false, detail: 'profile not found' };
    if (profile.reputation < min) {
      return { ok: false, detail: `reputation ${profile.reputation} < ${min}` };
    }
    return { ok: true };
  },
};
