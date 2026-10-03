/**
 * Corporation inventory reads (`/api/corporations`). Corporation membership is
 * authoritative in the Social service; this service delegates the check.
 */
import { Router, type IRouter } from 'express';

import { env } from '../config/env.js';
import { asyncHandler } from '../lib/asyncHandler.js';
import { HttpError } from '../lib/httpError.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { getHolderInventory, getStack } from '../services/inventory.service.js';
import { isSocialConfigured, isCorporationMember } from '../services/social.client.js';
import { corporationParams, corporationStackParams } from './schemas.js';

/** Router mounted at `/api/corporations`. */
export const corporationsRoutes: IRouter = Router();

/**
 * Requires that the caller may read a corporation's inventory.
 * @param corporationId - Corporation id.
 * @param playerId - Caller player id.
 */
async function requireCorporationAccess(corporationId: string, playerId: string): Promise<void> {
  if (!isSocialConfigured()) {
    // Local dev without Social: trust the authenticated player.
    if (env.authDevBypass) return;
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const member = await isCorporationMember(playerId, corporationId);
  if (!member) throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'You are not a member of this corporation');
}

/** GET /:corporationId/inventory — Corporation inventory (member only). */
corporationsRoutes.get(
  '/:corporationId/inventory',
  validate(corporationParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationAccess(corporationId, player.id);
    res.json(await getHolderInventory({ holderType: 'corporation', holderId: corporationId }));
  }),
);

/** GET /:corporationId/inventory/stacks/:goodType — One corporation stack (member only). */
corporationsRoutes.get(
  '/:corporationId/inventory/stacks/:goodType',
  validate(corporationStackParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationAccess(corporationId, player.id);
    const stack = await getStack({ holderType: 'corporation', holderId: corporationId }, req.params.goodType);
    if (!stack) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'Stack not found', status: 404 });
      return;
    }
    res.json(stack);
  }),
);
