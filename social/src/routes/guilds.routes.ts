/**
 * Guild routes (`/api/guilds`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listGuildActivity } from '../services/guildActivity.service.js';
import * as ranks from '../services/guildRanks.service.js';
import * as requests from '../services/guildRequests.service.js';
import * as guilds from '../services/guilds.service.js';
import {
  createGuildBody,
  guildIdParams,
  guildMemberParams,
  guildPatchBody,
  guildRankParams,
  guildRequestParams,
  joinGuildBody,
  limitQuery,
  memberRankBody,
  rankBody,
  rankPatchBody,
  searchQuery,
  targetPlayerBody,
} from './schemas.js';

/** Router for guilds, their members, ranks and join requests. */
export const guildsRoutes: IRouter = Router();

/** GET /?search=&limit= — Public guild directory. */
guildsRoutes.get(
  '/',
  validate(searchQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await guilds.listGuilds(String(req.query.search ?? ''), Number(req.query.limit)));
  }),
);

/** POST / — Create a guild; the caller becomes owner. */
guildsRoutes.post(
  '/',
  validate(createGuildBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await guilds.createGuild(requirePlayer(req).id, req.body));
  }),
);

/** GET /:guildId — Public guild page (ranks, members, presence). */
guildsRoutes.get(
  '/:guildId',
  validate(guildIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await guilds.getGuildPage(req.params.guildId));
  }),
);

/** PATCH /:guildId — Edit guild (manage_guild). */
guildsRoutes.patch(
  '/:guildId',
  validate(guildIdParams, 'params'),
  validate(guildPatchBody),
  asyncHandler(async (req, res) => {
    res.json(await guilds.updateGuild(req.params.guildId, requirePlayer(req).id, req.body));
  }),
);

/** DELETE /:guildId — Disband (owner). */
guildsRoutes.delete(
  '/:guildId',
  validate(guildIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await guilds.disbandGuild(req.params.guildId, requirePlayer(req).id);
    res.status(204).send();
  }),
);

/** POST /:guildId/transfer — Transfer ownership (owner). */
guildsRoutes.post(
  '/:guildId/transfer',
  validate(guildIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    res.json(await guilds.transferOwnership(req.params.guildId, requirePlayer(req).id, req.body.playerId));
  }),
);

/** GET /:guildId/activity — Internal journal (members only). */
guildsRoutes.get(
  '/:guildId/activity',
  validate(guildIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    await guilds.requireMember(req.params.guildId, requirePlayer(req).id);
    res.json(await listGuildActivity(req.params.guildId, Number(req.query.limit)));
  }),
);

// ── Members ─────────────────────────────────────────────────────────────────

/** GET /:guildId/members — Members with rank and presence. */
guildsRoutes.get(
  '/:guildId/members',
  validate(guildIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await guilds.requireGuild(req.params.guildId);
    res.json(await guilds.listMembers(req.params.guildId));
  }),
);

/** PATCH /:guildId/members/:playerId — Change a member's rank (manage_members). */
guildsRoutes.patch(
  '/:guildId/members/:playerId',
  validate(guildMemberParams, 'params'),
  validate(memberRankBody),
  asyncHandler(async (req, res) => {
    res.json(
      await guilds.setMemberRank(req.params.guildId, requirePlayer(req).id, req.params.playerId, req.body.rankId),
    );
  }),
);

/** DELETE /:guildId/members/:playerId — Leave (self) or kick (manage_members). */
guildsRoutes.delete(
  '/:guildId/members/:playerId',
  validate(guildMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    await guilds.removeMember(req.params.guildId, requirePlayer(req).id, req.params.playerId);
    res.status(204).send();
  }),
);

// ── Ranks ───────────────────────────────────────────────────────────────────

/** GET /:guildId/ranks — Ranks, highest first. */
guildsRoutes.get(
  '/:guildId/ranks',
  validate(guildIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await guilds.requireGuild(req.params.guildId);
    res.json(await guilds.listRanks(req.params.guildId));
  }),
);

/** POST /:guildId/ranks — Create a rank (manage_ranks). */
guildsRoutes.post(
  '/:guildId/ranks',
  validate(guildIdParams, 'params'),
  validate(rankBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await ranks.createRank(req.params.guildId, requirePlayer(req).id, req.body));
  }),
);

/** PATCH /:guildId/ranks/:rankId — Edit a rank (manage_ranks). */
guildsRoutes.patch(
  '/:guildId/ranks/:rankId',
  validate(guildRankParams, 'params'),
  validate(rankPatchBody),
  asyncHandler(async (req, res) => {
    res.json(await ranks.updateRank(req.params.guildId, requirePlayer(req).id, Number(req.params.rankId), req.body));
  }),
);

/** DELETE /:guildId/ranks/:rankId — Delete a rank (manage_ranks). */
guildsRoutes.delete(
  '/:guildId/ranks/:rankId',
  validate(guildRankParams, 'params'),
  asyncHandler(async (req, res) => {
    await ranks.deleteRank(req.params.guildId, requirePlayer(req).id, Number(req.params.rankId));
    res.status(204).send();
  }),
);

// ── Recruitment ─────────────────────────────────────────────────────────────

/** POST /:guildId/join — Join directly (open) or apply (apply). */
guildsRoutes.post(
  '/:guildId/join',
  validate(guildIdParams, 'params'),
  validate(joinGuildBody),
  asyncHandler(async (req, res) => {
    const result = await requests.requestJoin(req.params.guildId, requirePlayer(req).id, req.body.message);
    res.status(result.joined ? 200 : 201).json(result);
  }),
);

/** POST /:guildId/invitations — Invite a player (invite). */
guildsRoutes.post(
  '/:guildId/invitations',
  validate(guildIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    const result = await requests.invitePlayer(req.params.guildId, requirePlayer(req).id, req.body.playerId);
    res.status(result.joined ? 200 : 201).json(result);
  }),
);

/** GET /:guildId/requests — Pending applications and invitations (recruit or invite). */
guildsRoutes.get(
  '/:guildId/requests',
  validate(guildIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await requests.listGuildRequests(req.params.guildId, requirePlayer(req).id));
  }),
);

/** POST /:guildId/requests/:id/accept — Accept an application (recruit). */
guildsRoutes.post(
  '/:guildId/requests/:id/accept',
  validate(guildRequestParams, 'params'),
  asyncHandler(async (req, res) => {
    await requests.resolveGuildRequest(req.params.guildId, requirePlayer(req).id, Number(req.params.id), true);
    res.status(204).send();
  }),
);

/** POST /:guildId/requests/:id/decline — Decline an application (recruit) or withdraw an invitation (invite). */
guildsRoutes.post(
  '/:guildId/requests/:id/decline',
  validate(guildRequestParams, 'params'),
  asyncHandler(async (req, res) => {
    await requests.resolveGuildRequest(req.params.guildId, requirePlayer(req).id, Number(req.params.id), false);
    res.status(204).send();
  }),
);
