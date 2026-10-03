/**
 * Player-facing market routes (`/api/market`): catalog, order book, demands/contracts and
 * trades. A player trades from their own wallet; passing `corporationId` trades from a
 * corporation the player belongs to.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { ensureCorporationMember } from '../services/authorization.js';
import { listCatalog } from '../services/catalog.service.js';
import {
  cancelDemand,
  cancelOrder,
  createDemand,
  fulfillDemand,
  listDemands,
  listOrders,
  listTrades,
  placeOrder,
  requireDemand,
  requireOrder,
  bookDepth,
  type Party,
} from '../services/market.service.js';
import {
  bookQuery,
  demandBody,
  demandsQuery,
  fulfillBody,
  orderBody,
  orderIdParams,
  ordersQuery,
  tradesQuery,
} from './schemas.js';

/** Router mounted at `/api/market`. */
export const marketRoutes: IRouter = Router();

/**
 * Resolves the trading party from the request: the caller's corporation when
 * `corporationId` is provided (membership checked in Social), the player otherwise.
 */
async function resolveParty(req: { body: { corporationId?: string } }, playerId: string): Promise<Party> {
  const corporationId = req.body.corporationId;
  if (corporationId) {
    const party: Party = { holderType: 'corporation', holderId: corporationId, isCorporation: true, actorId: playerId };
    await ensureCorporationMember(party, playerId);
    return party;
  }
  return { holderType: 'player', holderId: playerId, isCorporation: false, actorId: playerId };
}

/** GET /catalog — Tradable good types. */
marketRoutes.get(
  '/catalog',
  asyncHandler(async (_req, res) => {
    res.json(await listCatalog(true));
  }),
);

/** GET /book?goodType= — Order book depth (open buy/sell counts). */
marketRoutes.get(
  '/book',
  validate(bookQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await bookDepth(String(req.query.goodType)));
  }),
);

// ── Orders ────────────────────────────────────────────────────────────────────

/** GET /orders?goodType=&side=&status=&limit= — Public order book. */
marketRoutes.get(
  '/orders',
  validate(ordersQuery, 'query'),
  asyncHandler(async (req, res) => {
    const q = req.query as unknown as { goodType?: string; side?: never; status?: never; limit: number };
    res.json(await listOrders({ goodType: q.goodType, side: q.side, status: q.status, limit: Number(q.limit) }));
  }),
);

/** GET /orders/:id — One order. */
marketRoutes.get(
  '/orders/:id',
  validate(orderIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await requireOrder(req.params.id));
  }),
);

/** POST /orders — Place an order (immediate matching). */
marketRoutes.post(
  '/orders',
  validate(orderBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const party = await resolveParty(req, player.id);
    res.status(201).json(await placeOrder(party, { ...req.body, actor: player.id }));
  }),
);

/** POST /orders/:id/cancel — Cancel one of the caller's orders. */
marketRoutes.post(
  '/orders/:id/cancel',
  validate(orderIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await cancelOrder(req.params.id, await resolveParty(req, player.id)));
  }),
);

// ── Demands / contracts ───────────────────────────────────────────────────────

/** GET /demands?goodType=&status=&limit= — Open demands. */
marketRoutes.get(
  '/demands',
  validate(demandsQuery, 'query'),
  asyncHandler(async (req, res) => {
    const q = req.query as unknown as { goodType?: string; status?: never; limit: number };
    res.json(await listDemands({ goodType: q.goodType, status: q.status, limit: Number(q.limit) }));
  }),
);

/** GET /demands/:id — One demand. */
marketRoutes.get(
  '/demands/:id',
  validate(orderIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await requireDemand(req.params.id));
  }),
);

/** POST /demands — Create a demand/contract. */
marketRoutes.post(
  '/demands',
  validate(demandBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const party = await resolveParty(req, player.id);
    res.status(201).json(await createDemand(party, { ...req.body, actor: player.id }));
  }),
);

/** POST /demands/:id/fulfill — Fulfil a demand as a supplier (settles the trade). */
marketRoutes.post(
  '/demands/:id/fulfill',
  validate(orderIdParams, 'params'),
  validate(fulfillBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const party = await resolveParty(req, player.id);
    res.status(201).json(await fulfillDemand(req.params.id, party, req.body.unitPrice));
  }),
);

/** POST /demands/:id/cancel — Cancel one of the caller's demands. */
marketRoutes.post(
  '/demands/:id/cancel',
  validate(orderIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    res.json(await cancelDemand(req.params.id, await resolveParty(req, player.id)));
  }),
);

// ── Trades ────────────────────────────────────────────────────────────────────

/** GET /trades?status=&limit= — Trades where the caller is buyer or seller. */
marketRoutes.get(
  '/trades',
  validate(tradesQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const q = req.query as unknown as { status?: never; limit: number };
    res.json(await listTrades({ holderType: 'player', holderId: player.id, status: q.status, limit: Number(q.limit) }));
  }),
);
