/**
 * Internal routes for the game server and trusted services (`/api/internal`). The caller is
 * authenticated by `serviceAuth` (Keycloak service account) on the mount point, then each
 * route requires its capability role.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { HttpError } from '../lib/httpError.js';
import { requireServiceRole, SERVICE_ROLES } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { recordActivity } from '../services/activity.service.js';
import { authorize, authorizeAll, permissionCatalog } from '../services/authorize.service.js';
import { recordEncounter } from '../services/encounters.service.js';
import {
  addNpcCorporationMember,
  createCorporation,
  disbandCorporation,
  getCorporationMembership,
  listCorporationMemberships,
  removeCorporationMember,
  removeNpcCorporationMember,
  requireCorporation,
  setCorporationMemberRank,
  transferCorporationCeo,
  updateCorporation,
} from '../services/corporations.service.js';
import { getGroupMembership, getGroupSummary, getPlayerGroup } from '../services/groups.service.js';
import {
  addNpcPoliticalMember,
  createPoliticalEntity,
  disbandPoliticalEntity,
  getPoliticalMembership,
  listPoliticalMemberships,
  removeNpcPoliticalMember,
  requirePoliticalEntity,
  transferPoliticalHead,
  updatePoliticalEntity,
} from '../services/politics.service.js';
import { getPresence, setPresence, setPresenceBatch } from '../services/presence.service.js';
import {
  applyStats,
  ensureNpcProfile,
  ensureProfile,
  getProfile,
  getProfilesByIds,
  searchProfiles,
} from '../services/profiles.service.js';
import { withLivePlaytime } from '../services/playtime.service.js';
import { adjustReputation, rehabilitate } from '../services/reputation.service.js';
import { listActiveSanctions } from '../services/sanctions.service.js';
import {
  activityBody,
  authorizeBody,
  corporationIdParams,
  corporationMemberParams,
  corporationPatchBody,
  corporationQueryOptional,
  corporationQueryRequired,
  encounterBody,
  groupIdParams,
  groupQueryOptional,
  internalCreateCorporationBody,
  internalCreatePoliticalEntityBody,
  internalProfileQuery,
  memberRankBody,
  npcCorporationBody,
  npcPoliticalBody,
  npcProfileBody,
  playerIdParams,
  politicalEntityIdParams,
  politicalEntityPatchBody,
  politicalQueryOptional,
  politicalQueryRequired,
  presenceBatchBody,
  presenceBody,
  statsBody,
  targetPlayerBody,
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

/** GET /players/:playerId — Full profile (incl. reputation), or null when it does not exist. */
internalRoutes.get(
  '/players/:playerId',
  requireServiceRole(SERVICE_ROLES.profileRead),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const profile = await getProfile(req.params.playerId);
    res.json(profile ? await withLivePlaytime(profile) : null);
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

/** GET /players/:playerId/presence — Status and location of a player (default offline, location null). */
internalRoutes.get(
  '/players/:playerId/presence',
  requireServiceRole(SERVICE_ROLES.profileRead),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ playerId: req.params.playerId, ...(await getPresence(req.params.playerId)) });
  }),
);

/**
 * PUT /players/presence/batch — Batch heartbeat: refresh the TTL of every active player
 * of a game server (one call per minute instead of one per player). Players absent from
 * the call expire back to `offline` on TTL. Omitted `status` keeps a live one.
 */
internalRoutes.put(
  '/players/presence/batch',
  requireServiceRole(SERVICE_ROLES.profileWrite),
  validate(presenceBatchBody),
  asyncHandler(async (req, res) => {
    res.json(await setPresenceBatch(req.body.players));
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

/** DELETE /players/:playerId/corporation?corporationId= — Remove an NPC from one corporation. */
internalRoutes.delete(
  '/players/:playerId/corporation',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(playerIdParams, 'params'),
  validate(corporationQueryRequired, 'query'),
  asyncHandler(async (req, res) => {
    await removeNpcCorporationMember(req.query.corporationId as string, req.params.playerId);
    res.status(204).send();
  }),
);

/**
 * POST /corporations — Create a corporation with an explicit CEO (player or NPC).
 * Lets the game server seed fully-NPC corporations.
 */
internalRoutes.post(
  '/corporations',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(internalCreateCorporationBody),
  asyncHandler(async (req, res) => {
    const { ceoId, ...data } = req.body;
    res.status(201).json(await createCorporation(ceoId, data));
  }),
);

/** PATCH /corporations/:corporationId — Edit a corporation, acting as its current CEO. */
internalRoutes.patch(
  '/corporations/:corporationId',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(corporationIdParams, 'params'),
  validate(corporationPatchBody),
  asyncHandler(async (req, res) => {
    const corporation = await requireCorporation(req.params.corporationId);
    res.json(await updateCorporation(req.params.corporationId, corporation.ceoId, req.body));
  }),
);

/** DELETE /corporations/:corporationId — Disband a corporation, acting as its current CEO. */
internalRoutes.delete(
  '/corporations/:corporationId',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const corporation = await requireCorporation(req.params.corporationId);
    await disbandCorporation(req.params.corporationId, corporation.ceoId);
    res.status(204).send();
  }),
);

/** POST /corporations/:corporationId/transfer — Transfer the CEO, acting as the current CEO. */
internalRoutes.post(
  '/corporations/:corporationId/transfer',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(corporationIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    const corporation = await requireCorporation(req.params.corporationId);
    res.json(await transferCorporationCeo(req.params.corporationId, corporation.ceoId, req.body.playerId));
  }),
);

/** PATCH /corporations/:corporationId/members/:playerId — Assign a member's rank, acting as the CEO. */
internalRoutes.patch(
  '/corporations/:corporationId/members/:playerId',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(corporationMemberParams, 'params'),
  validate(memberRankBody),
  asyncHandler(async (req, res) => {
    const corporation = await requireCorporation(req.params.corporationId);
    res.json(
      await setCorporationMemberRank(
        req.params.corporationId,
        corporation.ceoId,
        req.params.playerId,
        req.body.rankId,
      ),
    );
  }),
);

/** DELETE /corporations/:corporationId/members/:playerId — Remove a member, acting as the CEO. */
internalRoutes.delete(
  '/corporations/:corporationId/members/:playerId',
  requireServiceRole(SERVICE_ROLES.corporationWrite),
  validate(corporationMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    const corporation = await requireCorporation(req.params.corporationId);
    await removeCorporationMember(req.params.corporationId, corporation.ceoId, req.params.playerId);
    res.status(204).send();
  }),
);

/** GET /corporations/:corporationId/politics — Corporation's political (fiscal) attachment. */
internalRoutes.get(
  '/corporations/:corporationId/politics',
  requireServiceRole(SERVICE_ROLES.corporationRead),
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const corporation = await requireCorporation(req.params.corporationId);
    res.json({ corporationId: corporation.id, politicalEntityId: corporation.politicalEntityId });
  }),
);

// ── Politics (game server) ──────────────────────────────────────────────────

/** POST /politics — Create a political entity with an explicit head (player or NPC). */
internalRoutes.post(
  '/politics',
  requireServiceRole(SERVICE_ROLES.politicsWrite),
  validate(internalCreatePoliticalEntityBody),
  asyncHandler(async (req, res) => {
    const { headId, ...data } = req.body;
    res.status(201).json(await createPoliticalEntity(headId, data));
  }),
);

/** PATCH /politics/:entityId — Edit a political entity, acting as its current head. */
internalRoutes.patch(
  '/politics/:entityId',
  requireServiceRole(SERVICE_ROLES.politicsWrite),
  validate(politicalEntityIdParams, 'params'),
  validate(politicalEntityPatchBody),
  asyncHandler(async (req, res) => {
    const entity = await requirePoliticalEntity(req.params.entityId);
    if (!entity.headId) throw new HttpError(409, 'NO_HEAD', 'Political entity has no head');
    res.json(await updatePoliticalEntity(req.params.entityId, entity.headId, req.body));
  }),
);

/** DELETE /politics/:entityId — Disband a political entity, acting as its current head. */
internalRoutes.delete(
  '/politics/:entityId',
  requireServiceRole(SERVICE_ROLES.politicsWrite),
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const entity = await requirePoliticalEntity(req.params.entityId);
    if (!entity.headId) throw new HttpError(409, 'NO_HEAD', 'Political entity has no head');
    await disbandPoliticalEntity(req.params.entityId, entity.headId);
    res.status(204).send();
  }),
);

/** POST /politics/:entityId/transfer — Transfer the head office, acting as the current head. */
internalRoutes.post(
  '/politics/:entityId/transfer',
  requireServiceRole(SERVICE_ROLES.politicsWrite),
  validate(politicalEntityIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    const entity = await requirePoliticalEntity(req.params.entityId);
    if (!entity.headId) throw new HttpError(409, 'NO_HEAD', 'Political entity has no head');
    res.json(await transferPoliticalHead(req.params.entityId, entity.headId, req.body.playerId));
  }),
);

/** PUT /players/:playerId/politics — Add an NPC to a political entity (office optional). */
internalRoutes.put(
  '/players/:playerId/politics',
  requireServiceRole(SERVICE_ROLES.politicsWrite),
  validate(playerIdParams, 'params'),
  validate(npcPoliticalBody),
  asyncHandler(async (req, res) => {
    const member = await addNpcPoliticalMember(req.body.entityId, req.params.playerId, req.body.officeId);
    res.json({
      entityId: member.entityId,
      playerId: member.playerId,
      officeId: member.officeId,
      joinedAt: member.joinedAt,
    });
  }),
);

/** DELETE /players/:playerId/politics?entityId= — Remove an NPC from a political entity. */
internalRoutes.delete(
  '/players/:playerId/politics',
  requireServiceRole(SERVICE_ROLES.politicsWrite),
  validate(playerIdParams, 'params'),
  validate(politicalQueryRequired, 'query'),
  asyncHandler(async (req, res) => {
    await removeNpcPoliticalMember(req.query.entityId as string, req.params.playerId);
    res.status(204).send();
  }),
);

/**
 * GET /players/:playerId/politics[?entityId=] — Political membership(s) and office of a profile.
 * With `entityId`, returns that membership or null; without, the list of memberships.
 */
internalRoutes.get(
  '/players/:playerId/politics',
  requireServiceRole(SERVICE_ROLES.politicsRead),
  validate(playerIdParams, 'params'),
  validate(politicalQueryOptional, 'query'),
  asyncHandler(async (req, res) => {
    const entityId = req.query.entityId as string | undefined;
    if (entityId) {
      const membership = await getPoliticalMembership(entityId, req.params.playerId);
      res.json(membership ? { ...membership.entity, office: membership.office } : null);
      return;
    }
    const memberships = await listPoliticalMemberships(req.params.playerId, Number(req.query.limit), Number(req.query.offset));
    res.json({
      ...memberships,
      items: memberships.items.map((m) => ({ ...m.entity, office: m.office })),
    });
  }),
);

/** POST /players/:playerId/stats — Apply playtime/reputation deltas and role. */
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

/**
 * GET /players/:playerId/corporation[?corporationId=] — Corporation(s) and rank of a player.
 * With `corporationId`, returns that membership or null; without, the list of memberships.
 */
internalRoutes.get(
  '/players/:playerId/corporation',
  requireServiceRole(SERVICE_ROLES.corporationRead),
  validate(playerIdParams, 'params'),
  validate(corporationQueryOptional, 'query'),
  asyncHandler(async (req, res) => {
    const corporationId = req.query.corporationId as string | undefined;
    if (corporationId) {
      const membership = await getCorporationMembership(corporationId, req.params.playerId);
      res.json(membership ? { ...membership.corporation, rank: membership.rank } : null);
      return;
    }
    const memberships = await listCorporationMemberships(
      req.params.playerId,
      Number(req.query.limit),
      Number(req.query.offset),
    );
    res.json({
      ...memberships,
      items: memberships.items.map((m) => ({ ...m.corporation, rank: m.rank })),
    });
  }),
);

/**
 * GET /players/:playerId/group[?groupId=] — Group of a player (a player belongs to at
 * most one group). With `groupId`, returns that membership or null; without, the player's
 * single group or null.
 */
internalRoutes.get(
  '/players/:playerId/group',
  requireServiceRole(SERVICE_ROLES.groupRead),
  validate(playerIdParams, 'params'),
  validate(groupQueryOptional, 'query'),
  asyncHandler(async (req, res) => {
    const groupId = req.query.groupId as string | undefined;
    if (groupId) {
      const membership = await getGroupMembership(groupId, req.params.playerId);
      res.json(membership ? { group: membership.group, member: membership.member } : null);
      return;
    }
    const membership = await getPlayerGroup(req.params.playerId);
    res.json(membership ? { group: membership.group, member: membership.member } : null);
  }),
);

/** GET /groups/:groupId — Group summary with member count, or null when it does not exist. */
internalRoutes.get(
  '/groups/:groupId',
  requireServiceRole(SERVICE_ROLES.groupRead),
  validate(groupIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await getGroupSummary(req.params.groupId));
  }),
);

/**
 * GET /players?search=&playerIds=&limit= — Resolve profiles by display-name substring
 * and/or explicit ids (trusted services that only store opaque player ids). When `search`
 * is empty, only the given ids are resolved.
 */
internalRoutes.get(
  '/players',
  requireServiceRole(SERVICE_ROLES.profileRead),
  validate(internalProfileQuery, 'query'),
  asyncHandler(async (req, res) => {
    const search = String(req.query.search ?? '');
    const playerIds = req.query.playerIds as string[];
    const limit = Number(req.query.limit);
    const offset = Number(req.query.offset);
    if (!search) {
      res.json(await getProfilesByIds(playerIds, limit, offset));
      return;
    }
    const results = await searchProfiles(search, limit, offset);
    res.json({
      ...results,
      items: results.items.map((p) => ({ playerId: p.playerId, displayName: p.displayName, entityType: p.entityType })),
    });
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

// ── Authorization (policy decision point) ───────────────────────────────────

/**
 * POST /authorize — Does `playerId` hold `action` on `holderType`/`holderId`?
 * Accepts one check or `{ checks: [...] }` (up to 50) and always answers 200: a refusal
 * is a result (`allowed: false`, `reason: not_member | missing_permission`), never an
 * error, so a batch never loses its other answers. Callers map `reason` onto their own
 * error codes and messages.
 */
internalRoutes.post(
  '/authorize',
  requireServiceRole(SERVICE_ROLES.authorize),
  validate(authorizeBody),
  asyncHandler(async (req, res) => {
    if (Array.isArray(req.body.checks)) {
      res.json({ results: await authorizeAll(req.body.checks) });
      return;
    }
    res.json(await authorize(req.body));
  }),
);

/** GET /permissions/catalog — Every catalogued action and its rules, localized. */
internalRoutes.get(
  '/permissions/catalog',
  requireServiceRole(SERVICE_ROLES.authorize),
  asyncHandler(async (_req, res) => {
    res.json({ actions: permissionCatalog() });
  }),
);
