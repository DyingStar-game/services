/**
 * Routes for the authenticated player's own wallet (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { ensurePlayerAccount, getPlayerAccounts } from '../services/accounts.service.js';
import { listPlayerTransactions } from '../services/transactions.service.js';
import { limitQuery } from './schemas.js';

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