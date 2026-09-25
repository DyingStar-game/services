/**
 * Wallet accounts: a player or corporation (or the system tax vault) owns zero or more
 * accounts, one per currency. Balances are stored as integer minor units and never go
 * below zero (debt is handled later by economic reputation).
 *
 * `holderId` is an opaque UUID: the Keycloak subject for players, or the `corporationId`
 * of the Social service for corporations (economy has no local copy of those tables).
 */
import { sql } from 'drizzle-orm';
import { bigint, check, index, pgTable, text, timestamp, unique, uuid } from 'drizzle-orm/pg-core';

/** Account owners. `system` is reserved for the tax vault (see `ECONOMY_TAX_VAULT_UUID`). */
export const ACCOUNT_HOLDER_TYPES = ['player', 'corporation', 'system'] as const;
export type AccountHolderType = (typeof ACCOUNT_HOLDER_TYPES)[number];

export const ACCOUNT_STATUSES = ['active', 'locked', 'frozen'] as const;
export type AccountStatus = (typeof ACCOUNT_STATUSES)[number];

export const accounts = pgTable(
  'accounts',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    holderType: text('holder_type').$type<AccountHolderType>().notNull(),
    holderId: uuid('holder_id').notNull(),
    /** Currency code; `credits` is the universal currency. */
    currency: text('currency').notNull().default('credits'),
    /** Balance in integer minor units, never negative. */
    balance: bigint('balance', { mode: 'number' }).notNull().default(0),
    status: text('status').$type<AccountStatus>().notNull().default('active'),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    unique('accounts_holder_currency_unique').on(t.holderType, t.holderId, t.currency),
    index('accounts_holder_idx').on(t.holderType, t.holderId),
    check('accounts_balance_positive', sql`${t.balance} >= 0`),
  ],
);

export type Account = typeof accounts.$inferSelect;
export type NewAccount = typeof accounts.$inferInsert;