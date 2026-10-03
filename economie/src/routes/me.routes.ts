/**
 * Routes for the authenticated player's own wallet (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { ensurePlayerAccount, getPlayerAccounts } from '../services/accounts.service.js';
import { getTaxDebts } from '../services/politics.service.js';
import { payTaxDebts } from '../services/taxation.service.js';
import { listPlayerTransactions } from '../services/transactions.service.js';
import { limitQuery, payTaxesBody } from './schemas.js';

/** Router for the current player's wallet. */
export const meRoutes: IRouter = Router();

/** GET /wallet — All wallet accounts of the player (one per currency, `credits` auto-created). */
meRoutes.get(
  '/wallet',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await ensurePlayerAccount(player.id);
    res.json({ accounts: await getPlayerAccounts(player.id) });
  }),
);

/** GET /wallet/transactions?limit= — Ledger of the player (all currencies, newest first). */
meRoutes.get(
  '/wallet/transactions',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listPlayerTransactions(player.id, Number(req.query.limit)));
  }),
);

/** GET /taxes — My tax debts (due and settled), newest first. */
meRoutes.get(
  '/taxes',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await getTaxDebts('player', player.id));
  }),
);

/** POST /taxes/pay — Settle all my affordable due tax debts. */
meRoutes.post(
  '/taxes/pay',
  validate(payTaxesBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await payTaxDebts('player', player.id, { currency: req.body.currency, entityId: req.body.entityId }));
  }),
);