/**
 * Corporation routes (`/api/corporations`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listCorporationActivity } from '../services/corporationActivity.service.js';
import * as ranks from '../services/corporationRanks.service.js';
import * as requests from '../services/corporationRequests.service.js';
import * as corporations from '../services/corporations.service.js';
import {
  corporationIdParams,
  corporationMemberParams,
  corporationPatchBody,
  corporationRankParams,
  corporationRequestParams,
  createCorporationBody,
  joinCorporationBody,
  limitQuery,
  memberRankBody,
  rankBody,
  rankPatchBody,
  searchQuery,
  targetPlayerBody,
} from './schemas.js';

/** Router for corporations, their members, ranks and join requests. */
export const corporationsRoutes: IRouter = Router();

/** GET /?search=&limit= — Public corporation directory. */
corporationsRoutes.get(
  '/',
  validate(searchQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await corporations.listCorporations(String(req.query.search ?? ''), Number(req.query.limit)));
  }),
);

/** POST / — Create a corporation; the caller becomes CEO. */
corporationsRoutes.post(
  '/',
  validate(createCorporationBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await corporations.createCorporation(requirePlayer(req).id, req.body));
  }),
);

/** GET /:corporationId — Public corporation page (ranks, members, presence). */
corporationsRoutes.get(
  '/:corporationId',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await corporations.getCorporationPage(req.params.corporationId));
  }),
);

/** PATCH /:corporationId — Edit corporation (manage_corporation). */
corporationsRoutes.patch(
  '/:corporationId',
  validate(corporationIdParams, 'params'),
  validate(corporationPatchBody),
  asyncHandler(async (req, res) => {
    res.json(await corporations.updateCorporation(req.params.corporationId, requirePlayer(req).id, req.body));
  }),
);

/** DELETE /:corporationId — Disband (CEO). */
corporationsRoutes.delete(
  '/:corporationId',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await corporations.disbandCorporation(req.params.corporationId, requirePlayer(req).id);
    res.status(204).send();
  }),
);

/** POST /:corporationId/transfer — Transfer leadership (CEO). */
corporationsRoutes.post(
  '/:corporationId/transfer',
  validate(corporationIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    res.json(await corporations.transferCorporationCeo(req.params.corporationId, requirePlayer(req).id, req.body.playerId));
  }),
);

/** GET /:corporationId/activity — Internal journal (members only). */
corporationsRoutes.get(
  '/:corporationId/activity',
  validate(corporationIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    await corporations.requireCorporationMember(req.params.corporationId, requirePlayer(req).id);
    res.json(await listCorporationActivity(req.params.corporationId, Number(req.query.limit)));
  }),
);

// ── Members ─────────────────────────────────────────────────────────────────

/** GET /:corporationId/members — Members with rank and presence. */
corporationsRoutes.get(
  '/:corporationId/members',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await corporations.requireCorporation(req.params.corporationId);
    res.json(await corporations.listCorporationMembers(req.params.corporationId));
  }),
);

/** PATCH /:corporationId/members/:playerId — Change a member's rank (manage_members). */
corporationsRoutes.patch(
  '/:corporationId/members/:playerId',
  validate(corporationMemberParams, 'params'),
  validate(memberRankBody),
  asyncHandler(async (req, res) => {
    res.json(
      await corporations.setCorporationMemberRank(
        req.params.corporationId,
        requirePlayer(req).id,
        req.params.playerId,
        req.body.rankId,
      ),
    );
  }),
);

/** DELETE /:corporationId/members/:playerId — Leave (self) or kick (manage_members). */
corporationsRoutes.delete(
  '/:corporationId/members/:playerId',
  validate(corporationMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    await corporations.removeCorporationMember(req.params.corporationId, requirePlayer(req).id, req.params.playerId);
    res.status(204).send();
  }),
);

// ── Ranks ───────────────────────────────────────────────────────────────────

/** GET /:corporationId/ranks — Ranks, highest first. */
corporationsRoutes.get(
  '/:corporationId/ranks',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await corporations.requireCorporation(req.params.corporationId);
    res.json(await corporations.listCorporationRanks(req.params.corporationId));
  }),
);

/** POST /:corporationId/ranks — Create a rank (manage_ranks). */
corporationsRoutes.post(
  '/:corporationId/ranks',
  validate(corporationIdParams, 'params'),
  validate(rankBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await ranks.createCorporationRank(req.params.corporationId, requirePlayer(req).id, req.body));
  }),
);

/** PATCH /:corporationId/ranks/:rankId — Edit a rank (manage_ranks). */
corporationsRoutes.patch(
  '/:corporationId/ranks/:rankId',
  validate(corporationRankParams, 'params'),
  validate(rankPatchBody),
  asyncHandler(async (req, res) => {
    res.json(
      await ranks.updateCorporationRank(
        req.params.corporationId,
        requirePlayer(req).id,
        Number(req.params.rankId),
        req.body,
      ),
    );
  }),
);

/** DELETE /:corporationId/ranks/:rankId — Delete a rank (manage_ranks). */
corporationsRoutes.delete(
  '/:corporationId/ranks/:rankId',
  validate(corporationRankParams, 'params'),
  asyncHandler(async (req, res) => {
    await ranks.deleteCorporationRank(req.params.corporationId, requirePlayer(req).id, Number(req.params.rankId));
    res.status(204).send();
  }),
);

// ── Recruitment ─────────────────────────────────────────────────────────────

/** POST /:corporationId/join — Join directly (open) or apply (apply). */
corporationsRoutes.post(
  '/:corporationId/join',
  validate(corporationIdParams, 'params'),
  validate(joinCorporationBody),
  asyncHandler(async (req, res) => {
    const result = await requests.requestJoinCorporation(req.params.corporationId, requirePlayer(req).id, req.body.message);
    res.status(result.joined ? 200 : 201).json(result);
  }),
);

/** POST /:corporationId/invitations — Invite a player (invite). */
corporationsRoutes.post(
  '/:corporationId/invitations',
  validate(corporationIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    const result = await requests.inviteCorporationPlayer(req.params.corporationId, requirePlayer(req).id, req.body.playerId);
    res.status(result.joined ? 200 : 201).json(result);
  }),
);

/** GET /:corporationId/requests — Pending applications and invitations (recruit or invite). */
corporationsRoutes.get(
  '/:corporationId/requests',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await requests.listCorporationRequests(req.params.corporationId, requirePlayer(req).id));
  }),
);

/** POST /:corporationId/requests/:id/accept — Accept an application (recruit). */
corporationsRoutes.post(
  '/:corporationId/requests/:id/accept',
  validate(corporationRequestParams, 'params'),
  asyncHandler(async (req, res) => {
    await requests.resolveCorporationRequest(req.params.corporationId, requirePlayer(req).id, Number(req.params.id), true);
    res.status(204).send();
  }),
);

/** POST /:corporationId/requests/:id/decline — Decline an application (recruit) or withdraw an invitation (invite). */
corporationsRoutes.post(
  '/:corporationId/requests/:id/decline',
  validate(corporationRequestParams, 'params'),
  asyncHandler(async (req, res) => {
    await requests.resolveCorporationRequest(req.params.corporationId, requirePlayer(req).id, Number(req.params.id), false);
    res.status(204).send();
  }),
);