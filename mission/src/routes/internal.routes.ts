/**
 * Internal routes for the game server and trusted services (`/api/internal`). The caller is
 * authenticated by `serviceAuth` (Keycloak service account) on the mount point, then each
 * route requires its capability role. This is the authoritative channel: the game server
 * creates missions, reports verified objective progress and completes them.
 */
import { Router, type IRouter } from 'express';
import { t } from '../i18n/index.js';

import { asyncHandler } from '../lib/asyncHandler.js';
import { HttpError } from '../lib/httpError.js';
import { requireService, requireServiceRole, SERVICE_ROLES } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import {
  acceptMission,
  completeMission,
  getAssignment,
  listAssignmentsForPlayer,
  listMissionsByIds,
  reportProgress,
  verifyPlayerObjectives,
} from '../services/assignments.service.js';
import {
  cancelMission,
  confirmObjective,
  createMission,
  expireDueMissions,
  getMissionById,
  listMissions,
  setMissionEventInternal,
  shareMission,
  unshareMission,
  updateMission,
} from '../services/missions.service.js';
import { settleMissionRewards } from '../services/rewards.service.js';
import {
  createMissionBody,
  eventBody,
  internalAssignBody,
  internalCompleteBody,
  internalProgressBody,
  missionIdParams,
  limitQuery,
  missionListQuery,
  objectiveParams,
  playerIdParams,
  settleBody,
  shareBody,
  internalVerifyBody,
  updateMissionBody,
} from './schemas.js';

/** Router for game-server driven mission updates. */
export const internalRoutes: IRouter = Router();

// ── Mission catalogue ────────────────────────────────────────────────────────

/** POST /missions — Create a mission (system, IA corporation/city or scenario). */
internalRoutes.post(
  '/missions',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(createMissionBody),
  asyncHandler(async (req, res) => {
    const created = await createMission(req.body, requireService(req).clientId);
    res.status(201).json(created);
  }),
);

/** GET /missions?limit=&offset= — Full mission catalogue with filters (any status). */
internalRoutes.get(
  '/missions',
  requireServiceRole(SERVICE_ROLES.missionRead),
  validate(missionListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const found = await listMissions(
      {
        status: req.query.status as never,
        kind: req.query.kind as never,
        category: req.query.category as never,
        issuerType: req.query.issuerType as never,
        issuerId: req.query.issuerId as string | undefined,
        visibility: req.query.visibility as never,
        groupId: req.query.groupId as string | undefined,
        groupClaimable: req.query.groupClaimable as never,
        isEvent: req.query.isEvent as never,
      },
      Number(req.query.limit),
      Number(req.query.offset),
    );
    res.json({ missions: found.items, total: found.total, limit: found.limit, offset: found.offset });
  }),
);

/** POST /missions/expire — Expire every mission past its deadline (scheduled job). */
internalRoutes.post(
  '/missions/expire',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  asyncHandler(async (_req, res) => {
    res.json({ expired: await expireDueMissions() });
  }),
);

/** PATCH /missions/:missionId — Update a mission's mutable fields. */
internalRoutes.patch(
  '/missions/:missionId',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(missionIdParams, 'params'),
  validate(updateMissionBody),
  asyncHandler(async (req, res) => {
    res.json(await updateMission(req.params.missionId, req.body));
  }),
);

/** POST /missions/:missionId/cancel — Cancel a mission and its active assignments. */
internalRoutes.post(
  '/missions/:missionId/cancel',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await cancelMission(req.params.missionId));
  }),
);

/** POST /missions/:missionId/share — Share a mission with a group (game server, trusted). */
internalRoutes.post(
  '/missions/:missionId/share',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(missionIdParams, 'params'),
  validate(shareBody),
  asyncHandler(async (req, res) => {
    res.json(await shareMission(req.params.missionId, req.body.groupId, { trusted: true }));
  }),
);

/** DELETE /missions/:missionId/share — Remove the group share (game server, trusted). */
internalRoutes.delete(
  '/missions/:missionId/share',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await unshareMission(req.params.missionId, { trusted: true }));
  }),
);

/** POST /missions/:missionId/event — Declare/lift the "big event" flag (game server). */
internalRoutes.post(
  '/missions/:missionId/event',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(missionIdParams, 'params'),
  validate(eventBody),
  asyncHandler(async (req, res) => {
    res.json(await setMissionEventInternal(req.params.missionId, req.body));
  }),
);

// ── Objective progress ───────────────────────────────────────────────────────

/** POST /missions/:missionId/objectives/:objectiveId/progress — Verified progress report. */
internalRoutes.post(
  '/missions/:missionId/objectives/:objectiveId/progress',
  requireServiceRole(SERVICE_ROLES.missionProgress),
  validate(objectiveParams, 'params'),
  validate(internalProgressBody),
  asyncHandler(async (req, res) => {
    const { playerId, quantity } = req.body;
    res.json(await reportProgress(req.params.missionId, playerId, req.params.objectiveId, quantity));
  }),
);

/** POST /missions/:missionId/objectives/:objectiveId/confirm — Confirm an issuer-verified objective (game server). */
internalRoutes.post(
  '/missions/:missionId/objectives/:objectiveId/confirm',
  requireServiceRole(SERVICE_ROLES.missionProgress),
  validate(objectiveParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await confirmObjective(req.params.missionId, req.params.objectiveId));
  }),
);

/** POST /missions/:missionId/verify — Re-measure service objectives for a holder (game server). */
internalRoutes.post(
  '/missions/:missionId/verify',
  requireServiceRole(SERVICE_ROLES.missionProgress),
  validate(missionIdParams, 'params'),
  validate(internalVerifyBody),
  asyncHandler(async (req, res) => {
    res.json(await verifyPlayerObjectives(req.params.missionId, req.body.playerId));
  }),
);

// ── Completion & rewards ─────────────────────────────────────────────────────

/** POST /missions/:missionId/assign — Assign a mission to a holder (player or NPC). */
internalRoutes.post(
  '/missions/:missionId/assign',
  requireServiceRole(SERVICE_ROLES.missionWrite),
  validate(missionIdParams, 'params'),
  validate(internalAssignBody),
  asyncHandler(async (req, res) => {
    const { playerId, holderType } = req.body;
    res.status(201).json(await acceptMission(req.params.missionId, playerId, holderType));
  }),
);

/** POST /missions/:missionId/complete — Complete a player's mission and settle its reward. */
internalRoutes.post(
  '/missions/:missionId/complete',
  requireServiceRole(SERVICE_ROLES.missionComplete),
  validate(missionIdParams, 'params'),
  validate(internalCompleteBody),
  asyncHandler(async (req, res) => {
    const { playerId, force, settle } = req.body;
    res.json(
      await completeMission(req.params.missionId, playerId, {
        force,
        settle,
      }),
    );
  }),
);

/** POST /missions/:missionId/settle — Replay unsettled reward shares (idempotent). */
internalRoutes.post(
  '/missions/:missionId/settle',
  requireServiceRole(SERVICE_ROLES.missionComplete),
  validate(missionIdParams, 'params'),
  validate(settleBody),
  asyncHandler(async (req, res) => {
    const mission = await getMissionById(req.params.missionId);
    if (!mission) throw new HttpError(404, 'NOT_FOUND', t('not_found.mission_bare'));
    const assignment = await getAssignment(req.params.missionId, req.body.playerId);
    if (!assignment) throw new HttpError(404, 'NOT_FOUND', t('not_found.assignment'));
    if (assignment.status !== 'completed') {
      throw new HttpError(409, 'ASSIGNMENT_NOT_COMPLETED', `Assignment is ${assignment.status}`);
    }
    res.json({ settlements: await settleMissionRewards(mission, { force: true }) });
  }),
);

/** GET /players/:playerId/missions?limit=&offset= — A page of a player's assignments (with their missions). */
internalRoutes.get(
  '/players/:playerId/missions',
  requireServiceRole(SERVICE_ROLES.missionRead),
  validate(playerIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const limit = Number(req.query.limit);
    const offset = Number(req.query.offset);
    const found = await listAssignmentsForPlayer(req.params.playerId, limit, offset);
    const referenced = await listMissionsByIds(found.items.map((a) => a.missionId));
    const byId = new Map(referenced.map((m) => [m.id, m]));
    res.json({
      missions: found.items.map((assignment) => ({
        assignment,
        mission: byId.get(assignment.missionId) ?? null,
      })),
      total: found.total,
      limit,
      offset,
    });
  }),
);
