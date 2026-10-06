/**
 * Internal routes for the game server, the market and the mission service (`/api/internal`).
 * The caller is authenticated by `serviceAuth` on the mount point, then each route requires
 * its capability role. Holds reserve goods; consumption transfers their ownership.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requireServiceRole, SERVICE_ROLES } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import {
  consumeHold,
  createHold,
  creditStack,
  debitStack,
  getHolderInventory,
  getStack,
  registerInstance,
  releaseHold,
  setInstanceStatus,
  transferInstance,
  transferStack,
  type Holder,
} from '../services/inventory.service.js';
import {
  createPoi,
  deletePoi,
  getPoiView,
  listHolderPois,
  resolvePois,
  updatePoi,
} from '../services/pois.service.js';
import type { PoiOwnerType } from '../db/schema/index.js';
import {
  consumeHoldBody,
  createHoldBody,
  creditBody,
  holderParams,
  holderStackParams,
  holdIdParams,
  instanceParams,
  instanceStatusBody,
  internalCreatePoiBody,
  limitQuery,
  poiHolderParams,
  poiParams,
  registerInstanceBody,
  resolvePoisBody,
  transferInstanceBody,
  transferStackBody,
  updatePoiBody,
} from './schemas.js';

/** Router for trusted-service driven updates. */
export const internalRoutes: IRouter = Router();

/** Builds a holder from validated path params. */
function holderFrom(req: { params: Record<string, string> }): Holder {
  return { holderType: req.params.holderType as Holder['holderType'], holderId: req.params.holderId };
}

// ── Reads ─────────────────────────────────────────────────────────────────────

/** GET /holders/:holderType/:holderId?limit=&offset= — Inventory (stacks complete, instances paged). */
internalRoutes.get(
  '/holders/:holderType/:holderId',
  requireServiceRole(SERVICE_ROLES.read),
  validate(holderParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await getHolderInventory(holderFrom(req), Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** GET /holders/:holderType/:holderId/stacks/:goodType — One stack with held/available. */
internalRoutes.get(
  '/holders/:holderType/:holderId/stacks/:goodType',
  requireServiceRole(SERVICE_ROLES.read),
  validate(holderStackParams, 'params'),
  asyncHandler(async (req, res) => {
    const stack = await getStack(holderFrom(req), req.params.goodType);
    if (!stack) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'Stack not found', status: 404 });
      return;
    }
    res.json(stack);
  }),
);

// ── Grants & consumption ──────────────────────────────────────────────────────

/** POST /holders/:holderType/:holderId/credit — Grant fungible goods. */
internalRoutes.post(
  '/holders/:holderType/:holderId/credit',
  requireServiceRole(SERVICE_ROLES.credit),
  validate(holderParams, 'params'),
  validate(creditBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await creditStack(holderFrom(req), req.body.goodType, req.body.quantity));
  }),
);

/** POST /holders/:holderType/:holderId/debit — Consume available fungible goods. */
internalRoutes.post(
  '/holders/:holderType/:holderId/debit',
  requireServiceRole(SERVICE_ROLES.credit),
  validate(holderParams, 'params'),
  validate(creditBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await debitStack(holderFrom(req), req.body.goodType, req.body.quantity));
  }),
);

/** POST /holders/:holderType/:holderId/instances — Register (or re-affirm) a unique instance. */
internalRoutes.post(
  '/holders/:holderType/:holderId/instances',
  requireServiceRole(SERVICE_ROLES.credit),
  validate(holderParams, 'params'),
  validate(registerInstanceBody),
  asyncHandler(async (req, res) => {
    res
      .status(201)
      .json(await registerInstance(holderFrom(req), req.body.goodType, req.body.instanceId, req.body.metadata));
  }),
);

/** PATCH /holders/:holderType/:holderId/instances/:instanceId — Set physical status. */
internalRoutes.patch(
  '/holders/:holderType/:holderId/instances/:instanceId',
  requireServiceRole(SERVICE_ROLES.credit),
  validate(instanceParams, 'params'),
  validate(instanceStatusBody),
  asyncHandler(async (req, res) => {
    res.json(await setInstanceStatus(holderFrom(req), req.params.instanceId, req.body.status));
  }),
);

// ── Direct transfers (no hold) ────────────────────────────────────────────────

/** POST /transfers — Move available fungible goods between holders. */
internalRoutes.post(
  '/transfers',
  requireServiceRole(SERVICE_ROLES.transfer),
  validate(transferStackBody),
  asyncHandler(async (req, res) => {
    const { from, to, goodType, quantity } = req.body;
    res.status(201).json(await transferStack(from, to, goodType, quantity));
  }),
);

/** POST /transfers/instance — Move a unique instance between holders. */
internalRoutes.post(
  '/transfers/instance',
  requireServiceRole(SERVICE_ROLES.transfer),
  validate(transferInstanceBody),
  asyncHandler(async (req, res) => {
    const { from, to, instanceId } = req.body;
    res.status(201).json(await transferInstance(from, to, instanceId));
  }),
);

// ── Holds ─────────────────────────────────────────────────────────────────────

/** POST /holders/:holderType/:holderId/holds — Reserve goods for a pending operation. */
internalRoutes.post(
  '/holders/:holderType/:holderId/holds',
  requireServiceRole(SERVICE_ROLES.hold),
  validate(holderParams, 'params'),
  validate(createHoldBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await createHold(holderFrom(req), req.body));
  }),
);

/** POST /holds/:holdId/release — Release a hold (goods become available). */
internalRoutes.post(
  '/holds/:holdId/release',
  requireServiceRole(SERVICE_ROLES.hold),
  validate(holdIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await releaseHold(req.params.holdId));
  }),
);

/** POST /holds/:holdId/consume — Transfer the reserved goods to a holder (settlement). */
internalRoutes.post(
  '/holds/:holdId/consume',
  requireServiceRole(SERVICE_ROLES.transfer),
  validate(holdIdParams, 'params'),
  validate(consumeHoldBody),
  asyncHandler(async (req, res) => {
    res.json(await consumeHold(req.params.holdId, req.body.to));
  }),
);

// ── POIs ─────────────────────────────────────────────────────────────────────

/**
 * POST /pois/resolve — Batch geometry resolution (mission zone matching).
 * Unknown ids are omitted from the result rather than failing the call.
 */
internalRoutes.post(
  '/pois/resolve',
  requireServiceRole(SERVICE_ROLES.read),
  validate(resolvePoisBody),
  asyncHandler(async (req, res) => {
    res.json({ pois: await resolvePois(req.body.ids) });
  }),
);

/** GET /pois/:poiId — One POI with its shares. */
internalRoutes.get(
  '/pois/:poiId',
  requireServiceRole(SERVICE_ROLES.read),
  validate(poiParams, 'params'),
  asyncHandler(async (req, res) => {
    const view = await getPoiView(req.params.poiId);
    if (!view) {
      res.status(404).json({ error: 'NOT_FOUND', message: 'POI not found', status: 404 });
      return;
    }
    res.json(view);
  }),
);

/** GET /holders/:holderType/:holderId/pois?limit=&offset= — POIs owned by a holder (incl. `political`). */
internalRoutes.get(
  '/holders/:holderType/:holderId/pois',
  requireServiceRole(SERVICE_ROLES.read),
  validate(poiHolderParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(
      await listHolderPois(
        req.params.holderType as PoiOwnerType,
        req.params.holderId,
        Number(req.query.limit),
        Number(req.query.offset),
      ),
    );
  }),
);

/** POST /pois — Create a POI on behalf of any holder (game server / system). */
internalRoutes.post(
  '/pois',
  requireServiceRole(SERVICE_ROLES.poiManage),
  validate(internalCreatePoiBody),
  asyncHandler(async (req, res) => {
    const { owner, ...input } = req.body;
    res.status(201).json(await createPoi({ ownerType: owner.type, ownerId: owner.id }, input));
  }),
);

/** PATCH /pois/:poiId — Edit a POI (geometry, name, visibility...). */
internalRoutes.patch(
  '/pois/:poiId',
  requireServiceRole(SERVICE_ROLES.poiManage),
  validate(poiParams, 'params'),
  validate(updatePoiBody),
  asyncHandler(async (req, res) => {
    res.json(await updatePoi(req.params.poiId, req.body));
  }),
);

/** DELETE /pois/:poiId — Delete a POI (shares cascade). */
internalRoutes.delete(
  '/pois/:poiId',
  requireServiceRole(SERVICE_ROLES.poiManage),
  validate(poiParams, 'params'),
  asyncHandler(async (req, res) => {
    await deletePoi(req.params.poiId);
    res.status(204).end();
  }),
);
