/**
 * Goods and their ownership.
 *
 * A *good type* is either **fungible** (tracked as a stack: type + quantity, e.g. 500 t of
 * ore) or **unique** (tracked as an instance with a stable UUID, e.g. a specific truck).
 * The registry table makes that nature authoritative and stable per type; it is filled on
 * first use. Stacks and instances reference it by `goodType`.
 *
 * Ownership is scoped by `holderType`/`holderId`. No physical location is tracked here:
 * where a good is in the game world is owned by the game server / persistence.
 */
import { sql } from 'drizzle-orm';
import {
  bigint,
  check,
  index,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from 'drizzle-orm/pg-core';

import { HOLDER_TYPES, type HolderType } from './holders.js';

/** Whether a good type is tracked as a fungible stack or as unique instances. */
export const GOOD_KINDS = ['stack', 'instance'] as const;
export type GoodKind = (typeof GOOD_KINDS)[number];

/** Registry making the nature (stack/instance) of each good type authoritative. */
export const goodTypes = pgTable('inventory_good_types', {
  goodType: text('good_type').primaryKey(),
  kind: text('kind').$type<GoodKind>().notNull(),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
});

export type GoodType = typeof goodTypes.$inferSelect;
export type NewGoodType = typeof goodTypes.$inferInsert;

/** Fungible goods: one row per (holder, good type). Quantity never negative. */
export const inventoryStacks = pgTable(
  'inventory_stacks',
  {
    holderType: text('holder_type').$type<HolderType>().notNull(),
    holderId: uuid('holder_id').notNull(),
    goodType: text('good_type').notNull(),
    quantity: bigint('quantity', { mode: 'number' }).notNull().default(0),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.holderType, t.holderId, t.goodType] }),
    check('inventory_stacks_quantity_positive', sql`${t.quantity} >= 0`),
    index('inventory_stacks_holder_idx').on(t.holderType, t.holderId),
    check('inventory_stacks_holder_type_valid', sql`${t.holderType} in ('player','npc','corporation','system')`),
  ],
);

export type InventoryStack = typeof inventoryStacks.$inferSelect;
export type NewInventoryStack = typeof inventoryStacks.$inferInsert;

/** Physical state of a unique instance, from this registry's point of view. */
export const INSTANCE_STATUSES = ['stored', 'in_world'] as const;
export type InstanceStatus = (typeof INSTANCE_STATUSES)[number];

/** Unique goods: one row per instance UUID, owned by exactly one holder. */
export const inventoryInstances = pgTable(
  'inventory_instances',
  {
    id: uuid('id').primaryKey(),
    goodType: text('good_type').notNull(),
    holderType: text('holder_type').$type<HolderType>().notNull(),
    holderId: uuid('holder_id').notNull(),
    status: text('status').$type<InstanceStatus>().notNull().default('stored'),
    metadata: jsonb('metadata').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index('inventory_instances_holder_idx').on(t.holderType, t.holderId),
    index('inventory_instances_good_type_idx').on(t.goodType),
    uniqueIndex('inventory_instances_id_good_unique').on(t.id, t.goodType),
  ],
);

export type InventoryInstance = typeof inventoryInstances.$inferSelect;
export type NewInventoryInstance = typeof inventoryInstances.$inferInsert;

/** Compiled holder-type list, exported for zod validation. */
export const HOLDER_TYPE_VALUES = [...HOLDER_TYPES] as const;
