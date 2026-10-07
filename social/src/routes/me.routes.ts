/**
 * Routes for the authenticated player's own profile (`/api/me`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listActivity } from '../services/activity.service.js';
import { permissionCatalog } from '../services/authorize.service.js';
import { listPlayerRequests, resolvePlayerRequest } from '../services/corporationRequests.service.js';
import { getCorporationRefMap, listCorporationMemberships } from '../services/corporations.service.js';
import { getPlayerGroup, listPlayerGroupInvitations, resolveGroupInvitation } from '../services/groups.service.js';
import { getPoliticalRefMap, listPoliticalMemberships } from '../services/politics.service.js';
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
    const [presence, corporationRefs, politicsRefs, groupMembership] = await Promise.all([
      getPresence(player.id),
      getCorporationRefMap([player.id]),
      getPoliticalRefMap([player.id]),
      getPlayerGroup(player.id),
    ]);
    res.json({
      ...profile,
      presence,
      corporations: corporationRefs.get(player.id) ?? [],
      politics: politicsRefs.get(player.id) ?? [],
      group: groupMembership ? { ...groupMembership.group, joinedAt: groupMembership.member.joinedAt } : null,
    });
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

/** GET /activity — Own activity history (newest first, paginated). */
meRoutes.get(
  '/activity',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listActivity(player.id, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** GET /permissions/catalog — Catalog of organisation actions (localized, static). */
meRoutes.get(
  '/permissions/catalog',
  asyncHandler(async (_req, res) => {
    res.json({ actions: permissionCatalog() });
  }),
);

// ── Corporation ─────────────────────────────────────────────────────────────

/** GET /corporations — Corporations the player belongs to, with rank (paginated). */
meRoutes.get(
  '/corporations',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const memberships = await listCorporationMemberships(requirePlayer(req).id, Number(req.query.limit), Number(req.query.offset));
    res.json({
      ...memberships,
      items: memberships.items.map((m) => ({ ...m.corporation, joinedAt: m.member.joinedAt, rank: m.rank })),
    });
  }),
);

// ── Politics ────────────────────────────────────────────────────────────────

/** GET /politics — Political entities the player belongs to, with office (paginated). */
meRoutes.get(
  '/politics',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const memberships = await listPoliticalMemberships(requirePlayer(req).id, Number(req.query.limit), Number(req.query.offset));
    res.json({
      ...memberships,
      items: memberships.items.map((m) => ({ ...m.entity, joinedAt: m.member.joinedAt, office: m.office })),
    });
  }),
);

/** GET /corporation/requests — My pending invitations and applications (paginated). */
meRoutes.get(
  '/corporation/requests',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listPlayerRequests(requirePlayer(req).id, Number(req.query.limit), Number(req.query.offset)));
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

// ── Group ───────────────────────────────────────────────────────────────────

/** GET /groups — The player's group (a player belongs to at most one), or null. */
meRoutes.get(
  '/groups',
  asyncHandler(async (req, res) => {
    const membership = await getPlayerGroup(requirePlayer(req).id);
    res.json(membership ? { ...membership.group, joinedAt: membership.member.joinedAt } : null);
  }),
);

/** GET /group/invitations — My pending group invitations (paginated). */
meRoutes.get(
  '/group/invitations',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listPlayerGroupInvitations(requirePlayer(req).id, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /group/invitations/:id/accept — Accept an invitation and join the group. */
meRoutes.post(
  '/group/invitations/:id/accept',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await resolveGroupInvitation(requirePlayer(req).id, Number(req.params.id), true);
    res.status(204).send();
  }),
);

/** POST /group/invitations/:id/decline — Decline an invitation. */
meRoutes.post(
  '/group/invitations/:id/decline',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await resolveGroupInvitation(requirePlayer(req).id, Number(req.params.id), false);
    res.status(204).send();
  }),
);

// ── Reputation ──────────────────────────────────────────────────────────────

/** GET /reputation?limit=&offset= — Score, paginated history and active sanctions (reachable while sanctioned). */
meRoutes.get(
  '/reputation',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const profile = await ensureProfile(player.id, player.username);
    const [events, activeSanctions] = await Promise.all([
      listReputationEvents(player.id, Number(req.query.limit), Number(req.query.offset)),
      listActiveSanctions(player.id),
    ]);
    res.json({
      reputation: profile.reputation,
      events: events.items,
      eventsTotal: events.total,
      limit: events.limit,
      offset: events.offset,
      activeSanctions,
    });
  }),
);

/** GET /sanctions — My active sanctions (reachable while sanctioned). */
meRoutes.get(
  '/sanctions',
  asyncHandler(async (req, res) => {
    res.json(await listActiveSanctions(requirePlayer(req).id));
  }),
);
