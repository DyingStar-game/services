/**
 * Political entity POI reads (`/api/politics`). Membership is authoritative in the
 * Social service; this service delegates the check (any office may read).
 */
import { Router, type IRouter } from 'express';

import { env } from '../config/env.js';
import { asyncHandler } from '../lib/asyncHandler.js';
import { HttpError } from '../lib/httpError.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { getPoiView, listOrgPois, requireOrgRead, requirePoi } from '../services/pois.service.js';
import { getPoliticalMembership, isSocialConfigured } from '../services/social.client.js';
import { limitQuery, politicalEntityParams, politicalPoiParams } from './schemas.js';

/** Router mounted at `/api/politics`. */
export const politicsRoutes: IRouter = Router();

/**
 * Requires that the caller belongs to the political entity (membership in Social).
 * @param entityId - Political entity id.
 * @param playerId - Caller player id.
 */
async function requirePoliticalAccess(entityId: string, playerId: string): Promise<void> {
  if (!isSocialConfigured()) {
    // Local dev without Social: trust the authenticated player.
    if (env.authDevBypass) return;
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const membership = await getPoliticalMembership(playerId, entityId);
  if (!membership) {
    throw new HttpError(403, 'NOT_POLITICAL_MEMBER', 'You are not a member of this political entity');
  }
}

/** GET /:entityId/pois?limit=&offset= — POIs owned by / granted to the entity (member only). */
politicsRoutes.get(
  '/:entityId/pois',
  validate(politicalEntityParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { entityId } = req.params;
    await requirePoliticalAccess(entityId, player.id);
    res.json(await listOrgPois('political', entityId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** GET /:entityId/pois/:poiId — One POI of the entity's scope (member only). */
politicsRoutes.get(
  '/:entityId/pois/:poiId',
  validate(politicalPoiParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { entityId } = req.params;
    await requirePoliticalAccess(entityId, player.id);
    const poi = await requirePoi(req.params.poiId);
    await requireOrgRead(poi, 'political', entityId);
    res.json(await getPoiView(poi.id));
  }),
);
