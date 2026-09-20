/**
 * Block list routes (`/api/blocks`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import * as blocks from '../services/blocks.service.js';
import { playerIdParams, targetPlayerBody } from './schemas.js';

/** Router for the player's block list. */
export const blocksRoutes: IRouter = Router();

/** GET / — Blocked players. */
blocksRoutes.get(
  '/',
  asyncHandler(async (req, res) => {
    res.json(await blocks.listBlocks(requirePlayer(req).id));
  }),
);

/** POST / — Block a player (drops any friendship/request). */
blocksRoutes.post(
  '/',
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    await blocks.blockPlayer(requirePlayer(req).id, req.body.playerId);
    res.status(204).send();
  }),
);

/** DELETE /:playerId — Unblock a player. */
blocksRoutes.delete(
  '/:playerId',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const removed = await blocks.unblockPlayer(requirePlayer(req).id, req.params.playerId);
    if (!removed) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'Player is not blocked', status: 404 });
      return;
    }
    res.status(204).send();
  }),
);
