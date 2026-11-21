/**
 * Routes for the authenticated player's own missions (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listPlayerMissions } from '../services/assignments.service.js';
import { listMissions } from '../services/missions.service.js';
import { listInvitationsFor } from '../services/invitations.service.js';
import { progressOfMissions } from '../services/progress.service.js';
import { withPoiNames } from '../services/zones.service.js';
import { assignmentListQuery, missionCreatedListQuery } from './schemas.js';

/** Router for the current player's missions. */
export const meRoutes: IRouter = Router();

/** GET /mission-invitations — The invitations I may answer (mine, my group's), with their mission. */
meRoutes.get(
  '/mission-invitations',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json({ invitations: await listInvitationsFor(player.id) });
  }),
);

/**
 * GET /missions?status=&limit=&offset= — The player's missions (assignments joined with missions),
 * each with its `progress`: the step under way and how far it is, so a list can show it.
 */
meRoutes.get(
  '/missions',
  validate(assignmentListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const found = await listPlayerMissions(
      player.id,
      req.query.status as never,
      Number(req.query.limit),
      Number(req.query.offset),
    );
    const progress = await progressOfMissions(found.items.map((row) => row.mission.id));
    // The places named, as in the browse: "M0001", not a POI id.
    const named = await withPoiNames(found.items.map((row) => row.mission));
    const missions = found.items.map((row, i) => ({ ...row, mission: named[i], progress: progress.get(row.mission.id) ?? null }));
    res.json({ missions, total: found.total, limit: found.limit, offset: found.offset });
  }),
);

/**
 * GET /missions-created?status=&limit=&offset= — Every mission the caller created (`createdBy`),
 * regardless of zone, group or free slots: the creator's management view, not the browse.
 */
meRoutes.get(
  '/missions-created',
  validate(missionCreatedListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const found = await listMissions(
      { createdBy: player.id, status: req.query.status as never },
      Number(req.query.limit),
      Number(req.query.offset),
    );
    res.json({ missions: await withPoiNames(found.items), total: found.total, limit: found.limit, offset: found.offset });
  }),
);
