/**
 * Ledger of every movement between accounts. Each row records the credited amount plus
 * any tax/fee collected, so the total debited from the payer is recoverable from one row.
 * Amounts are integer minor units.
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
  unique,
  uuid,
} from 'drizzle-orm/pg-core';

import { accounts } from './accounts.js';

export const TRANSACTION_TYPES = [
  'transfer',
  'deposit',
  'withdrawal',
  'tax',
  'fee',
  'mission_reward',
  'salary',
  'donation',
  'corporation_fund',
  'system',
] as const;
export type TransactionType = (typeof TRANSACTION_TYPES)[number];

export const transactions = pgTable(
  'transactions',
  {
    id: bigint('id', { mode: 'number' }).primaryKey().generatedAlwaysAsIdentity(),
    /** Null for system-created rows (tax collection) and unknown senders. */
    fromAccountId: uuid('from_account_id').references(() => accounts.id, { onDelete: 'set null' }),
    /** Null for withdrawals and internal rows. */
    toAccountId: uuid('to_account_id').references(() => accounts.id, { onDelete: 'set null' }),
    type: text('type').$type<TransactionType>().notNull(),
    currency: text('currency').notNull().default('credits'),
    /** Amount credited to `toAccountId`. */
    amount: bigint('amount', { mode: 'number' }).notNull(),
    /** Tax collected on the payer and routed to the system vault. */
    taxAmount: bigint('tax_amount', { mode: 'number' }).notNull().default(0),
    feeAmount: bigint('fee_amount', { mode: 'number' }).notNull().default(0),
    /** Idempotency key supplied by the caller (game server); unique when present. */
    externalId: text('external_id'),
    /** Business reference, e.g. a mission id. */
    reference: text('reference'),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    unique('transactions_external_id_unique').on(t.externalId),
    index('transactions_from_created_idx').on(t.fromAccountId, t.createdAt.desc()),
    index('transactions_to_created_idx').on(t.toAccountId, t.createdAt.desc()),
    index('transactions_created_idx').on(t.createdAt.desc()),
    check('transactions_amount_positive', sql`${t.amount} > 0`),
  ],
);

export type Transaction = typeof transactions.$inferSelect;
export type NewTransaction = typeof transactions.$inferInsert;