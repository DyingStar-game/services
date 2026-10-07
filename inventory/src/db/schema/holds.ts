/**
 * Holds reserve owned goods for a pending operation (a market order or a mission item
 * escrow). A hold does not move ownership: the goods stay on the holder's stack/instance,
 * but their quantity is no longer *available* until the hold is consumed (transferred) or
 * released (cancelled). This is how the "no magic transfer" rule is enforced: the game
 * server confirms the physical exchange, then the hold is consumed into a real transfer.
 */
import { sql } from 'drizzle-orm';
import {
  bigint,
  check,
  index,
  jsonb,
  pgTable,
  text,
  timestamp,
  uuid,
} from 'drizzle-orm/pg-core';

import { type HolderType } from './holders.js';

/** What a hold is attached to. `refId` is opaque (order id, escrow id, ...). */
export const HOLD_REF_TYPES = ['market_order', 'mission_escrow', 'manual'] as const;
export type HoldRefType = (typeof HOLD_REF_TYPES)[number];

/** Whether the hold reserves quantity of a fungible stack or a unique instance. */
export const HOLD_KINDS = ['stack', 'instance'] as const;
export type HoldKind = (typeof HOLD_KINDS)[number];

export const HOLD_STATUSES = ['active', 'consumed', 'released'] as const;
export type HoldStatus = (typeof HOLD_STATUSES)[number];

export const inventoryHolds = pgTable(
  'inventory_holds',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    /** Holder whose goods are reserved. */
    holderType: text('holder_type').$type<HolderType>().notNull(),
    holderId: uuid('holder_id').notNull(),
    kind: text('kind').$type<HoldKind>().notNull(),
    goodType: text('good_type').notNull(),
    /** Reserved quantity for a stack hold; 1 for an instance hold. */
    quantity: bigint('quantity', { mode: 'number' }).notNull(),
    /** Instance UUID for an instance hold; null for a stack hold. */
    instanceId: uuid('instance_id'),
    refType: text('ref_type').$type<HoldRefType>().notNull(),
    /** Opaque reference (market order id, mission assignment/escrow id, ...). */
    refId: text('ref_id'),
    status: text('status').$type<HoldStatus>().notNull().default('active'),
    details: jsonb('details').$type<Record<string, unknown>>(),
    expiresAt: timestamp('expires_at', { withTimezone: true }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    check('inventory_holds_quantity_positive', sql`${t.quantity} > 0`),
    check(
      'inventory_holds_kind_consistent',
      sql`(${t.kind} = 'instance' and ${t.instanceId} is not null) or (${t.kind} = 'stack' and ${t.instanceId} is null)`,
    ),
    index('inventory_holds_holder_idx').on(t.holderType, t.holderId, t.goodType),
    index('inventory_holds_ref_idx').on(t.refType, t.refId),
    index('inventory_holds_instance_idx').on(t.instanceId),
  ],
);

export type InventoryHold = typeof inventoryHolds.$inferSelect;
export type NewInventoryHold = typeof inventoryHolds.$inferInsert;
