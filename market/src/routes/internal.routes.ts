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
import { listDemands, listOrders, listTrades, requireTrade, settleTrade } from '../services/market.service.js';
import {
  catalogBody,
  catalogEnabledBody,
  goodTypeParams,
  internalDemandsQuery,
  internalOrdersQuery,
  internalTradesQuery,
  tradeIdParams,
} from './schemas.js';

/** Router for game-server driven updates. */
export const internalRoutes: IRouter = Router();

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
