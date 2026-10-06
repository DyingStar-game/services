/**
 * Routes for the authenticated player's own inventory and POIs (`/api/me`).
 *
 * POI access is resolved from ownership: the caller owns the POI, or (for an
 * organization-owned POI) holds the organization's management permission in Social.
 * Read access widens to grants and `public` POIs.
 */
import { Router, type IRouter } from 'express';

import type { PoiOwnerType } from '../db/schema/index.js';
import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { getHolderInventory, getStack } from '../services/inventory.service.js';
import {
  assertCorporationManage,
  assertPoliticalManage,
  assertPoiWriteAccess,
  createPoi,
  deletePoi,
  getPoiView,
  listPlayerPois,
  requirePlayerRead,
  requirePoi,
  revokeShare,
  sharePoi,
  transferPoi,
  updatePoi,
} from '../services/pois.service.js';
import {
  createPoiBody,
  poiParams,
  poiShareParams,
  sharePoiBody,
  transferPoiBody,
  updatePoiBody,
} from './schemas.js';

/** Router for the current player's inventory. */
export const meRoutes: IRouter = Router();

/** GET /inventory — Stacks (with held/available) and owned instances. */
meRoutes.get(
  '/inventory',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await getHolderInventory({ holderType: 'player', holderId: player.id }));
  }),
);

/** GET /inventory/stacks/:goodType — One stack of the player, with held/available. */
meRoutes.get(
  '/inventory/stacks/:goodType',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const stack = await getStack({ holderType: 'player', holderId: player.id }, req.params.goodType);
    if (!stack) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'Stack not found', status: 404 });
      return;
    }
    res.json(stack);
  }),
);

// ── POIs ─────────────────────────────────────────────────────────────────────

/** GET /pois — POIs of the player's own scope (owned, granted, public). */
meRoutes.get(
  '/pois',
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listPlayerPois(player.id));
  }),
);

/** POST /pois — Create a POI (own, or for a corporation/political entity the player manages). */
meRoutes.post(
  '/pois',
  validate(createPoiBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { owner, ...input } = req.body;
    let holder: { ownerType: PoiOwnerType; ownerId: string } = {
      ownerType: 'player',
      ownerId: player.id,
    };
    if (owner?.type === 'corporation') {
      await assertCorporationManage(player.id, owner.id);
      holder = { ownerType: 'corporation', ownerId: owner.id };
    } else if (owner?.type === 'political') {
      await assertPoliticalManage(player.id, owner.id);
      holder = { ownerType: 'political', ownerId: owner.id };
    }
    res.status(201).json(await createPoi(holder, input));
  }),
);

/** GET /pois/:poiId — One POI with its shares (owned, granted or public). */
meRoutes.get(
  '/pois/:poiId',
  validate(poiParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await requirePlayerRead(poi, player.id);
    res.json(await getPoiView(poi.id));
  }),
);

/** PATCH /pois/:poiId — Edit a POI the player manages (ownership unchanged). */
meRoutes.patch(
  '/pois/:poiId',
  validate(poiParams, 'params'),
  validate(updatePoiBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await assertPoiWriteAccess(poi, player.id);
    res.json(await updatePoi(poi.id, req.body));
  }),
);

/** DELETE /pois/:poiId — Delete a POI the player manages (shares cascade). */
meRoutes.delete(
  '/pois/:poiId',
  validate(poiParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await assertPoiWriteAccess(poi, player.id);
    await deletePoi(poi.id);
    res.status(204).end();
  }),
);

/** POST /pois/:poiId/shares — Grant read-only access to a player/NPC/corp/political entity. */
meRoutes.post(
  '/pois/:poiId/shares',
  validate(poiParams, 'params'),
  validate(sharePoiBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await assertPoiWriteAccess(poi, player.id);
    res.status(201).json(await sharePoi(poi, req.body, player.id));
  }),
);

/** DELETE /pois/:poiId/shares/:granteeType/:granteeId — Revoke a grant. */
meRoutes.delete(
  '/pois/:poiId/shares/:granteeType/:granteeId',
  validate(poiShareParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await assertPoiWriteAccess(poi, player.id);
    await revokeShare(poi.id, {
      granteeType: req.params.granteeType as 'player' | 'npc' | 'corporation' | 'political',
      granteeId: req.params.granteeId,
    });
    res.status(204).end();
  }),
);

/** POST /pois/:poiId/transfer — Hand ownership over to another player/NPC/org. */
meRoutes.post(
  '/pois/:poiId/transfer',
  validate(poiParams, 'params'),
  validate(transferPoiBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await assertPoiWriteAccess(poi, player.id);
    res.json(await transferPoi(poi, req.body));
  }),
);
