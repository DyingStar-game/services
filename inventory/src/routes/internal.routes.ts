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
  consumeHoldBody,
  createHoldBody,
  creditBody,
  holderParams,
  holderStackParams,
  holdIdParams,
  instanceParams,
  instanceStatusBody,
  registerInstanceBody,
  transferInstanceBody,
  transferStackBody,
} from './schemas.js';

/** Router for trusted-service driven updates. */
export const internalRoutes: IRouter = Router();

/** Builds a holder from validated path params. */
function holderFrom(req: { params: Record<string, string> }): Holder {
  return { holderType: req.params.holderType as Holder['holderType'], holderId: req.params.holderId };
}

// ── Reads ─────────────────────────────────────────────────────────────────────

/** GET /holders/:holderType/:holderId — Full inventory (stacks with available, instances). */
internalRoutes.get(
  '/holders/:holderType/:holderId',
  requireServiceRole(SERVICE_ROLES.read),
  validate(holderParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await getHolderInventory(holderFrom(req)));
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
