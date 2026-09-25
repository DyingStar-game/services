/**
 * Wallet accounts: lazy creation (mirrors `ensureProfile` in Social), lookup helpers
 * and the reserved system tax vault.
 */
import { and, eq } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  accounts,
  type Account,
  type AccountHolderType,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/**
 * Fetches an account by id.
 * @param accountId - Account id.
 * @returns Account, or null if none.
 */
export async function getAccountById(accountId: string): Promise<Account | null> {
  const rows = await db.select().from(accounts).where(eq(accounts.id, accountId)).limit(1);
  return rows[0] ?? null;
}

/**
 * Fetches an account or throws 404.
 * @param accountId - Account id.
 * @returns Account.
 */
export async function requireAccount(accountId: string): Promise<Account> {
  const account = await getAccountById(accountId);
  if (!account) throw notFound(`Account ${accountId} not found`);
  return account;
}

/**
 * All accounts of a holder (one per currency).
 * @param holderType - Holder kind.
 * @param holderId - Player id or corporation id (opaque UUID).
 * @returns Accounts.
 */
export async function getAccounts(holderType: AccountHolderType, holderId: string): Promise<Account[]> {
  return db
    .select()
    .from(accounts)
    .where(and(eq(accounts.holderType, holderType), eq(accounts.holderId, holderId)))
    .orderBy(accounts.currency);
}

/** All wallet accounts of a player, one per currency. */
export function getPlayerAccounts(playerId: string): Promise<Account[]> {
  return getAccounts('player', playerId);
}

/** All wallet accounts of a corporation, one per currency. */
export function getCorporationAccounts(corporationId: string): Promise<Account[]> {
  return getAccounts('corporation', corporationId);
}

/**
 * Returns the account or creates it (defaulting to `credits`). Safe against concurrent
 * creation, like `ensureProfile` in Social.
 * @param holderType - Holder kind.
 * @param holderId - Player id or corporation id.
 * @param currency - Currency code.
 * @returns Existing or freshly created account.
 */
export async function ensureAccount(
  holderType: AccountHolderType,
  holderId: string,
  currency: string,
): Promise<Account> {
  try {
    const [created] = await db
      .insert(accounts)
      .values({ holderType, holderId, currency })
      .returning();
    return created;
  } catch (err) {
    if (!isUniqueViolation(err)) throw err;
    const raced = await getAccount(holderType, holderId, currency);
    if (raced) return raced;
    throw err;
  }
}

/**
 * Single-currency lookup by holder.
 * @param holderType - Holder kind.
 * @param holderId - Holder id.
 * @param currency - Currency code.
 * @returns Account, or null if none.
 */
export async function getAccount(
  holderType: AccountHolderType,
  holderId: string,
  currency: string,
): Promise<Account | null> {
  const rows = await db
    .select()
    .from(accounts)
    .where(and(eq(accounts.holderType, holderType), eq(accounts.holderId, holderId), eq(accounts.currency, currency)))
    .limit(1);
  return rows[0] ?? null;
}

/** Returns the player's account for a currency, creating it if needed. */
export function ensurePlayerAccount(playerId: string, currency = 'credits'): Promise<Account> {
  return ensureAccount('player', playerId, currency);
}

/** Returns the corporation's account for a currency, creating it if needed. */
export function ensureCorporationAccount(corporationId: string, currency = 'credits'): Promise<Account> {
  return ensureAccount('corporation', corporationId, currency);
}

/**
 * Rejects operations blocked by the account status (anything but `active`).
 * @param account - Account to check.
 */
export function requireActiveAccount(account: Account): void {
  if (account.status !== 'active') {
    throw new HttpError(403, 'ACCOUNT_LOCKED', `Account is ${account.status}`);
  }
}