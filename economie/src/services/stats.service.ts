/**
 * Economic analytics for the admin dashboard: money supply, transaction volume, taxes
 * collected, richest holders and a daily time series.
 *
 * Note: `economie` stores holder ids as opaque UUIDs (no display names); the admin UI can
 * resolve names through the Social service.
 */
import { and, count, desc, eq, gte, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { accounts, transactions } from '../db/schema/index.js';

/** Money supply per currency. */
export interface MoneySupplyBucket {
  currency: string;
  total: number;
  accounts: number;
}

/** A holder ranking entry (player, NPC or corporation). */
export interface RichestHolder {
  holderId: string;
  currency: string;
  balance: number;
  /** Pseudonym, resolved through Social for the admin dashboard (null when unknown). */
  displayName?: string | null;
}

/** One day of the volume time series. */
export interface DailySeriesPoint {
  date: string;
  transactions: number;
  volume: number;
  tax: number;
}

/** Aggregate movements over the period. */
export interface TransactionTotals {
  count: number;
  volume: number;
  tax: number;
  fee: number;
}

/** Full admin dashboard payload. */
export interface EconomyStats {
  moneySupply: MoneySupplyBucket[];
  transactions: { total: TransactionTotals; period: TransactionTotals };
  richestPlayers: RichestHolder[];
  richestNpcs: RichestHolder[];
  richestCorporations: RichestHolder[];
  series: DailySeriesPoint[];
}

/**
 * Computes the admin dashboard figures.
 * @param days - Length of the time series and the "period" window (default 30).
 * @param top - Number of richest holders per ranking (default 10).
 * @returns Aggregates, rankings and the daily series.
 */
export async function getEconomyStats(days = 30, top = 10): Promise<EconomyStats> {
  const since = new Date(Date.now() - days * 86_400_000);

  const [supplyRows, totalRows, periodRows, richestPlayers, richestNpcs, richestCorporations, seriesRows] =
    await Promise.all([
    db
      .select({
        currency: accounts.currency,
        total: sql<number>`coalesce(sum(${accounts.balance}), 0)::bigint`,
        accounts: count(),
      })
      .from(accounts)
      .groupBy(accounts.currency)
      .orderBy(accounts.currency),
    db
      .select({
        count: count(),
        volume: sql<number>`coalesce(sum(${transactions.amount}), 0)::bigint`,
        tax: sql<number>`coalesce(sum(${transactions.taxAmount}), 0)::bigint`,
        fee: sql<number>`coalesce(sum(${transactions.feeAmount}), 0)::bigint`,
      })
      .from(transactions),
    db
      .select({
        count: count(),
        volume: sql<number>`coalesce(sum(${transactions.amount}), 0)::bigint`,
        tax: sql<number>`coalesce(sum(${transactions.taxAmount}), 0)::bigint`,
        fee: sql<number>`coalesce(sum(${transactions.feeAmount}), 0)::bigint`,
      })
      .from(transactions)
      .where(gte(transactions.createdAt, since)),
    db
      .select({ holderId: accounts.holderId, currency: accounts.currency, balance: accounts.balance })
      .from(accounts)
      .where(and(eq(accounts.holderType, 'player'), eq(accounts.status, 'active')))
      .orderBy(desc(accounts.balance))
      .limit(top),
    db
      .select({ holderId: accounts.holderId, currency: accounts.currency, balance: accounts.balance })
      .from(accounts)
      .where(and(eq(accounts.holderType, 'npc'), eq(accounts.status, 'active')))
      .orderBy(desc(accounts.balance))
      .limit(top),
    db
      .select({ holderId: accounts.holderId, currency: accounts.currency, balance: accounts.balance })
      .from(accounts)
      .where(and(eq(accounts.holderType, 'corporation'), eq(accounts.status, 'active')))
      .orderBy(desc(accounts.balance))
      .limit(top),
    db
      .select({
        date: sql<string>`to_char(date_trunc('day', ${transactions.createdAt}), 'YYYY-MM-DD')`,
        transactions: count(),
        volume: sql<number>`coalesce(sum(${transactions.amount}), 0)::bigint`,
        tax: sql<number>`coalesce(sum(${transactions.taxAmount}), 0)::bigint`,
      })
      .from(transactions)
      .where(gte(transactions.createdAt, since))
      .groupBy(sql`date_trunc('day', ${transactions.createdAt})`)
      .orderBy(sql`date_trunc('day', ${transactions.createdAt})`),
  ]);

  const toTotals = (rows: { count: number; volume: number; tax: number; fee: number }[]): TransactionTotals => {
    const row = rows[0] ?? { count: 0, volume: 0, tax: 0, fee: 0 };
    return {
      count: Number(row.count),
      volume: Number(row.volume),
      tax: Number(row.tax),
      fee: Number(row.fee),
    };
  };

  return {
    moneySupply: supplyRows.map((r) => ({ currency: r.currency, total: Number(r.total), accounts: Number(r.accounts) })),
    transactions: { total: toTotals(totalRows), period: toTotals(periodRows) },
    richestPlayers: richestPlayers.map((r) => ({ holderId: r.holderId, currency: r.currency, balance: Number(r.balance) })),
    richestNpcs: richestNpcs.map((r) => ({ holderId: r.holderId, currency: r.currency, balance: Number(r.balance) })),
    richestCorporations: richestCorporations.map((r) => ({
      holderId: r.holderId,
      currency: r.currency,
      balance: Number(r.balance),
    })),
    series: seriesRows.map((r) => ({
      date: r.date,
      transactions: Number(r.transactions),
      volume: Number(r.volume),
      tax: Number(r.tax),
    })),
  };
}
