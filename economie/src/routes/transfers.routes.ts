/**
 * Direct player-to-player transfers (`/api/transfers`). Automatic tax is applied and
 * collected by the system vault (see `ECONOMY_TRANSFER_TAX_BPS`).
 */
import { Router, type IRouter } from 'express';
import { t } from '../i18n/index.js';

import { env } from '../config/env.js';
import { asyncHandler } from '../lib/asyncHandler.js';
import { HttpError } from '../lib/httpError.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { ensurePlayerAccount } from '../services/accounts.service.js';
import { notifyPlayer } from '../services/social.client.js';
import { transfer } from '../services/transactions.service.js';
import { transferBody } from './schemas.js';

/** Router mounted at `/api/transfers`. */
export const transfersRoutes: IRouter = Router();

/** POST / — Move credits to another player (automatic tax, payer-side). */
transfersRoutes.post(
  '/',
  validate(transferBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { toPlayerId, amount, memo } = req.body;

    if (toPlayerId === player.id) {
      throw new HttpError(400, 'INVALID_TARGET', t('target.self'));
    }
    if (amount < env.economy.minTransfer) {
      throw new HttpError(400, 'INVALID_AMOUNT', t('amount.min', { min: env.economy.minTransfer }));
    }
    if (env.economy.maxTransfer > 0 && amount > env.economy.maxTransfer) {
      throw new HttpError(400, 'INVALID_AMOUNT', t('amount.max', { max: env.economy.maxTransfer }));
    }

    const [fromAccount, toAccount] = await Promise.all([
      ensurePlayerAccount(player.id),
      ensurePlayerAccount(toPlayerId),
    ]);

    const result = await transfer({
      fromAccountId: fromAccount.id,
      toAccountId: toAccount.id,
      amount,
      type: 'transfer',
      reference: 'player_transfer',
      details: memo ? { memo } : undefined,
    });

    // Ephemeral push to the receiver (best-effort: a notification failure never
    // fails the transfer — the ledger is the source of truth anyway).
    void notifyPlayer(toPlayerId, {
      type: 'transfer_received',
      data: {
        amount,
        currency: result.transaction.currency,
        fromPlayerId: player.id,
        ...(memo ? { memo } : {}),
      },
    }).catch((err: Error) => console.warn(`[transfers] notify failed: ${err.message}`));

    res.status(201).json(result);
  }),
);