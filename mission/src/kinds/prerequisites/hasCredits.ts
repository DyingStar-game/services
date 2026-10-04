import { z } from 'zod';
import { t } from '../../i18n/index.js';

import { getWalletAccounts } from '../../services/economy.client.js';
import type { PrerequisiteKind } from '../types.js';

const paramsSchema = z
  .object({
    currency: z.string().trim().min(1).max(16).default('credits'),
    amount: z.number().int().min(1).max(10_000_000_000_000),
  })
  .strict();

/** Wallet balance gate: the player must hold at least `amount` of `currency`. */
export const hasCreditsPrerequisite: PrerequisiteKind = {
  kind: 'has_credits',
  paramsSchema,
  paramsJsonSchema: {
    type: 'object',
    required: ['amount'],
    properties: {
      currency: { type: 'string', maxLength: 16, default: 'credits' },
      amount: { type: 'integer', minimum: 1 },
    },
    additionalProperties: false,
  },
  summary: { en: 'Player wallet holds at least {amount} of {currency}', fr: 'Le portefeuille du joueur détient au moins {amount} de {currency}' },
  categories: ['trading', 'delivery', 'construction', 'reception', 'mining', 'farming', 'crafting'],
  async check({ playerId, params }) {
    const { currency, amount } = paramsSchema.parse(params);
    const accounts = await getWalletAccounts('player', playerId);
    const balance = accounts.find((account) => account.currency === currency)?.balance ?? 0;
    if (balance < amount) {
      return { ok: false, detail: t('prereq.detail.balance', { balance, amount, currency }) };
    }
    return { ok: true };
  },
};
