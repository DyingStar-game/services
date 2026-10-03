/**
 * Market exchange model. This service owns *exchange flows* only: it never touches the
 * physical world. Tradable goods are identified by a `goodType` in the catalog; ownership
 * is delegated to the inventory service and money to the economy service.
 *
 * - `market_catalog`: the (game-defined) list of tradable good types and their nature.
 * - `market_orders`: the continuous order book (sell orders for stacks/instances, buy
 *   orders for stacks). Matching is immediate when a new order crosses a resting one.
 * - `market_demands`: requests/contracts (B2B) that any holder can fulfil directly.
 * - `market_trades`: an executed exchange; settlement moves goods (inventory) and money
 *   (economy) idempotently.
 */
import { sql } from 'drizzle-orm';
import {
  bigint,
  boolean,
  check,
  index,
  jsonb,
  pgTable,
  text,
  timestamp,
  uuid,
} from 'drizzle-orm/pg-core';

/** Holder kinds, mirroring inventory/economy. `system` covers the game/NPC brokers. */
export const HOLDER_TYPES = ['player', 'npc', 'corporation', 'system'] as const;
export type HolderType = (typeof HOLDER_TYPES)[number];

/** Whether a good is fungible (stack) or a unique instance. */
export const GOOD_KINDS = ['stack', 'instance'] as const;
export type GoodKind = (typeof GOOD_KINDS)[number];

/** Order side: `sell` offers goods, `buy` bids for fungible goods. */
export const ORDER_SIDES = ['buy', 'sell'] as const;
export type OrderSide = (typeof ORDER_SIDES)[number];

export const ORDER_STATUSES = ['open', 'partially_filled', 'filled', 'cancelled', 'expired'] as const;
export type OrderStatus = (typeof ORDER_STATUSES)[number];

export const DEMAND_STATUSES = ['open', 'fulfilled', 'cancelled', 'expired'] as const;
export type DemandStatus = (typeof DEMAND_STATUSES)[number];

export const TRADE_STATUSES = ['pending', 'settled', 'cancelled'] as const;
export type TradeStatus = (typeof TRADE_STATUSES)[number];

/** Game-defined list of tradable good types. Fed by the game server. */
export const marketCatalog = pgTable('market_catalog', {
  goodType: text('good_type').primaryKey(),
  kind: text('kind').$type<GoodKind>().notNull(),
  /** Display unit (e.g. `t`, `units`). */
  unit: text('unit'),
  displayName: text('display_name'),
  enabled: boolean('enabled').notNull().default(true),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

export type MarketCatalogEntry = typeof marketCatalog.$inferSelect;
export type NewMarketCatalogEntry = typeof marketCatalog.$inferInsert;

/** Order book entry. */
export const marketOrders = pgTable(
  'market_orders',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    side: text('side').$type<OrderSide>().notNull(),
    goodType: text('good_type').notNull(),
    kind: text('kind').$type<GoodKind>().notNull(),
    /** Instance UUID for an instance sell order; null for stack orders. */
    instanceId: uuid('instance_id'),
    quantity: bigint('quantity', { mode: 'number' }).notNull(),
    remaining: bigint('remaining', { mode: 'number' }).notNull(),
    /** Credits per unit (stack) or total price (instance). */
    price: bigint('price', { mode: 'number' }).notNull(),
    currency: text('currency').notNull().default('credits'),
    holderType: text('holder_type').$type<HolderType>().notNull(),
    holderId: uuid('holder_id').notNull(),
    isCorporation: boolean('is_corporation').notNull().default(false),
    status: text('status').$type<OrderStatus>().notNull().default('open'),
    /** Player id or service client id that created the order. */
    createdBy: text('created_by'),
    expiresAt: timestamp('expires_at', { withTimezone: true }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    check('market_orders_quantity_positive', sql`${t.quantity} > 0`),
    check('market_orders_price_non_negative', sql`${t.price} >= 0`),
    check(
      'market_orders_buy_is_stack',
      sql`${t.side} = 'sell' or ${t.kind} = 'stack'`,
    ),
    index('market_orders_book_idx').on(t.goodType, t.side, t.status, t.price),
    index('market_orders_holder_idx').on(t.holderType, t.holderId),
    index('market_orders_instance_idx').on(t.instanceId),
  ],
);

export type MarketOrder = typeof marketOrders.$inferSelect;
export type NewMarketOrder = typeof marketOrders.$inferInsert;

/** Request/contract: a holder asks for goods and any holder may fulfil it. */
export const marketDemands = pgTable(
  'market_demands',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    goodType: text('good_type').notNull(),
    kind: text('kind').$type<GoodKind>().notNull(),
    instanceId: uuid('instance_id'),
    quantity: bigint('quantity', { mode: 'number' }).notNull(),
    /** Maximum credits per unit the requester accepts. */
    maxPrice: bigint('max_price', { mode: 'number' }).notNull(),
    currency: text('currency').notNull().default('credits'),
    holderType: text('holder_type').$type<HolderType>().notNull(),
    holderId: uuid('holder_id').notNull(),
    isCorporation: boolean('is_corporation').notNull().default(false),
    message: text('message'),
    status: text('status').$type<DemandStatus>().notNull().default('open'),
    createdBy: text('created_by'),
    expiresAt: timestamp('expires_at', { withTimezone: true }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    check('market_demands_quantity_positive', sql`${t.quantity} > 0`),
    check('market_demands_max_price_non_negative', sql`${t.maxPrice} >= 0`),
    index('market_demands_open_idx').on(t.goodType, t.status),
    index('market_demands_holder_idx').on(t.holderType, t.holderId),
  ],
);

export type MarketDemand = typeof marketDemands.$inferSelect;
export type NewMarketDemand = typeof marketDemands.$inferInsert;

/** Executed exchange. `goodsMoved`/`moneyMoved` make settlement safely retryable. */
export const marketTrades = pgTable(
  'market_trades',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    goodType: text('good_type').notNull(),
    kind: text('kind').$type<GoodKind>().notNull(),
    instanceId: uuid('instance_id'),
    quantity: bigint('quantity', { mode: 'number' }).notNull(),
    unitPrice: bigint('unit_price', { mode: 'number' }).notNull(),
    totalPrice: bigint('total_price', { mode: 'number' }).notNull(),
    currency: text('currency').notNull().default('credits'),
    buyerType: text('buyer_type').$type<HolderType>().notNull(),
    buyerId: uuid('buyer_id').notNull(),
    sellerType: text('seller_type').$type<HolderType>().notNull(),
    sellerId: uuid('seller_id').notNull(),
    buyOrderId: uuid('buy_order_id'),
    sellOrderId: uuid('sell_order_id'),
    demandId: uuid('demand_id'),
    status: text('status').$type<TradeStatus>().notNull().default('pending'),
    /** Idempotent settlement steps. */
    goodsMoved: boolean('goods_moved').notNull().default(false),
    moneyMoved: boolean('money_moved').notNull().default(false),
    failureReason: text('failure_reason'),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    settledAt: timestamp('settled_at', { withTimezone: true }),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    check('market_trades_quantity_positive', sql`${t.quantity} > 0`),
    index('market_trades_status_idx').on(t.status),
    index('market_trades_buyer_idx').on(t.buyerType, t.buyerId),
    index('market_trades_seller_idx').on(t.sellerType, t.sellerId),
  ],
);

export type MarketTrade = typeof marketTrades.$inferSelect;
export type NewMarketTrade = typeof marketTrades.$inferInsert;
