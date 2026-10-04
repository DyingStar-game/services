/**
 * Player-facing group routes (`/api/groups`): temporary groups used to claim and share
 * missions. Management (patch, disband, kick, invite) is owner-only; members leave on
 * their own. Ownership passes to the oldest member when the owner leaves.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { notFound } from '../lib/httpError.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import {
  createGroup,
  disbandGroup,
  getGroupSummary,
  inviteGroupPlayer,
  leaveGroup,
  listGroupMembers,
  removeGroupMember,
  requireGroupMembership,
  updateGroup,
} from '../services/groups.service.js';
import { createGroupBody, groupIdParams, groupInviteBody, groupMemberParams, groupPatchBody } from './schemas.js';

/** Router mounted at `/api/groups`. */
export const groupsRoutes: IRouter = Router();

/** POST / — Create a group (the caller becomes its owner and first member). */
groupsRoutes.post(
  '/',
  validate(createGroupBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.status(201).json(await createGroup(player.id, req.body));
  }),
);

/** GET /:groupId — Group summary (members only). */
groupsRoutes.get(
  '/:groupId',
  validate(groupIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await requireGroupMembership(req.params.groupId, player.id);
    const summary = await getGroupSummary(req.params.groupId);
    if (!summary) throw notFound(`Group ${req.params.groupId} not found`);
    res.json(summary);
  }),
);

/** PATCH /:groupId — Update editable fields (owner only). */
groupsRoutes.patch(
  '/:groupId',
  validate(groupIdParams, 'params'),
  validate(groupPatchBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await updateGroup(req.params.groupId, player.id, req.body));
  }),
);

/** DELETE /:groupId — Disband the group (owner only, cascade-deletes members/invitations). */
groupsRoutes.delete(
  '/:groupId',
  validate(groupIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await disbandGroup(req.params.groupId, player.id);
    res.status(204).send();
  }),
);

/** GET /:groupId/members — Members with profile and presence (members only). */
groupsRoutes.get(
  '/:groupId/members',
  validate(groupIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listGroupMembers(req.params.groupId, player.id));
  }),
);

/** DELETE /:groupId/members/:playerId — Remove a member (owner only). */
groupsRoutes.delete(
  '/:groupId/members/:playerId',
  validate(groupMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await removeGroupMember(req.params.groupId, player.id, req.params.playerId);
    res.status(204).send();
  }),
);

/** POST /:groupId/leave — Leave the group (owner succession when the owner leaves). */
groupsRoutes.post(
  '/:groupId/leave',
  validate(groupIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await leaveGroup(req.params.groupId, player.id);
    res.status(204).send();
  }),
);

/** POST /:groupId/invitations — Invite a player (owner only). */
groupsRoutes.post(
  '/:groupId/invitations',
  validate(groupIdParams, 'params'),
  validate(groupInviteBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.status(201).json(await inviteGroupPlayer(req.params.groupId, player.id, req.body.playerId));
  }),
);
