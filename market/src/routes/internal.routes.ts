/**
 * Internal routes for the game server (`/api/internal`). The caller is authenticated by
 * `serviceAuth` on the mount point, then each route requires its capability role. The game
 * server pushes the catalog, inspects the book and can retry pending settlements.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requireServiceRole, SERVICE_ROLES } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listCatalog, setCatalogEnabled, upsertCatalogEntry } from '../services/catalog.service.js';
import {
  cancelDemand,
  cancelOrder,
  createDemand,
  fulfillDemand,
  listDemands,
  listOrders,
  listTrades,
  placeOrder,
  requireTrade,
  settleTrade,
  type Party,
} from '../services/market.service.js';
import {
  catalogBody,
  catalogEnabledBody,
  goodTypeParams,
  internalDemandBody,
  internalDemandsQuery,
  internalFulfillBody,
  internalHolderBody,
  internalOrderBody,
  internalOrdersQuery,
  internalTradesQuery,
  orderIdParams,
  tradeIdParams,
} from './schemas.js';

/** Router for game-server driven updates. */
export const internalRoutes: IRouter = Router();

/**
 * Resolves the trading party from an explicit holder (game server acting for a player,
 * an NPC or a corporation). When an `actor` player is supplied, `market:trade` on a
 * corporation holder is decided by Social.
 */
function internalParty(input: {
  holderType: 'player' | 'npc' | 'corporation' | 'system';
  holderId: string;
  actor?: string;
}): Party {
  return {
    holderType: input.holderType,
    holderId: input.holderId,
    isCorporation: input.holderType === 'corporation',
    actorId: input.actor ?? input.holderId,
  };
}

// ── Catalog ───────────────────────────────────────────────────────────────────

/** GET /catalog — All catalog entries (including disabled). */
internalRoutes.get(
  '/catalog',
  requireServiceRole(SERVICE_ROLES.read),
  asyncHandler(async (_req, res) => {
    res.json(await listCatalog(false));
  }),
);

/** PUT /catalog/:goodType — Upsert a catalog entry (game-defined good type). */
internalRoutes.put(
  '/catalog/:goodType',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(goodTypeParams, 'params'),
  validate(catalogBody),
  asyncHandler(async (req, res) => {
    res.json(await upsertCatalogEntry(req.params.goodType, req.body));
  }),
);

/** PATCH /catalog/:goodType — Enable/disable a good type. */
internalRoutes.patch(
  '/catalog/:goodType',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(goodTypeParams, 'params'),
  validate(catalogEnabledBody),
  asyncHandler(async (req, res) => {
    res.json(await setCatalogEnabled(req.params.goodType, req.body.enabled));
  }),
);

// ── Reads ─────────────────────────────────────────────────────────────────────

/** GET /orders — Order book (all holders). */
internalRoutes.get(
  '/orders',
  requireServiceRole(SERVICE_ROLES.read),
  validate(internalOrdersQuery, 'query'),
  asyncHandler(async (req, res) => {
    const q = req.query as unknown as {
      goodType?: string;
      side?: never;
      status?: never;
      holderType?: never;
      holderId?: string;
      limit: number;
    };
    res.json(
      await listOrders({
        goodType: q.goodType,
        side: q.side,
        status: q.status,
        holderType: q.holderType,
        holderId: q.holderId,
        limit: Number(q.limit),
      }),
    );
  }),
);

/** GET /demands — Demands (all holders). */
internalRoutes.get(
  '/demands',
  requireServiceRole(SERVICE_ROLES.read),
  validate(internalDemandsQuery, 'query'),
  asyncHandler(async (req, res) => {
    const q = req.query as unknown as {
      goodType?: string;
      status?: never;
      holderType?: never;
      holderId?: string;
      limit: number;
    };
    res.json(
      await listDemands({
        goodType: q.goodType,
        status: q.status,
        holderType: q.holderType,
        holderId: q.holderId,
        limit: Number(q.limit),
      }),
    );
  }),
);

/** GET /trades — Trades (all holders). */
internalRoutes.get(
  '/trades',
  requireServiceRole(SERVICE_ROLES.read),
  validate(internalTradesQuery, 'query'),
  asyncHandler(async (req, res) => {
    const q = req.query as unknown as { status?: never; holderType?: never; holderId?: string; limit: number };
    res.json(
      await listTrades({
        status: q.status,
        holderType: q.holderType,
        holderId: q.holderId,
        limit: Number(q.limit),
      }),
    );
  }),
);

// ── Game-server driven trading (players / NPCs / corporations) ────────────────

/** POST /orders — Place an order on behalf of an explicit holder (immediate matching). */
internalRoutes.post(
  '/orders',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(internalOrderBody),
  asyncHandler(async (req, res) => {
    const body = req.body as { holderType: Party['holderType']; holderId: string; corporationId?: string };
    const party = internalParty(body);
    const input = req.body as Omit<Parameters<typeof placeOrder>[1], 'actor'>;
    res.status(201).json(await placeOrder(party, { ...input, actor: req.service?.clientId ?? party.actorId }));
  }),
);

/** POST /orders/:id/cancel — Cancel an order owned by the given holder. */
internalRoutes.post(
  '/orders/:id/cancel',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(orderIdParams, 'params'),
  validate(internalHolderBody),
  asyncHandler(async (req, res) => {
    res.json(await cancelOrder(req.params.id, internalParty(req.body)));
  }),
);

/** POST /demands — Create a demand/contract on behalf of an explicit holder. */
internalRoutes.post(
  '/demands',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(internalDemandBody),
  asyncHandler(async (req, res) => {
    const body = req.body as { holderType: Party['holderType']; holderId: string; corporationId?: string };
    const party = internalParty(body);
    const input = req.body as Omit<Parameters<typeof createDemand>[1], 'actor'>;
    res.status(201).json(await createDemand(party, { ...input, actor: req.service?.clientId ?? party.actorId }));
  }),
);

/** POST /demands/:id/fulfill — Fulfil a demand as the given holder (settles the trade). */
internalRoutes.post(
  '/demands/:id/fulfill',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(orderIdParams, 'params'),
  validate(internalFulfillBody),
  asyncHandler(async (req, res) => {
    const body = req.body as { unitPrice: number };
    res.status(201).json(await fulfillDemand(req.params.id, internalParty(req.body), body.unitPrice));
  }),
);

/** POST /demands/:id/cancel — Cancel a demand owned by the given holder. */
internalRoutes.post(
  '/demands/:id/cancel',
  requireServiceRole(SERVICE_ROLES.manage),
  validate(orderIdParams, 'params'),
  validate(internalHolderBody),
  asyncHandler(async (req, res) => {
    res.json(await cancelDemand(req.params.id, internalParty(req.body)));
  }),
);

// ── Settlement retry ──────────────────────────────────────────────────────────

/** GET /trades/:id — One trade. */
internalRoutes.get(
  '/trades/:id',
  requireServiceRole(SERVICE_ROLES.read),
  validate(tradeIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await requireTrade(req.params.id));
  }),
);

/** POST /trades/:id/settle — Retry settling a pending trade (idempotent). */
internalRoutes.post(
  '/trades/:id/settle',
  requireServiceRole(SERVICE_ROLES.settle),
  validate(tradeIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await settleTrade(req.params.id));
  }),
);
