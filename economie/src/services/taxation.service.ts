/**
 * Political taxation, modelled on rent: an assessment computes the amount due and books it
 * as a debt; the debtor then triggers the payment.
 *
 * - Corporate tax: a share of each attached corporation's treasury balance.
 * - Income tax: a share of each member's income (salaries/primes/mission rewards) received
 *   since the entity's previous assessment.
 *
 * Payments are atomic and partial-tolerant: every affordable debt is settled, the rest stay due.
 */
import { randomUUID } from 'crypto';
import { and, asc, eq, gt, inArray, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  accounts,
  corporationSettings,
  politicalMembers,
  politicalSettings,
  politicalTaxDebts,
  transactions,
  type Account,
  type NewPoliticalTaxDebt,
  type TaxDebtorType,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { ensureAccount, ensurePoliticalAccount, requireActiveAccount } from './accounts.service.js';
import { getPoliticalSettings } from './politics.service.js';

/** Transaction types counted as taxable citizen income. */
const INCOME_TYPES = ['salary', 'prime', 'mission_reward'] as const;

type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

/** Outcome of an assessment run. */
export interface AssessmentResult {
  entityId: string;
  assessmentId: string;
  currency: string;
  /** Start of the income window used (previous assessment, or now on the first run). */
  since: string;
  corporateDebts: number;
  incomeDebts: number;
  booked: number;
}

/** A settled tax debt. */
export interface SettledDebt {
  debtId: number;
  entityId: string;
  amount: number;
}

/** Outcome of a payment run. */
export interface TaxPaymentResult {
  debtorType: TaxDebtorType;
  debtorId: string;
  paid: SettledDebt[];
  paidTotal: number;
  /** Number of debts still due after this payment. */
  remaining: number;
}

/**
 * Computes and books the tax debts of every taxpayer attached to a political entity.
 * Idempotent per run via the generated `assessmentId`; updates the income-tax window.
 * @param entityId - Political entity id.
 * @param currency - Currency to assess (default `credits`).
 * @returns Assessment summary.
 */
export async function assessTaxes(entityId: string, currency = 'credits'): Promise<AssessmentResult> {
  const settings = await getPoliticalSettings(entityId);
  const now = new Date();
  // On the first assessment, only future income is taxed (no historical windfall).
  const since = settings.lastAssessedAt ?? now;
  const assessmentId = randomUUID();
  const debts: NewPoliticalTaxDebt[] = [];

  // Corporate tax: attached corporations, on their treasury balance.
  const attached = await db
    .select({ corporationId: corporationSettings.corporationId })
    .from(corporationSettings)
    .where(eq(corporationSettings.politicalEntityId, entityId));
  if (attached.length > 0 && settings.corporateTaxBps > 0) {
    const balances = await db
      .select({ holderId: accounts.holderId, balance: accounts.balance })
      .from(accounts)
      .where(
        and(
          eq(accounts.holderType, 'corporation'),
          eq(accounts.currency, currency),
          inArray(accounts.holderId, attached.map((c) => c.corporationId)),
        ),
      );
    const byCorp = new Map(balances.map((b) => [b.holderId, Number(b.balance)]));
    for (const c of attached) {
      const base = byCorp.get(c.corporationId) ?? 0;
      const amount = Math.floor((base * settings.corporateTaxBps) / 10_000);
      if (amount > 0) {
        debts.push({
          assessmentId,
          entityId,
          debtorType: 'corporation',
          debtorId: c.corporationId,
          currency,
          amount,
          taxType: 'corporate_tax',
          details: { base, rateBps: settings.corporateTaxBps },
        });
      }
    }
  }

  // Income tax: members, on the income received since the previous assessment.
  const members = await db.select().from(politicalMembers).where(eq(politicalMembers.entityId, entityId));
  if (members.length > 0 && settings.incomeTaxBps > 0) {
    const memberIds = members.map((m) => m.playerId);
    const incomeRows = await db
      .select({ holderId: accounts.holderId, total: sql<number>`coalesce(sum(${transactions.amount}), 0)::bigint` })
      .from(transactions)
      .innerJoin(accounts, eq(accounts.id, transactions.toAccountId))
      .where(
        and(
          inArray(accounts.holderId, memberIds),
          inArray(accounts.holderType, ['player', 'npc']),
          gt(transactions.createdAt, since),
          inArray(transactions.type, [...INCOME_TYPES]),
        ),
      )
      .groupBy(accounts.holderId);
    const income = new Map(incomeRows.map((r) => [r.holderId, Number(r.total)]));
    for (const m of members) {
      const base = income.get(m.playerId) ?? 0;
      const amount = Math.floor((base * settings.incomeTaxBps) / 10_000);
      if (amount > 0) {
        debts.push({
          assessmentId,
          entityId,
          debtorType: m.holderType,
          debtorId: m.playerId,
          currency,
          amount,
          taxType: 'income_tax',
          details: { base, rateBps: settings.incomeTaxBps, since: since.toISOString() },
        });
      }
    }
  }

  let booked = 0;
  if (debts.length > 0) {
    const inserted = await db
      .insert(politicalTaxDebts)
      .values(debts)
      .onConflictDoNothing()
      .returning({ id: politicalTaxDebts.id });
    booked = inserted.length;
  }
  await db
    .update(politicalSettings)
    .set({ lastAssessedAt: now, updatedAt: new Date() })
    .where(eq(politicalSettings.entityId, entityId));

  return {
    entityId,
    assessmentId,
    currency,
    since: since.toISOString(),
    corporateDebts: debts.filter((d) => d.taxType === 'corporate_tax').length,
    incomeDebts: debts.filter((d) => d.taxType === 'income_tax').length,
    booked,
  };
}

/** Locks an account for update and caches the locked balance. */
async function lockAccount(tx: Tx, cache: Map<string, { account: Account; balance: number }>, accountId: string) {
  const cached = cache.get(accountId);
  if (cached) return cached;
  const [row] = await tx.select().from(accounts).where(eq(accounts.id, accountId)).for('update');
  if (!row) throw notFound(`Account ${accountId} not found`);
  requireActiveAccount(row);
  const entry = { account: row, balance: Number(row.balance) };
  cache.set(accountId, entry);
  return entry;
}

/**
 * Settles every affordable due debt of a debtor (oldest first). Unaffordable debts stay due.
 * @param debtorType - Debtor kind.
 * @param debtorId - Debtor id.
 * @param opts - Optional currency and/or entity filter.
 * @returns Payment summary.
 */
export async function payTaxDebts(
  debtorType: TaxDebtorType,
  debtorId: string,
  opts: { currency?: string; entityId?: string } = {},
): Promise<TaxPaymentResult> {
  const conditions = [
    eq(politicalTaxDebts.debtorType, debtorType),
    eq(politicalTaxDebts.debtorId, debtorId),
    eq(politicalTaxDebts.status, 'due'),
  ];
  if (opts.currency) conditions.push(eq(politicalTaxDebts.currency, opts.currency));
  if (opts.entityId) conditions.push(eq(politicalTaxDebts.entityId, opts.entityId));

  const due = await db
    .select()
    .from(politicalTaxDebts)
    .where(and(...conditions))
    .orderBy(asc(politicalTaxDebts.createdAt), asc(politicalTaxDebts.id));

  if (due.length === 0) {
    return { debtorType, debtorId, paid: [], paidTotal: 0, remaining: 0 };
  }

  // Ensure the debtor's and the entities' accounts exist before locking inside the transaction.
  const debtorAccounts = new Map<string, string>();
  const entityAccounts = new Map<string, string>();
  for (const cur of new Set(due.map((d) => d.currency))) {
    const account = await ensureAccount(debtorType, debtorId, cur);
    debtorAccounts.set(cur, account.id);
    for (const entityId of new Set(due.filter((d) => d.currency === cur).map((d) => d.entityId))) {
      const entityAccount = await ensurePoliticalAccount(entityId, cur);
      entityAccounts.set(`${entityId}:${cur}`, entityAccount.id);
    }
  }

  const paid: SettledDebt[] = [];
  let paidTotal = 0;

  await db.transaction(async (tx) => {
    const cache = new Map<string, { account: Account; balance: number }>();
    for (const debt of due) {
      const debtor = await lockAccount(tx, cache, debtorAccounts.get(debt.currency)!);
      if (debtor.balance < debt.amount) continue;
      const entity = await lockAccount(tx, cache, entityAccounts.get(`${debt.entityId}:${debt.currency}`)!);

      debtor.balance -= debt.amount;
      entity.balance += debt.amount;
      await tx
        .update(accounts)
        .set({ balance: debtor.balance, updatedAt: new Date() })
        .where(eq(accounts.id, debtor.account.id));
      await tx
        .update(accounts)
        .set({ balance: entity.balance, updatedAt: new Date() })
        .where(eq(accounts.id, entity.account.id));
      await tx.insert(transactions).values({
        fromAccountId: debtor.account.id,
        toAccountId: entity.account.id,
        type: 'tax',
        currency: debt.currency,
        amount: debt.amount,
        reference: `tax_debt:${debt.id}`,
        details: { taxType: debt.taxType, entityId: debt.entityId, assessmentId: debt.assessmentId },
      });
      await tx
        .update(politicalTaxDebts)
        .set({ status: 'paid', paidAt: new Date() })
        .where(eq(politicalTaxDebts.id, debt.id));

      paid.push({ debtId: debt.id, entityId: debt.entityId, amount: debt.amount });
      paidTotal += debt.amount;
    }
  });

  const remainingRows = await db
    .select({ count: sql<number>`count(*)::int` })
    .from(politicalTaxDebts)
    .where(and(...conditions));
  const remaining = Number(remainingRows[0]?.count ?? 0);

  if (paid.length === 0 && remaining > 0) {
    throw new HttpError(409, 'INSUFFICIENT_FUNDS', 'Insufficient balance to settle any due tax debt');
  }

  return { debtorType, debtorId, paid, paidTotal, remaining };
}
