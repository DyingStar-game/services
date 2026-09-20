/**
 * Public-facing profile lookup routes (`/api/profiles`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { validate } from '../middleware/validate.js';
import { getPresence } from '../services/presence.service.js';
import { requireProfile, searchProfiles } from '../services/profiles.service.js';
import { playerIdParams, searchQuery } from './schemas.js';

/** Router for looking up other players. */
export const profilesRoutes: IRouter = Router();

/** GET /?search=&limit= — Search profiles by display name. */
profilesRoutes.get(
  '/',
  validate(searchQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await searchProfiles(String(req.query.search ?? ''), Number(req.query.limit)));
  }),
);

/** GET /:playerId — Public profile with online status (location is friends-only). */
profilesRoutes.get(
  '/:playerId',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const profile = await requireProfile(req.params.playerId);
    const presence = await getPresence(profile.playerId);
    res.json({ ...profile, status: presence.status });
  }),
);
