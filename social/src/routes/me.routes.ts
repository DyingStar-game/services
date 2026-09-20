/**
 * Routes for the authenticated player's own profile (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listActivity } from '../services/activity.service.js';
import { listPlayerRequests, resolvePlayerRequest } from '../services/guildRequests.service.js';
import { getGuildRefMap, getMembership } from '../services/guilds.service.js';
import { getPresence } from '../services/presence.service.js';
import { ensureProfile, updateProfile } from '../services/profiles.service.js';
import { limitQuery, profilePatchBody, requestIdParams } from './schemas.js';

/** Router for the current player's profile and history. */
export const meRoutes: IRouter = Router();

/** GET / — Own profile (created on first call) with presence. */
meRoutes.get(
  '/',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const profile = await ensureProfile(player.id, player.username);
    const [presence, guildRefs] = await Promise.all([getPresence(player.id), getGuildRefMap([player.id])]);
    res.json({ ...profile, presence, guild: guildRefs.get(player.id) ?? null });
  }),
);

/** PATCH / — Update editable profile fields. */
meRoutes.patch(
  '/',
  validate(profilePatchBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    await ensureProfile(player.id, player.username);
    res.json(await updateProfile(player.id, req.body));
  }),
);

/** GET /activity — Own activity history (newest first). */
meRoutes.get(
  '/activity',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listActivity(player.id, Number(req.query.limit)));
  }),
);

// ── Guild ───────────────────────────────────────────────────────────────────

/** GET /guild — Own guild with rank, or null. */
meRoutes.get(
  '/guild',
  asyncHandler(async (req, res) => {
    const membership = await getMembership(requirePlayer(req).id);
    res.json(membership ? { ...membership.guild, joinedAt: membership.member.joinedAt, rank: membership.rank } : null);
  }),
);

/** GET /guild/requests — My pending invitations and applications. */
meRoutes.get(
  '/guild/requests',
  asyncHandler(async (req, res) => {
    res.json(await listPlayerRequests(requirePlayer(req).id));
  }),
);

/** POST /guild/requests/:id/accept — Accept an invitation. */
meRoutes.post(
  '/guild/requests/:id/accept',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await resolvePlayerRequest(requirePlayer(req).id, Number(req.params.id), true);
    res.status(204).send();
  }),
);

/** POST /guild/requests/:id/decline — Decline an invitation or withdraw an application. */
meRoutes.post(
  '/guild/requests/:id/decline',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await resolvePlayerRequest(requirePlayer(req).id, Number(req.params.id), false);
    res.status(204).send();
  }),
);
