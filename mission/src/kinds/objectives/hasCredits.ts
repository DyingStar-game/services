import { z } from 'zod';

import { getWalletAccounts } from '../../services/economy.client.js';
import type { ObjectiveKind } from '../types.js';

const paramsSchema = z.object({ currency: z.string().trim().min(1).max(16).default('credits') }).strict();

/**
 * Snapshot of the holder's wallet: measured against Economy (`targetQuantity` is the
 * amount required), so the progression can regress if the holder spends before
 * completion (completion re-verifies).
 */
export const hasCreditsKind: ObjectiveKind = {
  kind: 'has_credits',
  evaluation: 'service',
  quantity: true,
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    properties: { currency: { type: 'string', maxLength: 16, default: 'credits' } },
    additionalProperties: false,
  },
  summary: 'Holder wallet balance reaches `targetQuantity` of `currency` (measured against Economy)',
  async measure({ playerId, holderType, objective }) {
    const { currency } = paramsSchema.parse(objective.params ?? {});
    const accounts = await getWalletAccounts(holderType, playerId);
    return accounts.find((account) => account.currency === currency)?.balance ?? 0;
  },
};
