import { z } from 'zod';
import { t } from '../../i18n/index.js';

import { isCorporationMember } from '../../services/social.client.js';
import type { PrerequisiteKind } from '../types.js';

const paramsSchema = z.object({ corporationId: z.string().uuid() }).strict();

/** Corporation gate: the player must be a member of `corporationId` (checked in Social). */
export const corporationMemberKind: PrerequisiteKind = {
  kind: 'corporation_member',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['corporationId'],
    properties: { corporationId: { type: 'string', format: 'uuid' } },
    additionalProperties: false,
  },
  summary: { en: 'Player is a member of the given corporation', fr: 'Le joueur est membre de la corporation donnée' },
  categories: 'all',
  async check({ playerId, params }) {
    const { corporationId } = paramsSchema.parse(params);
    if (!(await isCorporationMember(playerId, corporationId))) {
      return { ok: false, detail: t('prereq.detail.not_corp', { corporationId }) };
    }
    return { ok: true };
  },
};
