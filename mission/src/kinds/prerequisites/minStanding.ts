import { z } from 'zod';
import { t } from '../../i18n/index.js';

import { STANDING_DOMAINS } from '../../db/schema/index.js';
import { HttpError } from '../../lib/httpError.js';
import { getStanding } from '../../services/social.client.js';
import type { PrerequisiteKind } from '../types.js';

const paramsSchema = z
  .object({
    /** Standing domain: one of the mission categories, or `general` (derived RMS). */
    domain: z.enum(STANDING_DOMAINS),
    min: z.number().int().min(-1_000_000).max(1_000_000),
  })
  .strict();

/**
 * Character standing gate: the player's standing in a domain (Social) must reach `min`.
 * `general` is the derived RMS across all domains. A missing domain counts as 0.
 */
export const minStandingKind: PrerequisiteKind = {
  kind: 'min_standing',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['domain', 'min'],
    properties: {
      domain: {
        type: 'string',
        enum: [...STANDING_DOMAINS],
        description: 'Standing domain; `general` is derived (RMS of the five categories)',
      },
      min: { type: 'integer', minimum: -1_000_000, maximum: 1_000_000 },
    },
    additionalProperties: false,
  },
  name: { en: 'Minimum standing', fr: 'Standing minimal' },
  summary: {
    en: 'Player standing in {domain} (Social) is at least {min}',
    fr: 'Le standing du joueur dans {domain} (Social) est au moins de {min}',
  },
  categories: 'all',
  async check({ playerId, params }) {
    const { domain, min } = paramsSchema.parse(params);
    let standing: Record<string, number>;
    try {
      standing = await getStanding(playerId);
    } catch (err) {
      // A profile that does not exist counts as standing 0 (prereq fails cleanly), any
      // other Social failure bubbles as 502.
      if (err instanceof HttpError && err.params?.status === 404) standing = {};
      else throw err;
    }
    const current = standing[domain] ?? 0;
    if (current < min) {
      return { ok: false, detail: t('prereq.detail.standing', { standing: current, domain, min }) };
    }
    return { ok: true };
  },
};
