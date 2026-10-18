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
  assertPoiGeometryEditable,
  assertPoiNotInUse,
  assertPoiWriteAccess,
  createPoi,
  deletePoi,
  getPoiView,
  listPlayerPois,
  MISSION_POI_OWNER_ID,
  requirePlayerRead,
  requirePoi,
  revokeShare,
  sharePoi,
  transferPoi,
  updatePoi,
} from '../services/pois.service.js';
import {
  createPoiBody,
  limitQuery,
  poiParams,
  poiShareParams,
  sharePoiBody,
  transferPoiBody,
  updatePoiBody,
} from './schemas.js';

/** Router for the current player's inventory. */
export const meRoutes: IRouter = Router();

/** GET /inventory?limit=&offset= — Stacks (with held/available) and paged owned instances. */
meRoutes.get(
  '/inventory',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(
      await getHolderInventory(
        { holderType: 'player', holderId: player.id },
        Number(req.query.limit),
        Number(req.query.offset),
      ),
    );
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

/** GET /pois?limit=&offset= — POIs of the player's own scope (owned, granted, public). */
meRoutes.get(
  '/pois',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await listPlayerPois(player.id, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /pois — Create a POI (own, corporation/political managed, or mission-only). */
meRoutes.post(
  '/pois',
  validate(createPoiBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { owner, ...input } = req.body;
    // The acting player is always recorded: geometry stays editable by whoever placed the
    // POI, for their own, their corporation's or a mission's POIs alike.
    input.createdBy = player.id;
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
    } else if (owner?.type === 'mission') {
      holder = { ownerType: 'mission', ownerId: MISSION_POI_OWNER_ID };
      input.visibility = 'private';
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

/**
 * PATCH /pois/:poiId — Edit a POI the player manages (ownership unchanged).
 *
 * Geometry (`system`, `scene`, `x`, `y`, `z`, `radiusM`) may be corrected **by the player
 * who created the POI**, or by a member managing an organization POI: the criterion is the
 * creator because the same player can create a POI of their own, of their corporation or of
 * a mission, and in all three cases they are the one who placed it. It is refused while the
 * POI is the destination of a mission someone has accepted (`400 POI_IN_USE`), so a
 * contract's delivery radius never moves under the acceptor's feet.
 */
meRoutes.patch(
  '/pois/:poiId',
  validate(poiParams, 'params'),
  validate(updatePoiBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const poi = await requirePoi(req.params.poiId);
    await assertPoiWriteAccess(poi, player.id);
    const G = ['system', 'scene', 'x', 'y', 'z', 'radiusM'] as const;
    const touched = G.filter((f) => req.body[f] !== undefined);
    if (touched.length > 0) {
      assertPoiGeometryEditable(poi, player.id);
      await assertPoiNotInUse(poi.id);
    }
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
