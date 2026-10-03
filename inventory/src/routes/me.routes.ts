/**
 * Routes for the authenticated player's own inventory (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { getHolderInventory, getStack } from '../services/inventory.service.js';

/** Router for the current player's inventory. */
export const meRoutes: IRouter = Router();

/** GET /inventory — Stacks (with held/available) and owned instances. */
meRoutes.get(
  '/inventory',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await getHolderInventory({ holderType: 'player', holderId: player.id }));
  }),
);

/** GET /inventory/stacks/:goodType — One stack of the player, with held/available. */
meRoutes.get(
  '/inventory/stacks/:goodType',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const stack = await getStack({ holderType: 'player', holderId: player.id }, req.params.goodType);
    if (!stack) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'Stack not found', status: 404 });
      return;
    }
    res.json(stack);
  }),
);
