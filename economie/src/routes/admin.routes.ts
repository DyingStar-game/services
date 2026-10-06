/**
 * Admin dashboard routes (`/api/admin`). Requires the `moderator` role at least, like the
 * Social service's moderation dashboard. Player pseudonyms live in Social, so the player
 * search and the rich rankings are enriched through Social's internal API.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requireRole } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { getPlayerAccounts } from '../services/accounts.service.js';
import { isSocialConfigured, resolveProfiles, searchProfiles } from '../services/social.client.js';
import { getEconomyStats } from '../services/stats.service.js';
import { playerSearchQuery, statsQuery } from './schemas.js';

/** Router for moderators, admins and supervisors. */
export const adminRoutes: IRouter = Router();

adminRoutes.use(requireRole('moderator'));

/** GET /stats?days=&top= — Economic analytics for the admin dashboard. */
adminRoutes.get(
  '/stats',
  validate(statsQuery, 'query'),
  asyncHandler(async (req, res) => {
    const stats = await getEconomyStats(Number(req.query.days), Number(req.query.top));
    if (isSocialConfigured() && stats.richestPlayers.length > 0) {
      const names = new Map(
        (await resolveProfiles(stats.richestPlayers.map((p) => p.holderId))).map((p) => [p.playerId, p.displayName]),
      );
      stats.richestPlayers = stats.richestPlayers.map((p) => ({ ...p, displayName: names.get(p.holderId) ?? null }));
    }
    res.json(stats);
  }),
);

/** GET /players?search=&limit=&offset= — Search players by pseudonym, with their wallet balances. */
adminRoutes.get(
  '/players',
  validate(playerSearchQuery, 'query'),
  asyncHandler(async (req, res) => {
    const search = String(req.query.search ?? '');
    const limit = Number(req.query.limit);
    const offset = Number(req.query.offset);
    if (!isSocialConfigured()) {
      res.json({ items: [], total: 0, limit, offset });
      return;
    }
    const found = await searchProfiles(search, limit, offset);
    const players = await Promise.all(
      found.items.map(async (profile) => ({
        playerId: profile.playerId,
        displayName: profile.displayName,
        entityType: profile.entityType,
        accounts: await getPlayerAccounts(profile.playerId),
      })),
    );
    res.json({ ...found, items: players });
  }),
);
