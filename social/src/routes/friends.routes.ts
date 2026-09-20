/**
 * Friend list and request routes (`/api/friends`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import * as friends from '../services/friends.service.js';
import { limitQuery, playerIdParams, requestIdParams, targetPlayerBody } from './schemas.js';

/** Router for friendships. */
export const friendsRoutes: IRouter = Router();

/** GET / — Accepted friends with presence. */
friendsRoutes.get(
  '/',
  asyncHandler(async (req, res) => {
    res.json(await friends.listFriends(requirePlayer(req).id));
  }),
);

/** GET /online — Friends currently online or in mission, with location. */
friendsRoutes.get(
  '/online',
  asyncHandler(async (req, res) => {
    res.json(await friends.listOnlineFriends(requirePlayer(req).id));
  }),
);

/** GET /suggestions — Recently met players who are not friends yet. */
friendsRoutes.get(
  '/suggestions',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await friends.listSuggestions(requirePlayer(req).id, Number(req.query.limit)));
  }),
);

/** GET /requests — Pending requests, incoming and outgoing. */
friendsRoutes.get(
  '/requests',
  asyncHandler(async (req, res) => {
    res.json(await friends.listRequests(requirePlayer(req).id));
  }),
);

/** POST /requests — Send a friend request (auto-accepts a reverse pending one). */
friendsRoutes.post(
  '/requests',
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    const result = await friends.sendRequest(requirePlayer(req).id, req.body.playerId);
    res.status(result.status === 'accepted' ? 200 : 201).json(result);
  }),
);

/** POST /requests/:id/accept — Accept an incoming request. */
friendsRoutes.post(
  '/requests/:id/accept',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await friends.acceptRequest(Number(req.params.id), requirePlayer(req).id));
  }),
);

/** POST /requests/:id/decline — Decline an incoming request or cancel an outgoing one. */
friendsRoutes.post(
  '/requests/:id/decline',
  validate(requestIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await friends.declineRequest(Number(req.params.id), requirePlayer(req).id);
    res.status(204).send();
  }),
);

/** DELETE /:playerId — Remove a friend. */
friendsRoutes.delete(
  '/:playerId',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const removed = await friends.removeFriend(requirePlayer(req).id, req.params.playerId);
    if (!removed) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'Not friends', status: 404 });
      return;
    }
    res.status(204).send();
  }),
);
