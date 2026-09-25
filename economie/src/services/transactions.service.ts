/**
 * Money movements: atomic transfers with automatic taxes, internal credits/debits with
 * `externalId` idempotency, and ledger listings. Amounts are integer minor units.
 */
import { and, desc, eq, gte, inArray, or, sql } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  accounts,
  transactions,
  type Account,
  type Transaction,
  type TransactionType,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { getCorporationAccounts, getPlayerAccounts, requireActiveAccount } from './accounts.service.js';

const PG_UNIQUE_VIOLATION = '23505';

type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/** Context required to run a money movement (used to keep the ledger consistent). */
interface MovementContext {
  accountId: string;
  currency: string;
  amount: number;
  type: TransactionType;
  reference?: string;
  details?: Record<string, unknown>;
  externalId?: string;
}

/** Outcome of a movement: the ledger row and the new balances involved. */
export interface MovementResult {
  transaction: Transaction;
  amount: number;
  taxAmount: number;
  fromBalance?: number;
  toBalance?: number;
}

/**
 * Fetches the transaction already recorded for an `externalId`, if any.
 * @param externalId - Idempotency key.
 * @returns Existing transaction, or null.
 */
async function findByIdempotencyKey(externalId: string): Promise<Transaction | null> {
  const rows = await db
    .select()
    .from(transactions)
    .where(eq(transactions.externalId, externalId))
    .limit(1);
  return rows[0] ?? null;
}

/** Locks and returns an account, checking status; throws when unusable. */
async function loadActive(tx: Tx, accountId: string, label: string): Promise<Account> {
  const [row] = await tx.select().from(accounts).where(eq(accounts.id, accountId)).for('update');
  if (!row) throw notFound(`${label} account ${accountId} not found`);
  requireActiveAccount(row);
  return row;
}

/** Credits `amount` on an account (atomic increment). */
async function credit(tx: Tx, accountId: string, amount: number): Promise<number> {
  const [row] = await tx
    .update(accounts)
    .set({ balance: sql`${accounts.balance} + ${amount}`, updatedAt: new Date() })
    .where(eq(accounts.id, accountId))
    .returning({ balance: accounts.balance });
  return row.balance;
}

/** Debits `amount` on an account; returns the new balance or throws 409 when insufficient. */
async function debit(tx: Tx, accountId: string, amount: number): Promise<number> {
  const [row] = await tx
    .update(accounts)
    .set({ balance: sql`${accounts.balance} - ${amount}`, updatedAt: new Date() })
    .where(and(eq(accounts.id, accountId), gte(accounts.balance, amount)))
    .returning({ balance: accounts.balance });
  if (!row) throw new HttpError(409, 'INSUFFICIENT_FUNDS', 'Insufficient balance');
  return row.balance;
}

/** Returns (or creates, inside the transaction) the account receiving the tax. */
async function resolveTaxVault(tx: Tx, currency: string, taxToAccountId?: string): Promise<Account | null> {
  if (taxToAccountId) {
    return loadActive(tx, taxToAccountId, 'Tax recipient');
  }
  const systemVault = [accounts.holderType, accounts.holderId, accounts.currency] as const;
  const existing = await tx
    .select()
    .from(accounts)
    .where(
      and(
        eq(accounts.holderType, 'system'),
        eq(accounts.holderId, env.economy.taxVaultUuid),
        eq(accounts.currency, currency),
      ),
    )
    .limit(1);
  if (existing[0]) return existing[0];
  const [created] = await tx
    .insert(accounts)
    .values({ holderType: 'system', holderId: env.economy.taxVaultUuid, currency })
    .onConflictDoNothing({ target: [systemVault[0], systemVault[1], systemVault[2]] })
    .returning();
  if (created) return created;
  const raced = await tx
    .select()
    .from(accounts)
    .where(
      and(
        eq(accounts.holderType, 'system'),
        eq(accounts.holderId, env.economy.taxVaultUuid),
        eq(accounts.currency, currency),
      ),
    )
    .limit(1);
  return raced[0] ?? null;
}

/**
 * Computes the automatic tax on a gross amount (basis points, with an optional ceiling).
 * @param amount - Gross amount in minor units.
 * @param taxBps - Tax rate in basis points.
 * @returns Tax in minor units.
 */
export function computeTax(amount: number, taxBps: number): number {
  const tax = Math.floor((amount * taxBps) / 10_000);
  const ceiling = env.economy.transferTaxCeiling;
  return ceiling > 0 ? Math.min(tax, ceiling) : tax;
}

/**
 * Transfers funds between two accounts, collecting an automatic tax from the payer.
 * The payer is debited `amount + tax`; the receiver is credited `amount`; the tax goes
 * to the system vault (or `taxToAccountId`, e.g. the corporation itself for donations).
 * @param opts - Transfer parameters.
 * @returns The ledger row plus balances.
 */
export async function transfer(opts: {
  fromAccountId: string;
  toAccountId: string;
  amount: number;
  currency?: string;
  type?: TransactionType;
  taxBps?: number;
  taxToAccountId?: string;
  reference?: string;
  details?: Record<string, unknown>;
  externalId?: string;
}): Promise<MovementResult> {
  const { fromAccountId, toAccountId, amount, currency = 'credits', type = 'transfer' } = opts;
  if (amount <= 0) throw new HttpError(400, 'INVALID_AMOUNT', 'Amount must be positive');
  if (fromAccountId === toAccountId) throw new HttpError(400, 'INVALID_TARGET', 'Cannot transfer to the same account');
  if (opts.externalId && (await findByIdempotencyKey(opts.externalId))) {
    throw new HttpError(409, 'DUPLICATE_EXTERNAL_ID', 'Transaction already recorded');
  }

  const taxAmount = computeTax(amount, opts.taxBps ?? env.economy.transferTaxBps);

  try {
    return await db.transaction(async (tx) => {
      const from = await loadActive(tx, fromAccountId, 'Sender');
      const to = await loadActive(tx, toAccountId, 'Receiver');
      const total = amount + taxAmount;

      const fromBalance = await debit(tx, from.id, total);
      let toBalance = await credit(tx, to.id, amount);

      if (taxAmount > 0) {
        const vault = await resolveTaxVault(tx, currency, opts.taxToAccountId);
        if (vault) {
          // When the tax stays in the receiving account (donations), this second credit
          // targets the same account, so the real balance reflects the collected tax.
          const vaultBalance = await credit(tx, vault.id, taxAmount);
          if (vault.id === to.id) toBalance = vaultBalance;
        }
      }

      const [transaction] = await tx
        .insert(transactions)
        .values({
          fromAccountId: from.id,
          toAccountId: to.id,
          type,
          currency,
          amount,
          taxAmount,
          reference: opts.reference,
          details: opts.details
            ? { ...opts.details, ...(taxAmount > 0 ? { taxTo: opts.taxToAccountId ?? 'system' } : {}) }
            : taxAmount > 0
              ? { taxTo: opts.taxToAccountId ?? 'system' }
              : undefined,
          externalId: opts.externalId ?? null,
        })
        .returning();
      return { transaction, amount, taxAmount, fromBalance, toBalance };
    });
  } catch (err) {
    if (opts.externalId && isUniqueViolation(err)) {
      const existing = await findByIdempotencyKey(opts.externalId);
      if (existing) throw new HttpError(409, 'DUPLICATE_EXTERNAL_ID', 'Transaction already recorded');
    }
    throw err;
  }
}

/**
 * Generic single-sided movement: a credit (inbound) or a debit (outbound) reported by
 * the game server or another trusted service. Idempotent via `externalId`.
 * @param direction - `credit` or `debit`.
 * @param opts - Movement parameters.
 * @returns The ledger row plus the new balance.
 */
async function movement(
  direction: 'credit' | 'debit',
  opts: MovementContext,
): Promise<MovementResult> {
  const { accountId, amount, currency, type, reference, details, externalId } = opts;
  if (amount <= 0) throw new HttpError(400, 'INVALID_AMOUNT', 'Amount must be positive');
  if (externalId && (await findByIdempotencyKey(externalId))) {
    throw new HttpError(409, 'DUPLICATE_EXTERNAL_ID', 'Transaction already recorded');
  }

  try {
    return await db.transaction(async (tx) => {
      const account = await loadActive(tx, accountId, 'Target');
      const balance =
        direction === 'credit' ? await credit(tx, account.id, amount) : await debit(tx, account.id, amount);
      const [transaction] = await tx
        .insert(transactions)
        .values({
          fromAccountId: direction === 'debit' ? account.id : null,
          toAccountId: direction === 'credit' ? account.id : null,
          type,
          currency,
          amount,
          reference,
          details,
          externalId: externalId ?? null,
        })
        .returning();
      if (direction === 'credit') {
        return { transaction, amount, taxAmount: 0, toBalance: balance };
      }
      return { transaction, amount, taxAmount: 0, fromBalance: balance };
    });
  } catch (err) {
    if (externalId && isUniqueViolation(err)) {
      const existing = await findByIdempotencyKey(externalId);
      if (existing) throw new HttpError(409, 'DUPLICATE_EXTERNAL_ID', 'Transaction already recorded');
    }
    throw err;
  }
}

/** Credits an account (e.g. mission rewards, salaries). */
export function creditAccount(
  accountId: string,
  amount: number,
  opts: Partial<MovementContext> = {},
): Promise<MovementResult> {
  return movement('credit', {
    accountId,
    amount,
    currency: opts.currency ?? 'credits',
    type: opts.type ?? 'deposit',
    reference: opts.reference,
    details: opts.details,
    externalId: opts.externalId,
  });
}

/** Debits an account, rejecting insufficient balances. */
export function debitAccount(
  accountId: string,
  amount: number,
  opts: Partial<MovementContext> = {},
): Promise<MovementResult> {
  return movement('debit', {
    accountId,
    amount,
    currency: opts.currency ?? 'credits',
    type: opts.type ?? 'withdrawal',
    reference: opts.reference,
    details: opts.details,
    externalId: opts.externalId,
  });
}

/**
 * Ledger rows touching any of the given accounts, newest first.
 * @param accountIds - Account ids.
 * @param limit - Max rows.
 * @returns Transactions.
 */
export async function listTransactionsFor(accountIds: string[], limit: number): Promise<Transaction[]> {
  if (accountIds.length === 0) return [];
  return db
    .select()
    .from(transactions)
    .where(or(inArray(transactions.fromAccountId, accountIds), inArray(transactions.toAccountId, accountIds)))
    .orderBy(desc(transactions.createdAt), desc(transactions.id))
    .limit(limit);
}

/** Ledger of one account. */
export function listAccountTransactions(accountId: string, limit: number): Promise<Transaction[]> {
  return listTransactionsFor([accountId], limit);
}

/** Ledger of all of a player's accounts (one per currency). */
export async function listPlayerTransactions(playerId: string, limit: number): Promise<Transaction[]> {
  const playerAccounts = await getPlayerAccounts(playerId);
  return listTransactionsFor(playerAccounts.map((a) => a.id), limit);
}

/** Ledger of all of a corporation's accounts. */
export async function listCorporationTransactions(corporationId: string, limit: number): Promise<Transaction[]> {
  const corporationAccounts = await getCorporationAccounts(corporationId);
  return listTransactionsFor(corporationAccounts.map((a) => a.id), limit);
}