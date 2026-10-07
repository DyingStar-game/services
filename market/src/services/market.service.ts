/**
 * Market exchange: order book with immediate matching, demand/contract fulfilment and
 * trade settlement. This service never touches the physical world: a trade moves ownership
 * (inventory) and credits (economy), each idempotently, and is retryable while `pending`.
 */
import { and, asc, count, desc, eq, inArray, ne, or, sql } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  marketDemands,
  marketOrders,
  marketTrades,
  type GoodKind,
  type HolderType,
  type MarketDemand,
  type MarketOrder,
  type MarketTrade,
  type OrderSide,
  type TradeStatus,
} from '../db/schema/index.js';
import { HttpError, conflict, notFound } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { ensureCorporationTrade } from './authorization.js';
import { creditHolder, debitHolder, isEconomyConfigured } from './economy.client.js';
import { isInventoryConfigured, transferInstance, transferStack, type Holder } from './inventory.client.js';

type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

/** Owner of an order/demand/trade. */
export interface Party extends Holder {
  isCorporation: boolean;
  /** Authenticated player performing the action (used to authorize corporation trades). */
  actorId: string;
}

/** Result of placing an order: the order plus every trade it generated. */
export interface PlaceOrderResult {
  order: MarketOrder;
  trades: MarketTrade[];
}

/** Order book query filter. */
export interface OrderFilter {
  goodType?: string;
  side?: OrderSide;
  holderType?: HolderType;
  holderId?: string;
  status?: MarketOrder['status'];
  limit?: number;
  offset?: number;
}

/** Demand query filter. */
export interface DemandFilter {
  goodType?: string;
  holderType?: HolderType;
  holderId?: string;
  status?: MarketDemand['status'];
  limit?: number;
  offset?: number;
}

/** Trade query filter. */
export interface TradeFilter {
  holderType?: HolderType;
  holderId?: string;
  status?: TradeStatus;
  limit?: number;
  offset?: number;
}

/** Input accepted when placing an order (holder is derived by the route). */
export interface PlaceOrderInput {
  side: OrderSide;
  goodType: string;
  kind: GoodKind;
  instanceId?: string;
  quantity: number;
  /** Credits per unit (stack) or total price (instance). */
  price: number;
  currency?: string;
  actor: string;
}

/** Input accepted when creating a demand. */
export interface CreateDemandInput {
  goodType: string;
  kind: GoodKind;
  instanceId?: string;
  quantity: number;
  maxPrice: number;
  currency?: string;
  message?: string;
  actor: string;
}

/** Normalises a party into the inventory holder shape. */
function toHolder(party: Party): Holder {
  return { holderType: party.holderType, holderId: party.holderId };
}

/** Holder types whose wallets Economy exposes for settlement (buyer/seller money moves). */
const SETTLEABLE_HOLDER_TYPES = ['player', 'npc', 'corporation'] as const;
type SettleableHolderType = (typeof SETTLEABLE_HOLDER_TYPES)[number];

/** Whether a holder type can be settled through the Economy internal wallet API. */
function isSettleableHolder(holderType: HolderType): holderType is SettleableHolderType {
  return (SETTLEABLE_HOLDER_TYPES as readonly string[]).includes(holderType);
}

/** Resolves the expiration date from the configured default TTL. */
function defaultExpiry(): Date | undefined {
  const hours = env.market.orderTtlHours;
  if (!hours || hours <= 0) return undefined;
  return new Date(Date.now() + hours * 3_600_000);
}

/**
 * Creates a trade row for a buy/sell match or a demand fulfilment. Settlement is attempted
 * immediately; when the goods or funds are unavailable the trade stays `pending`.
 */
async function createTrade(input: {
  goodType: string;
  kind: GoodKind;
  instanceId?: string | null;
  quantity: number;
  unitPrice: number;
  totalPrice: number;
  currency: string;
  buyer: Party;
  seller: Party;
  buyOrderId?: string | null;
  sellOrderId?: string | null;
  demandId?: string | null;
  details?: Record<string, unknown>;
}): Promise<MarketTrade> {
  const [trade] = await db
    .insert(marketTrades)
    .values({
      goodType: input.goodType,
      kind: input.kind,
      instanceId: input.instanceId ?? null,
      quantity: input.quantity,
      unitPrice: input.unitPrice,
      totalPrice: input.totalPrice,
      currency: input.currency,
      buyerType: input.buyer.holderType,
      buyerId: input.buyer.holderId,
      sellerType: input.seller.holderType,
      sellerId: input.seller.holderId,
      buyOrderId: input.buyOrderId ?? null,
      sellOrderId: input.sellOrderId ?? null,
      demandId: input.demandId ?? null,
      status: 'pending',
      details: input.details,
    })
    .returning();
  return trade;
}

/**
 * Settles a trade: moves goods (inventory) then money (economy), idempotently. Safe to retry.
 * @param tradeId - Trade id.
 * @returns The settled (or still pending) trade.
 */
export async function settleTrade(tradeId: string): Promise<MarketTrade> {
  const [trade] = await db.select().from(marketTrades).where(eq(marketTrades.id, tradeId)).limit(1);
  if (!trade) throw notFound(t('not_found.trade', { id: tradeId }));
  if (trade.status === 'settled') return trade;

  const seller: Holder = { holderType: trade.sellerType, holderId: trade.sellerId };
  const buyer: Holder = { holderType: trade.buyerType, holderId: trade.buyerId };

  try {
    if (!trade.goodsMoved) {
      if (!isInventoryConfigured()) throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'Inventory is not configured');
      if (trade.kind === 'instance') {
        await transferInstance(seller, buyer, trade.instanceId!);
      } else {
        await transferStack(seller, buyer, trade.goodType, trade.quantity);
      }
      await db.update(marketTrades).set({ goodsMoved: true, updatedAt: new Date() }).where(eq(marketTrades.id, tradeId));
    }

    if (!trade.moneyMoved) {
      if (!isEconomyConfigured()) throw new HttpError(503, 'ECONOMY_NOT_CONFIGURED', 'Economy is not configured');
      if (!isSettleableHolder(trade.buyerType)) {
        throw new HttpError(400, 'UNSUPPORTED_HOLDER', t('holder.buyer', { holder: trade.buyerType }));
      }
      if (!isSettleableHolder(trade.sellerType)) {
        throw new HttpError(400, 'UNSUPPORTED_HOLDER', t('holder.seller', { holder: trade.sellerType }));
      }
      await debitHolder(trade.buyerType, trade.buyerId, {
        amount: trade.totalPrice,
        currency: trade.currency,
        reference: `market:trade:${trade.id}`,
        externalId: `market-trade-debit:${trade.id}`,
      });
      await creditHolder(trade.sellerType, trade.sellerId, {
        amount: trade.totalPrice,
        currency: trade.currency,
        reference: `market:trade:${trade.id}`,
        externalId: `market-trade-credit:${trade.id}`,
      });
      await db.update(marketTrades).set({ moneyMoved: true, updatedAt: new Date() }).where(eq(marketTrades.id, tradeId));
    }

    const [settled] = await db
      .update(marketTrades)
      .set({ status: 'settled', settledAt: new Date(), failureReason: null, updatedAt: new Date() })
      .where(eq(marketTrades.id, tradeId))
      .returning();
    return settled;
  } catch (err) {
    const reason = err instanceof HttpError ? err.code : 'SETTLEMENT_ERROR';
    const [failed] = await db
      .update(marketTrades)
      .set({ status: 'pending', failureReason: reason, updatedAt: new Date() })
      .where(eq(marketTrades.id, tradeId))
      .returning();
    if (err instanceof HttpError && err.status === 400) throw err;
    return failed;
  }
}

/** Best-price ordering for resting orders on a given side. */
function bookOrder(side: OrderSide): ReturnType<typeof asc>[] | ReturnType<typeof desc>[] {
  // Sell book: cheapest first. Buy book: highest first.
  return side === 'sell'
    ? [asc(marketOrders.price), asc(marketOrders.createdAt)]
    : [desc(marketOrders.price), asc(marketOrders.createdAt)];
}

/**
 * Matches a freshly placed order against resting opposite orders and returns the trades.
 * Buy and sell prices cross when `buy.price >= sell.price`; the trade executes at the
 * resting order's price. Full-fill matching: quantities must align, else partial matching
 * is applied on the resting order side (the incoming order's remainder stays open).
 */
async function matchOrder(incoming: MarketOrder, party: Party): Promise<MarketTrade[]> {
  const trades: MarketTrade[] = [];
  const opposite: OrderSide = incoming.side === 'sell' ? 'buy' : 'sell';
  let remaining = incoming.remaining;

  const resting = await db
    .select()
    .from(marketOrders)
    .where(
      and(
        eq(marketOrders.goodType, incoming.goodType),
        eq(marketOrders.side, opposite),
        eq(marketOrders.status, 'open'),
        ne(marketOrders.id, incoming.id),
        sql`${marketOrders.holderId} <> ${incoming.holderId}`,
      ),
    )
    .orderBy(...bookOrder(opposite));

  for (const candidate of resting) {
    if (remaining <= 0) break;
    const priceCrosses = incoming.side === 'buy' ? incoming.price >= candidate.price : candidate.price >= incoming.price;
    if (!priceCrosses) break;

    const fill = Math.min(remaining, candidate.remaining);
    if (fill <= 0) continue;

    const buyer = incoming.side === 'buy' ? party : partyFromOrder(candidate);
    const seller = incoming.side === 'sell' ? party : partyFromOrder(candidate);
    const unitPrice = candidate.price;

    const trade = await createTrade({
      goodType: incoming.goodType,
      kind: incoming.kind,
      instanceId: incoming.kind === 'instance' ? incoming.instanceId : null,
      quantity: fill,
      unitPrice,
      totalPrice: unitPrice * fill,
      currency: incoming.currency,
      buyer,
      seller,
      buyOrderId: incoming.side === 'buy' ? incoming.id : candidate.id,
      sellOrderId: incoming.side === 'sell' ? incoming.id : candidate.id,
    });
    trades.push(trade);

    await db.transaction(async (tx) => {
      await updateOrderRemaining(tx, candidate.id, candidate.remaining - fill);
      await updateOrderRemaining(tx, incoming.id, remaining - fill);
    });
    remaining -= fill;
  }

  return trades;
}

/** Rebuilds a party (holder + corporation flag) from a resting order. */
function partyFromOrder(order: MarketOrder): Party {
  return {
    holderType: order.holderType,
    holderId: order.holderId,
    isCorporation: order.isCorporation,
    actorId: order.createdBy ?? order.holderId,
  };
}

/** Decrements an order's remaining quantity and updates its status. */
async function updateOrderRemaining(tx: Tx, orderId: string, newRemaining: number): Promise<void> {
  const status = newRemaining <= 0 ? 'filled' : 'partially_filled';
  await tx
    .update(marketOrders)
    .set({ remaining: Math.max(0, newRemaining), status, updatedAt: new Date() })
    .where(eq(marketOrders.id, orderId));
}

/**
 * Places an order and immediately matches it against the book. Settlement of each generated
 * trade is attempted right away.
 * @param party - Order owner.
 * @param input - Order parameters.
 * @returns The order and its trades.
 */
export async function placeOrder(party: Party, input: PlaceOrderInput): Promise<PlaceOrderResult> {
  if (input.kind === 'instance' && input.side === 'buy') {
    throw new HttpError(400, 'INVALID_SIDE', 'Buy orders are only supported for fungible goods; use a demand for instances');
  }
  if (input.kind === 'instance' && !input.instanceId) {
    throw new HttpError(400, 'INSTANCE_REQUIRED', 'instanceId is required for an instance sell order');
  }
  await ensureCorporationTrade(party, party.actorId);

  const [order] = await db
    .insert(marketOrders)
    .values({
      side: input.side,
      goodType: input.goodType,
      kind: input.kind,
      instanceId: input.kind === 'instance' ? input.instanceId : null,
      quantity: input.quantity,
      remaining: input.quantity,
      price: input.price,
      currency: input.currency ?? 'credits',
      holderType: party.holderType,
      holderId: party.holderId,
      isCorporation: party.isCorporation,
      createdBy: input.actor,
      expiresAt: defaultExpiry(),
    })
    .returning();

  const trades = await matchOrder(order, party);
  for (const trade of trades) await settleTrade(trade.id);

  const [fresh] = await db.select().from(marketOrders).where(eq(marketOrders.id, order.id)).limit(1);
  return { order: fresh, trades };
}

/** Lists a page of orders matching a filter (newest first). */
export async function listOrders(filter: OrderFilter): Promise<Page<MarketOrder>> {
  const conditions = [];
  if (filter.goodType) conditions.push(eq(marketOrders.goodType, filter.goodType));
  if (filter.side) conditions.push(eq(marketOrders.side, filter.side));
  if (filter.holderType) conditions.push(eq(marketOrders.holderType, filter.holderType));
  if (filter.holderId) conditions.push(eq(marketOrders.holderId, filter.holderId));
  if (filter.status) conditions.push(eq(marketOrders.status, filter.status));
  const condition = conditions.length ? and(...conditions) : undefined;
  const limit = filter.limit ?? env.market.defaultLimit;
  const offset = filter.offset ?? 0;
  const [items, totalRows] = await Promise.all([
    db.select().from(marketOrders).where(condition).orderBy(desc(marketOrders.createdAt), desc(marketOrders.id)).limit(limit).offset(offset),
    db.select({ total: count() }).from(marketOrders).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/** Fetches an order or throws 404. */
export async function requireOrder(orderId: string): Promise<MarketOrder> {
  const [order] = await db.select().from(marketOrders).where(eq(marketOrders.id, orderId)).limit(1);
  if (!order) throw notFound(t('not_found.order', { id: orderId }));
  return order;
}

/**
 * Cancels an open/partially-filled order owned by the actor.
 * @param orderId - Order id.
 * @param party - Must be the owner.
 * @returns The cancelled order.
 */
export async function cancelOrder(orderId: string, party: Party): Promise<MarketOrder> {
  const order = await requireOrder(orderId);
  assertOwner(order.holderType, order.holderId, party, 'order');
  if (order.status === 'filled' || order.status === 'cancelled') {
    throw conflict(t('conflict.order_state', { id: orderId, status: order.status }));
  }
  const [row] = await db
    .update(marketOrders)
    .set({ status: 'cancelled', updatedAt: new Date() })
    .where(eq(marketOrders.id, orderId))
    .returning();
  return row;
}

/** Creates a demand/contract. */
export async function createDemand(party: Party, input: CreateDemandInput): Promise<MarketDemand> {
  if (input.kind === 'instance' && !input.instanceId) {
    // An instance demand may target a specific instance; without one it targets any instance of the type.
    // We keep it optional and match on type when absent.
  }
  await ensureCorporationTrade(party, party.actorId);
  const [demand] = await db
    .insert(marketDemands)
    .values({
      goodType: input.goodType,
      kind: input.kind,
      instanceId: input.kind === 'instance' ? input.instanceId ?? null : null,
      quantity: input.quantity,
      maxPrice: input.maxPrice,
      currency: input.currency ?? 'credits',
      holderType: party.holderType,
      holderId: party.holderId,
      isCorporation: party.isCorporation,
      message: input.message,
      createdBy: input.actor,
      expiresAt: defaultExpiry(),
    })
    .returning();
  return demand;
}

/** Lists a page of demands matching a filter (newest first). */
export async function listDemands(filter: DemandFilter): Promise<Page<MarketDemand>> {
  const conditions = [];
  if (filter.goodType) conditions.push(eq(marketDemands.goodType, filter.goodType));
  if (filter.holderType) conditions.push(eq(marketDemands.holderType, filter.holderType));
  if (filter.holderId) conditions.push(eq(marketDemands.holderId, filter.holderId));
  if (filter.status) conditions.push(eq(marketDemands.status, filter.status));
  const condition = conditions.length ? and(...conditions) : undefined;
  const limit = filter.limit ?? env.market.defaultLimit;
  const offset = filter.offset ?? 0;
  const [items, totalRows] = await Promise.all([
    db.select().from(marketDemands).where(condition).orderBy(desc(marketDemands.createdAt), desc(marketDemands.id)).limit(limit).offset(offset),
    db.select({ total: count() }).from(marketDemands).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/** Fetches a demand or throws 404. */
export async function requireDemand(demandId: string): Promise<MarketDemand> {
  const [demand] = await db.select().from(marketDemands).where(eq(marketDemands.id, demandId)).limit(1);
  if (!demand) throw notFound(t('not_found.demand', { id: demandId }));
  return demand;
}

/**
 * Fulfils an open demand: the seller delivers the goods to the requester (buyer) at an
 * agreed unit price (which must not exceed the demand's maximum).
 * @param demandId - Demand id.
 * @param seller - Fulfilling party.
 * @param unitPrice - Agreed credits per unit.
 * @returns The created (and settlement-attempted) trade.
 */
export async function fulfillDemand(demandId: string, seller: Party, unitPrice: number): Promise<MarketTrade> {
  const demand = await requireDemand(demandId);
  if (demand.status !== 'open') throw conflict(t('conflict.demand_state', { id: demandId, status: demand.status }));
  if (unitPrice > demand.maxPrice) {
    throw new HttpError(400, 'PRICE_ABOVE_MAX', `Unit price ${unitPrice} exceeds the demand max ${demand.maxPrice}`, { price: unitPrice, max: demand.maxPrice });
  }
  await ensureCorporationTrade(seller, seller.actorId);

  const trade = await createTrade({
    goodType: demand.goodType,
    kind: demand.kind,
    instanceId: demand.instanceId,
    quantity: demand.quantity,
    unitPrice,
    totalPrice: unitPrice * demand.quantity,
    currency: demand.currency,
    buyer: {
      holderType: demand.holderType,
      holderId: demand.holderId,
      isCorporation: demand.isCorporation,
      actorId: demand.createdBy ?? demand.holderId,
    },
    seller,
    demandId: demand.id,
  });

  await db
    .update(marketDemands)
    .set({ status: 'fulfilled', updatedAt: new Date() })
    .where(and(eq(marketDemands.id, demandId), eq(marketDemands.status, 'open')));

  return settleTrade(trade.id);
}

/**
 * Cancels an open demand owned by the actor.
 * @param demandId - Demand id.
 * @param party - Must be the owner.
 * @returns The cancelled demand.
 */
export async function cancelDemand(demandId: string, party: Party): Promise<MarketDemand> {
  const demand = await requireDemand(demandId);
  assertOwner(demand.holderType, demand.holderId, party, 'demand');
  if (demand.status !== 'open') throw conflict(t('conflict.demand_state', { id: demandId, status: demand.status }));
  const [row] = await db
    .update(marketDemands)
    .set({ status: 'cancelled', updatedAt: new Date() })
    .where(eq(marketDemands.id, demandId))
    .returning();
  return row;
}

/** Lists a page of trades involving a holder (either side), newest first. */
export async function listTrades(filter: TradeFilter): Promise<Page<MarketTrade>> {
  const conditions = [];
  if (filter.status) conditions.push(eq(marketTrades.status, filter.status));
  if (filter.holderType && filter.holderId) {
    conditions.push(
      or(
        and(eq(marketTrades.buyerType, filter.holderType), eq(marketTrades.buyerId, filter.holderId)),
        and(eq(marketTrades.sellerType, filter.holderType), eq(marketTrades.sellerId, filter.holderId)),
      )!,
    );
  }
  const condition = conditions.length ? and(...conditions) : undefined;
  const limit = filter.limit ?? env.market.defaultLimit;
  const offset = filter.offset ?? 0;
  const [items, totalRows] = await Promise.all([
    db.select().from(marketTrades).where(condition).orderBy(desc(marketTrades.createdAt), desc(marketTrades.id)).limit(limit).offset(offset),
    db.select({ total: count() }).from(marketTrades).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/** Fetches a trade or throws 404. */
export async function requireTrade(tradeId: string): Promise<MarketTrade> {
  const [trade] = await db.select().from(marketTrades).where(eq(marketTrades.id, tradeId)).limit(1);
  if (!trade) throw notFound(t('not_found.trade', { id: tradeId }));
  return trade;
}

/** Asserts that the party owns a resource (order/demand). */
function assertOwner(holderType: HolderType, holderId: string, party: Party, label: string): void {
  if (holderType !== party.holderType || holderId !== party.holderId) {
    throw new HttpError(403, 'FORBIDDEN', t('forbidden.no_ownership', { label }));
  }
}

/** One page of the order book for a good type: bids (best first) and asks (best first). */
export interface BookDepth {
  goodType: string;
  bids: MarketOrder[];
  asks: MarketOrder[];
  /** Open buy orders in total (beyond this page). */
  bidTotal: number;
  /** Open sell orders in total (beyond this page). */
  askTotal: number;
  limit: number;
  offset: number;
}

/**
 * Order book depth for a good type: a page of open buy orders and a page of open sell
 * orders, best price first, with the totals on each side.
 * @param goodType - Good type to depth.
 * @param limit - Max orders per side.
 * @param offset - Orders to skip on each side.
 * @returns The book page.
 */
export async function bookDepth(goodType: string, limit: number, offset: number): Promise<BookDepth> {
  const open = and(eq(marketOrders.goodType, goodType), inArray(marketOrders.status, ['open', 'partially_filled']));
  const buySide = and(open, eq(marketOrders.side, 'buy'));
  const sellSide = and(open, eq(marketOrders.side, 'sell'));
  const [bids, asks, buyTotal, sellTotal] = await Promise.all([
    db
      .select()
      .from(marketOrders)
      .where(buySide)
      .orderBy(desc(marketOrders.price), asc(marketOrders.createdAt), asc(marketOrders.id))
      .limit(limit)
      .offset(offset),
    db
      .select()
      .from(marketOrders)
      .where(sellSide)
      .orderBy(asc(marketOrders.price), asc(marketOrders.createdAt), asc(marketOrders.id))
      .limit(limit)
      .offset(offset),
    db.select({ total: count() }).from(marketOrders).where(buySide),
    db.select({ total: count() }).from(marketOrders).where(sellSide),
  ]);
  return {
    goodType,
    bids,
    asks,
    bidTotal: buyTotal[0]?.total ?? 0,
    askTotal: sellTotal[0]?.total ?? 0,
    limit,
    offset,
  };
}
