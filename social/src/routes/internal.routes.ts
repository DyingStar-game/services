/**
 * Internal routes for the game server and trusted services (`/api/internal`, `X-Internal-Key`).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { validate } from '../middleware/validate.js';
import { recordActivity } from '../services/activity.service.js';
import { recordEncounter } from '../services/encounters.service.js';
import { getMembership } from '../services/guilds.service.js';
import { setPresence } from '../services/presence.service.js';
import { applyStats, ensureProfile } from '../services/profiles.service.js';
import {
  activityBody,
  encounterBody,
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
  validate(playerIdParams, 'params'),
  validate(upsertPlayerBody),
  asyncHandler(async (req, res) => {
    res.json(await ensureProfile(req.params.playerId, req.body.displayName));
  }),
);

/** PUT /players/:playerId/presence — Set status and location. */
internalRoutes.put(
  '/players/:playerId/presence',
  validate(playerIdParams, 'params'),
  validate(presenceBody),
  asyncHandler(async (req, res) => {
    res.json(await setPresence(req.params.playerId, req.body.status, req.body.location));
  }),
);

/** POST /players/:playerId/stats — Apply playtime/reputation deltas, level and role. */
internalRoutes.post(
  '/players/:playerId/stats',
  validate(playerIdParams, 'params'),
  validate(statsBody),
  asyncHandler(async (req, res) => {
    res.json(await applyStats(req.params.playerId, req.body));
  }),
);

/** POST /players/:playerId/activity — Append a game activity entry. */
internalRoutes.post(
  '/players/:playerId/activity',
  validate(playerIdParams, 'params'),
  validate(activityBody),
  asyncHandler(async (req, res) => {
    await recordActivity(req.params.playerId, req.body.type, req.body.details);
    res.status(204).send();
  }),
);

/** GET /players/:playerId/guild — Guild and rank of a player (null if guildless). */
internalRoutes.get(
  '/players/:playerId/guild',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const membership = await getMembership(req.params.playerId);
    res.json(membership ? { ...membership.guild, rank: membership.rank } : null);
  }),
);

/** POST /encounters — Record that two players met. */
internalRoutes.post(
  '/encounters',
  validate(encounterBody),
  asyncHandler(async (req, res) => {
    await recordEncounter(req.body.playerId, req.body.otherPlayerId);
    res.status(204).send();
  }),
);
