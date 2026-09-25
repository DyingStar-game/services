/**
 * Routes for the authenticated player's own profile (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listActivity } from '../services/activity.service.js';
import { listPlayerRequests, resolvePlayerRequest } from '../services/corporationRequests.service.js';
import { getCorporationMembership, getCorporationRefMap } from '../services/corporations.service.js';
import { getPresence } from '../services/presence.service.js';
import { ensureProfile, updateProfile } from '../services/profiles.service.js';
import { listReputationEvents } from '../services/reputation.service.js';
import { listActiveSanctions } from '../services/sanctions.service.js';
import { limitQuery, profilePatchBody, requestIdParams } from './schemas.js';

/** Router for the current player's profile and history. */
export const meRoutes: IRouter = Router();

/** GET / — Own profile (created on first call) with presence. */
meRoutes.get(
  '/',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const profile = await ensureProfile(player.id, player.username);
    const [presence, corporationRefs] = await Promise.all([
      getPresence(player.id),
      getCorporationRefMap([player.id]),
    ]);
    res.json({ ...profile, presence, corporation: corporationRefs.get(player.id) ?? null });
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

// ── Corporation ─────────────────────────────────────────────────────────────

/** GET /corporation — Own corporation with rank, or null. */
meRoutes.get(
  '/corporation',
  asyncHandler(async (req, res) => {
    const membership = await getCorporationMembership(requirePlayer(req).id);
    res.json(
      membership
        ? { ...membership.corporation, joinedAt: membership.member.joinedAt, rank: membership.rank }
        : null,
    );
  }),
);

/** GET /corporation/requests — My pending invitations and applications. */
meRoutes.get(
  '/corporation/requests',
  asyncHandler(async (req, res) => {
    res.json(await listPlayerRequests(requirePlayer(req).id));
  }),
);

/** POST /corporation/requests/:id/accept — Accept an invitation. */
meRoutes.post(
  '/corporation/requests/:id/accept',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await resolvePlayerRequest(requirePlayer(req).id, Number(req.params.id), true);
    res.status(204).send();
  }),
);

/** POST /corporation/requests/:id/decline — Decline an invitation or withdraw an application. */
meRoutes.post(
  '/corporation/requests/:id/decline',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await resolvePlayerRequest(requirePlayer(req).id, Number(req.params.id), false);
    res.status(204).send();
  }),
);

// ── Reputation ──────────────────────────────────────────────────────────────

/** GET /reputation?limit= — Score, history and active sanctions (reachable while sanctioned). */
meRoutes.get(
  '/reputation',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const profile = await ensureProfile(player.id, player.username);
    const [events, activeSanctions] = await Promise.all([
      listReputationEvents(player.id, Number(req.query.limit)),
      listActiveSanctions(player.id),
    ]);
    res.json({ reputation: profile.reputation, events, activeSanctions });
  }),
);

/** GET /sanctions — My active sanctions (reachable while sanctioned). */
meRoutes.get(
  '/sanctions',
  asyncHandler(async (req, res) => {
    res.json(await listActiveSanctions(requirePlayer(req).id));
  }),
);
