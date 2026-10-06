/**
 * Routes for the authenticated player's own missions (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listPlayerMissions } from '../services/assignments.service.js';
import { listMissions } from '../services/missions.service.js';
import { assignmentListQuery, missionCreatedListQuery } from './schemas.js';

/** Router for the current player's missions. */
export const meRoutes: IRouter = Router();

/** GET /missions?status=&limit= — The player's missions (assignments joined with missions). */
meRoutes.get(
  '/missions',
  validate(assignmentListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const missions = await listPlayerMissions(
      player.id,
      req.query.status as never,
      Number(req.query.limit),
    );
    res.json({ missions });
  }),
);

/**
 * GET /missions-created?status=&limit= — Every mission the caller created (`createdBy`),
 * regardless of zone, group or free slots: the creator's management view, not the browse.
 */
meRoutes.get(
  '/missions-created',
  validate(missionCreatedListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const missions = await listMissions(
      { createdBy: player.id, status: req.query.status as never },
      Number(req.query.limit),
    );
    res.json({ missions });
  }),
);
