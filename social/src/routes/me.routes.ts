/**
 * Routes for the authenticated player's own profile (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listActivity } from '../services/activity.service.js';
import { getPresence } from '../services/presence.service.js';
import { ensureProfile, updateProfile } from '../services/profiles.service.js';
import { limitQuery, profilePatchBody } from './schemas.js';

/** Router for the current player's profile and history. */
export const meRoutes: IRouter = Router();

/** GET / — Own profile (created on first call) with presence. */
meRoutes.get(
  '/',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const profile = await ensureProfile(player.id, player.username);
    const presence = await getPresence(player.id);
    res.json({ ...profile, presence });
  }),
);

/** PATCH / — Update editable profile fields. */
meRoutes.patch(
  '/',
  validate(profilePatchBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await ensureProfile(player.id, player.username);
    res.json(await updateProfile(player.id, req.body));
  }),
);

/** GET /activity — Own activity history (newest first). */
meRoutes.get(
  '/activity',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listActivity(player.id, Number(req.query.limit)));
  }),
);
