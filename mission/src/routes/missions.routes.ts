/**
 * Player-facing mission routes (`/api/missions`): browse available missions, accept one,
 * report objective progress, complete (reward) or abandon.
 */
import { Router, type IRouter } from 'express';
import { t } from '../i18n/index.js';

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
  verifyPlayerObjectives,
} from '../services/assignments.service.js';
import {
  confirmObjective,
  createPlayerMission,
  getMissionWithObjectives,
  listMissions,
  requireMission,
  setMissionEvent,
  shareMission,
  unshareMission,
} from '../services/missions.service.js';
import { isGroupMember, getPlayerGroup, getPlayerPresence } from '../services/social.client.js';
import { validateMissionSpec } from '../services/spec.service.js';
import { missionKindsCatalog } from '../kinds/index.js';
import {
  createPlayerMissionBody,
  eventBody,
  missionIdParams,
  missionListQuery,
  objectiveParams,
  progressBody,
  shareBody,
  validateMissionBody,
} from './schemas.js';

/** Router mounted at `/api/missions`. */
export const missionsRoutes: IRouter = Router();

/** POST / — Create a player-sponsored mission (reward escrowed from the caller). */
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

/** POST /validate — Dry-run a mission spec (no persistence, no escrow). */
missionsRoutes.post(
  '/validate',
  validate(validateMissionBody),
  asyncHandler(async (req, res) => {
    const { mode, mission } = req.body;
    const spec = validateMissionSpec(
      {
        category: mission.category,
        objectives: mission.objectives,
        prerequisites: mission.prerequisites,
        rewards: mission.rewards,
        maxAssignees: mission.maxAssignees,
      },
      { escrowed: mode === 'player', requireReward: mode === 'player' },
    );
    res.json({ valid: true, spec });
  }),
);

/** GET /kinds — Discovery catalogue: categories, objective/prerequisite kinds, reward shape. */
missionsRoutes.get('/kinds', asyncHandler(async (_req, res) => {
  res.json(missionKindsCatalog());
}));

/** GET /?limit=&offset= — Browse missions: open (`available`/`active`) with a free slot, group-aware. */
missionsRoutes.get(
  '/',
  validate(missionListQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const filters = {
      kind: req.query.kind as never,
      category: req.query.category as never,
      issuerType: req.query.issuerType as never,
      issuerId: req.query.issuerId as string | undefined,
      visibility: req.query.visibility as never,
      groupId: req.query.groupId as string | undefined,
      groupClaimable: req.query.groupClaimable as never,
      isEvent: req.query.isEvent as never,
      hasFreeSlots: true as const,
      statuses: (req.query.status
        ? [req.query.status as never]
        : ['available', 'active']) as never,
    };
    // Group-shared missions are only listed to their members, and zoned missions only to
    // players inside one of their zones (both lookups fail-soft: a Social outage degrades
    // to "no group, no location" instead of breaking the browse).
    const [groupMembership, presence] = await Promise.all([
      getPlayerGroup(player.id).catch(() => null),
      getPlayerPresence(player.id).catch(() => null),
    ]);
    const viewerGroupId = groupMembership?.group.id ?? null;
    const viewerLocation = presence?.location ?? null;
    const found = await listMissions(
      { ...filters, viewerGroupId, viewerLocation },
      Number(req.query.limit),
      Number(req.query.offset),
    );
    res.json({ missions: found.items, total: found.total, limit: found.limit, offset: found.offset });
  }),
);

/** GET /:missionId — Mission detail with its objectives and the caller's assignment. */
missionsRoutes.get(
  '/:missionId',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const result = await getMissionWithObjectives(req.params.missionId);
    if (!result) throw new HttpError(404, 'NOT_FOUND', t('not_found.mission_bare'));
    if (result.mission.groupId && !(await isGroupMember(player.id, result.mission.groupId))) {
      // Hidden from players outside the target group.
      throw new HttpError(404, 'NOT_FOUND', t('not_found.mission_bare'));
    }
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

/** POST /:missionId/share — Share the mission with a group (creator or corporation member). */
missionsRoutes.post(
  '/:missionId/share',
  validate(missionIdParams, 'params'),
  validate(shareBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await shareMission(req.params.missionId, req.body.groupId, { actorId: player.id }));
  }),
);

/** DELETE /:missionId/share — Remove the group share (creator or corporation member). */
missionsRoutes.delete(
  '/:missionId/share',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await unshareMission(req.params.missionId, { actorId: player.id }));
  }),
);

/** POST /:missionId/event — Declare/lift the "big event" flag (corporation mission only). */
missionsRoutes.post(
  '/:missionId/event',
  validate(missionIdParams, 'params'),
  validate(eventBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await setMissionEvent(req.params.missionId, player.id, req.body));
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

/** POST /:missionId/verify — Re-measure the mission's service objectives (and perform deliveries). */
missionsRoutes.post(
  '/:missionId/verify',
  validate(missionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await verifyPlayerObjectives(req.params.missionId, player.id));
  }),
);

/** POST /:missionId/objectives/:objectiveId/confirm — Confirm an issuer-verified objective (mission creator). */
missionsRoutes.post(
  '/:missionId/objectives/:objectiveId/confirm',
  validate(objectiveParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const mission = await requireMission(req.params.missionId);
    if (mission.createdBy !== player.id) {
      throw new HttpError(403, 'NOT_MISSION_ISSUER', 'Only the creator of the mission can confirm its objectives');
    }
    res.json(await confirmObjective(req.params.missionId, req.params.objectiveId));
  }),
);
