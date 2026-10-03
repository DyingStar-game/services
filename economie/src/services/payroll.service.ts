/**
 * Corporate payroll: salary configuration (per role, with per-member overrides) and
 * payouts from the corporation treasury to its members. Salary is recurring and paid on
 * demand by a leader/treasurer; primes are one-off payments.
 *
 * Amounts are integer minor units. Payouts are atomic: either every member is paid or
 * none is (treasury balance is checked under a row lock).
 */
import { and, eq, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  accounts,
  corporationMemberSalaries,
  corporationMembers,
  corporationSalaryRoles,
  transactions,
  type CorporationMemberSalary,
  type CorporationRole,
  type CorporationSalaryRole,
  type TransactionType,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { ensureCorporationAccount, ensurePlayerAccount, requireActiveAccount } from './accounts.service.js';

type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

/** Effective salary of one member, after resolving the per-member override. */
export interface EffectiveSalary {
  playerId: string;
  role: CorporationRole;
  currency: string;
  amount: number;
  /** `member` when the amount comes from a per-member override, `role` otherwise. */
  source: 'member' | 'role';
}

/** Salary configuration of a corporation. */
export interface CorporationSalaries {
  corporationId: string;
  roleDefaults: CorporationSalaryRole[];
  memberOverrides: CorporationMemberSalary[];
}

/** Outcome of a payroll run. */
export interface PayrollResult {
  corporationId: string;
  currency: string;
  paid: { playerId: string; amount: number; source: 'member' | 'role' }[];
  skipped: string[];
  total: number;
}

/**
 * Salary configuration of a corporation (role defaults + member overrides).
 * @param corporationId - Corporation id.
 * @returns Configuration.
 */
export async function getSalaries(corporationId: string): Promise<CorporationSalaries> {
  const [roleDefaults, memberOverrides] = await Promise.all([
    db.select().from(corporationSalaryRoles).where(eq(corporationSalaryRoles.corporationId, corporationId)).orderBy(corporationSalaryRoles.role),
    db
      .select()
      .from(corporationMemberSalaries)
      .where(eq(corporationMemberSalaries.corporationId, corporationId))
      .orderBy(corporationMemberSalaries.playerId),
  ]);
  return { corporationId, roleDefaults, memberOverrides };
}

/**
 * Sets (upserts) the default salary for a role.
 * @param corporationId - Corporation id.
 * @param role - Role the salary applies to.
 * @param patch - Amount and/or enabled flag and currency.
 * @returns The stored role salary.
 */
export async function setRoleSalary(
  corporationId: string,
  role: CorporationRole,
  patch: { amount: number; currency?: string; enabled?: boolean },
): Promise<CorporationSalaryRole> {
  const currency = patch.currency ?? 'credits';
  const [row] = await db
    .insert(corporationSalaryRoles)
    .values({ corporationId, role, currency, amount: patch.amount, enabled: patch.enabled ?? true })
    .onConflictDoUpdate({
      target: [corporationSalaryRoles.corporationId, corporationSalaryRoles.role, corporationSalaryRoles.currency],
      set: { amount: patch.amount, enabled: patch.enabled ?? true, updatedAt: new Date() },
    })
    .returning();
  return row;
}

/**
 * Sets (upserts) a per-member salary override. The player must be a member.
 * @param corporationId - Corporation id.
 * @param playerId - Member player id.
 * @param patch - Amount and/or enabled flag and currency.
 * @returns The stored member salary.
 */
export async function setMemberSalary(
  corporationId: string,
  playerId: string,
  patch: { amount: number; currency?: string; enabled?: boolean },
): Promise<CorporationMemberSalary> {
  const membership = await db
    .select()
    .from(corporationMembers)
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)))
    .limit(1);
  if (!membership[0]) throw notFound(`Player ${playerId} is not a member of this corporation`);

  const currency = patch.currency ?? 'credits';
  const [row] = await db
    .insert(corporationMemberSalaries)
    .values({ corporationId, playerId, currency, amount: patch.amount, enabled: patch.enabled ?? true })
    .onConflictDoUpdate({
      target: [corporationMemberSalaries.corporationId, corporationMemberSalaries.playerId, corporationMemberSalaries.currency],
      set: { amount: patch.amount, enabled: patch.enabled ?? true, updatedAt: new Date() },
    })
    .returning();
  return row;
}

/**
 * Removes a per-member salary override (the role default applies again).
 * @param corporationId - Corporation id.
 * @param playerId - Member player id.
 */
export async function removeMemberSalary(corporationId: string, playerId: string): Promise<void> {
  await db
    .delete(corporationMemberSalaries)
    .where(and(eq(corporationMemberSalaries.corporationId, corporationId), eq(corporationMemberSalaries.playerId, playerId)));
}

/**
 * Resolves the effective salary of every member with an enabled, positive amount.
 * @param corporationId - Corporation id.
 * @param currency - Currency to consider.
 * @returns Effective salaries.
 */
async function resolveEffectiveSalaries(corporationId: string, currency: string): Promise<EffectiveSalary[]> {
  const [members, roleDefaults, overrides] = await Promise.all([
    db.select().from(corporationMembers).where(eq(corporationMembers.corporationId, corporationId)),
    db
      .select()
      .from(corporationSalaryRoles)
      .where(and(eq(corporationSalaryRoles.corporationId, corporationId), eq(corporationSalaryRoles.currency, currency))),
    db
      .select()
      .from(corporationMemberSalaries)
      .where(and(eq(corporationMemberSalaries.corporationId, corporationId), eq(corporationMemberSalaries.currency, currency))),
  ]);

  const roleAmount = new Map(roleDefaults.map((r) => [r.role, r]));
  const memberAmount = new Map(overrides.map((r) => [r.playerId, r]));

  const effective: EffectiveSalary[] = [];
  for (const member of members) {
    const override = memberAmount.get(member.playerId);
    const fallback = roleAmount.get(member.role);
    const source: 'member' | 'role' = override ? 'member' : 'role';
    const row = override ?? fallback;
    if (!row || !row.enabled || row.amount <= 0) continue;
    effective.push({ playerId: member.playerId, role: member.role, currency, amount: row.amount, source });
  }
  return effective;
}

/** Locks and returns the corporation treasury account for a currency. */
async function loadTreasury(tx: Tx, corporationId: string, currency: string): Promise<typeof accounts.$inferSelect> {
  const [row] = await tx
    .select()
    .from(accounts)
    .where(
      and(
        eq(accounts.holderType, 'corporation'),
        eq(accounts.holderId, corporationId),
        eq(accounts.currency, currency),
      ),
    )
    .for('update');
  if (!row) throw notFound(`Treasury account for ${corporationId} in ${currency} not found`);
  requireActiveAccount(row);
  return row;
}

/** Credits `amount` on an account (atomic increment), returning the new balance. */
async function credit(tx: Tx, accountId: string, amount: number): Promise<number> {
  const [row] = await tx
    .update(accounts)
    .set({ balance: sql`${accounts.balance} + ${amount}`, updatedAt: new Date() })
    .where(eq(accounts.id, accountId))
    .returning({ balance: accounts.balance });
  return row.balance;
}

/**
 * Records one payout row inside the given transaction.
 * @param tx - Transaction.
 * @param fromAccountId - Treasury account debited.
 * @param toAccountId - Member account credited.
 * @param amount - Amount in minor units.
 * @param type - Ledger type (`salary` or `prime`).
 * @param currency - Currency.
 * @param details - Extra ledger metadata.
 */
async function recordPayout(
  tx: Tx,
  fromAccountId: string,
  toAccountId: string,
  amount: number,
  type: TransactionType,
  currency: string,
  details?: Record<string, unknown>,
): Promise<void> {
  await tx.insert(transactions).values({
    fromAccountId,
    toAccountId,
    type,
    currency,
    amount,
    reference: type,
    details,
  });
}

/**
 * Pays every eligible member's salary from the corporation treasury, in one currency.
 * A member receives their override when configured, otherwise their role default.
 * The treasury is debited once for the total; insufficient funds abort the whole run.
 * @param corporationId - Corporation id.
 * @param currency - Currency to pay in.
 * @returns The list of payouts and the total.
 */
export async function runPayroll(corporationId: string, currency = 'credits'): Promise<PayrollResult> {
  const effective = await resolveEffectiveSalaries(corporationId, currency);
  const allMemberIds = await db
    .select({ playerId: corporationMembers.playerId })
    .from(corporationMembers)
    .where(eq(corporationMembers.corporationId, corporationId));
  const paidIds = new Set(effective.map((e) => e.playerId));
  const skipped = allMemberIds.map((m) => m.playerId).filter((id) => !paidIds.has(id));

  if (effective.length === 0) {
    return { corporationId, currency, paid: [], skipped, total: 0 };
  }

  const total = effective.reduce((sum, e) => sum + e.amount, 0);
  const treasury = await ensureCorporationAccount(corporationId, currency);
  const memberAccounts = new Map<string, string>();
  await Promise.all(
    effective.map(async (e) => {
      const account = await ensurePlayerAccount(e.playerId, currency);
      memberAccounts.set(e.playerId, account.id);
    }),
  );

  await db.transaction(async (tx) => {
    const treasuryRow = await loadTreasury(tx, corporationId, currency);
    if (treasuryRow.balance < total) {
      throw new HttpError(
        409,
        'INSUFFICIENT_FUNDS',
        `Treasury needs ${total} but holds ${treasuryRow.balance} ${currency}`,
      );
    }
    const [debited] = await tx
      .update(accounts)
      .set({ balance: sql`${accounts.balance} - ${total}`, updatedAt: new Date() })
      .where(and(eq(accounts.id, treasury.id), sql`${accounts.balance} >= ${total}`))
      .returning({ balance: accounts.balance });
    if (!debited) throw new HttpError(409, 'INSUFFICIENT_FUNDS', 'Insufficient treasury balance');

    for (const e of effective) {
      const toAccountId = memberAccounts.get(e.playerId)!;
      await credit(tx, toAccountId, e.amount);
      await recordPayout(tx, treasury.id, toAccountId, e.amount, 'salary', currency, {
        role: e.role,
        source: e.source,
      });
    }
  });

  return {
    corporationId,
    currency,
    paid: effective.map((e) => ({ playerId: e.playerId, amount: e.amount, source: e.source })),
    skipped,
    total,
  };
}

/**
 * Pays a one-off prime to a member from the corporation treasury.
 * @param corporationId - Corporation id.
 * @param playerId - Member player id.
 * @param amount - Amount in minor units.
 * @param options - Optional currency and memo.
 * @returns The new treasury and member balances.
 */
export async function payPrime(
  corporationId: string,
  playerId: string,
  amount: number,
  options: { currency?: string; memo?: string } = {},
): Promise<{ transaction: typeof transactions.$inferSelect; treasuryBalance: number; memberBalance: number }> {
  const currency = options.currency ?? 'credits';
  const membership = await db
    .select()
    .from(corporationMembers)
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)))
    .limit(1);
  if (!membership[0]) throw notFound(`Player ${playerId} is not a member of this corporation`);

  const [treasury, memberAccount] = await Promise.all([
    ensureCorporationAccount(corporationId, currency),
    ensurePlayerAccount(playerId, currency),
  ]);

  const result = await db.transaction(async (tx) => {
    const treasuryRow = await loadTreasury(tx, corporationId, currency);
    if (treasuryRow.balance < amount) {
      throw new HttpError(409, 'INSUFFICIENT_FUNDS', 'Insufficient treasury balance');
    }
    const [debited] = await tx
      .update(accounts)
      .set({ balance: sql`${accounts.balance} - ${amount}`, updatedAt: new Date() })
      .where(and(eq(accounts.id, treasury.id), sql`${accounts.balance} >= ${amount}`))
      .returning({ balance: accounts.balance });
    if (!debited) throw new HttpError(409, 'INSUFFICIENT_FUNDS', 'Insufficient treasury balance');
    const memberBalance = await credit(tx, memberAccount.id, amount);
    const [transaction] = await tx
      .insert(transactions)
      .values({
        fromAccountId: treasury.id,
        toAccountId: memberAccount.id,
        type: 'prime',
        currency,
        amount,
        reference: 'prime',
        details: options.memo ? { memo: options.memo, role: membership[0].role } : { role: membership[0].role },
      })
      .returning();
    return { transaction, treasuryBalance: debited.balance, memberBalance };
  });

  return result;
}
