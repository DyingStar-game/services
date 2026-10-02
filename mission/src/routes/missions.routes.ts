/**
 * Player-facing mission routes (`/api/missions`): browse available missions, accept one,
 * report objective progress, complete (reward) or abandon.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { HttpError } from '../lib/httpError.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import {
  abandonMission,
  acceptMission,
  completeMission,
  getAssignment,
  reportProgress,
} from '../services/assignments.service.js';
import {
  createPlayerMission,
  getMissionWithObjectives,
  listMissions,
} from '../services/missions.service.js';
import {
  createPlayerMissionBody,
  missionIdParams,
  missionListQuery,
  objectiveParams,
  progressBody,
} from './schemas.js';

/** Router mounted at `/api/missions`. */
export const missionsRoutes: IRouter = Router();

/** POST / — Create a player-sponsored mission (economic reward escrowed from the caller). */
missionsRoutes.post(
  '/',
  validate(createPlayerMissionBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId, ...input } = req.body;
    const created = await createPlayerMission(
      { ...input, issuerId: corporationId ?? null },
      player.id,
    );
    res.status(201).json(created);
  }),
);

/** GET / — Browse missions (defaults to open, `available` ones). */
missionsRoutes.get(
  '/',
  validate(missionListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const filters = {
      status: req.query.status as never,
      kind: req.query.kind as never,
      category: req.query.category as never,
      issuerType: req.query.issuerType as never,
      issuerId: req.query.issuerId as string | undefined,
      visibility: req.query.visibility as never,
    };
    if (!filters.status) filters.status = 'available' as never;
    res.json({ missions: await listMissions(filters, Number(req.query.limit)) });
  }),
);

/** GET /:missionId — Mission detail with its objectives and the caller's assignment. */
missionsRoutes.get(
  '/:missionId',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const result = await getMissionWithObjectives(req.params.missionId);
    if (!result) throw new HttpError(404, 'NOT_FOUND', 'Mission not found');
    const assignment = await getAssignment(req.params.missionId, player.id);
    res.json({ ...result, assignment });
  }),
);

/** POST /:missionId/accept — Accept the mission (creates the player's assignment). */
missionsRoutes.post(
  '/:missionId/accept',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.status(201).json(await acceptMission(req.params.missionId, player.id));
  }),
);

/** POST /:missionId/abandon — Abandon the current assignment. */
missionsRoutes.post(
  '/:missionId/abandon',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const assignment = await abandonMission(req.params.missionId, player.id);
    res.json({ assignment });
  }),
);

/** POST /:missionId/objectives/:objectiveId/progress — Report objective progress. */
missionsRoutes.post(
  '/:missionId/objectives/:objectiveId/progress',
  validate(objectiveParams, 'params'),
  validate(progressBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const result = await reportProgress(
      req.params.missionId,
      player.id,
      req.params.objectiveId,
      req.body.quantity,
    );
    res.json(result);
  }),
);

/** POST /:missionId/complete — Complete the mission and settle its reward. */
missionsRoutes.post(
  '/:missionId/complete',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await completeMission(req.params.missionId, player.id));
  }),
);
