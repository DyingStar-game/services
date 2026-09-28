/**
 * Internal routes for the game server and trusted services (`/api/internal`). The caller is
 * authenticated by `serviceAuth` (Keycloak service account) on the mount point, then each
 * route requires its capability role.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requireServiceRole, SERVICE_ROLES } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { recordActivity } from '../services/activity.service.js';
import { recordEncounter } from '../services/encounters.service.js';
import {
  addNpcCorporationMember,
  getCorporationMembership,
  removeNpcCorporationMember,
} from '../services/corporations.service.js';
import { setPresence } from '../services/presence.service.js';
import { applyStats, ensureNpcProfile, ensureProfile, getProfile } from '../services/profiles.service.js';
import { adjustReputation, rehabilitate } from '../services/reputation.service.js';
import { listActiveSanctions } from '../services/sanctions.service.js';
import {
  activityBody,
  encounterBody,
  npcCorporationBody,
  npcProfileBody,
  playerIdParams,
  presenceBody,
  statsBody,
  upsertPlayerBody,
} from './schemas.js';

/** Router for game-server driven updates. */
export const internalRoutes: IRouter = Router();

/** PUT /players/:playerId — Ensure a profile exists at login (existing display name is kept). */
internalRoutes.put(
  '/players/:playerId',
  requireServiceRole(SERVICE_ROLES.profileWrite),
  validate(playerIdParams, 'params'),
  validate(upsertPlayerBody),
  asyncHandler(async (req, res) => {
    res.json(await ensureProfile(req.params.playerId, req.body.displayName));
  }),
);

/** PUT /players/:playerId/presence — Set status and location. */
internalRoutes.put(
  '/players/:playerId/presence',
  requireServiceRole(SERVICE_ROLES.profileWrite),
  validate(playerIdParams, 'params'),
  validate(presenceBody),
  asyncHandler(async (req, res) => {
    res.json(await setPresence(req.params.playerId, req.body.status, req.body.location));
  }),
);

/** PUT /players/:playerId/npc — Create or update a server-managed NPC profile. */
internalRoutes.put(
  '/players/:playerId/npc',
  requireServiceRole(SERVICE_ROLES.profileWrite),
  validate(playerIdParams, 'params'),
  validate(npcProfileBody),
  asyncHandler(async (req, res) => {
    res.json(await ensureNpcProfile(req.params.playerId, req.body));
  }),
);

/** PUT /players/:playerId/corporation — Add an NPC to a corporation (rank optional, default when omitted). */
internalRoutes.put(
  '/players/:playerId/corporation',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(playerIdParams, 'params'),
  validate(npcCorporationBody),
  asyncHandler(async (req, res) => {
    const member = await addNpcCorporationMember(req.body.corporationId, req.params.playerId, req.body.rankId);
    res.json({
      corporationId: member.corporationId,
      playerId: member.playerId,
      rankId: member.rankId,
      joinedAt: member.joinedAt,
    });
  }),
);

/** DELETE /players/:playerId/corporation — Remove an NPC from its corporation. */
internalRoutes.delete(
  '/players/:playerId/corporation',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const membership = await getCorporationMembership(req.params.playerId);
    if (!membership) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'NPC is not a member of any corporation', status: 404 });
      return;
    }
    await removeNpcCorporationMember(membership.corporation.id, req.params.playerId);
    res.status(204).send();
  }),
);

/** POST /players/:playerId/stats — Apply playtime/reputation deltas, level and role. */
internalRoutes.post(
  '/players/:playerId/stats',
  requireServiceRole(SERVICE_ROLES.playerWrite),
  validate(playerIdParams, 'params'),
  validate(statsBody),
  asyncHandler(async (req, res) => {
    const { reputationDelta, reputationReason, ...stats } = req.body;
    const profile = Object.keys(stats).length ? await applyStats(req.params.playerId, stats) : await getProfile(req.params.playerId);
    if (reputationDelta) {
      const reputation = await adjustReputation(req.params.playerId, reputationDelta, 'game', reputationReason ?? 'Game event');
      res.json({ ...profile, reputation });
      return;
    }
    res.json(profile);
  }),
);

/** POST /players/:playerId/activity — Append a game activity entry. */
internalRoutes.post(
  '/players/:playerId/activity',
  requireServiceRole(SERVICE_ROLES.playerWrite),
  validate(playerIdParams, 'params'),
  validate(activityBody),
  asyncHandler(async (req, res) => {
    await recordActivity(req.params.playerId, req.body.type, req.body.details);
    res.status(204).send();
  }),
);

/** GET /players/:playerId/corporation — Corporation and rank of a player (null if corporationless). */
internalRoutes.get(
  '/players/:playerId/corporation',
  requireServiceRole(SERVICE_ROLES.corporationRead),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const membership = await getCorporationMembership(req.params.playerId);
    res.json(membership ? { ...membership.corporation, rank: membership.rank } : null);
  }),
);

/** GET /players/:playerId/sanctions — Active sanctions (for the game server to enforce mutes/bans). */
internalRoutes.get(
  '/players/:playerId/sanctions',
  requireServiceRole(SERVICE_ROLES.sanctionsRead),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await listActiveSanctions(req.params.playerId));
  }),
);

/** POST /reputation/rehabilitate — Run one rehabilitation pass now. */
internalRoutes.post(
  '/reputation/rehabilitate',
  requireServiceRole(SERVICE_ROLES.reputationWrite),
  asyncHandler(async (_req, res) => {
    res.json({ rehabilitated: await rehabilitate() });
  }),
);

/** POST /encounters — Record that two players met. */
internalRoutes.post(
  '/encounters',
  requireServiceRole(SERVICE_ROLES.reputationWrite),
  validate(encounterBody),
  asyncHandler(async (req, res) => {
    await recordEncounter(req.body.playerId, req.body.otherPlayerId);
    res.status(204).send();
  }),
);
