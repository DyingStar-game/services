/**
 * Corporate treasury: members and their roles (leader / treasurer / member), the
 * configurable internal tax and donations from members. Membership is keyed on the
 * opaque `corporationId` shared with the Social service.
 */
import { and, desc, eq, gte, inArray, lte, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  corporationMembers,
  corporationSettings,
  transactions,
  type CorporationMember,
  type CorporationRole,
  type CorporationSettings,
  type Transaction,
} from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { ensureCorporationAccount, ensurePlayerAccount, getCorporationAccounts } from './accounts.service.js';
import { transfer, type MovementResult } from './transactions.service.js';

const ROLE_RANK: Record<CorporationRole, number> = { member: 1, treasurer: 2, leader: 3 };

/**
 * Whether a membership grants at least `min` role (leader > treasurer > member).
 * @param member - Current membership.
 * @param min - Minimum required role.
 * @returns True when allowed.
 */
export function hasCorporationRole(member: CorporationMember, min: CorporationRole): boolean {
  return ROLE_RANK[member.role] >= ROLE_RANK[min];
}

/**
 * Fetches a player's membership in a corporation.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 * @returns Membership, or null.
 */
export async function getCorporationMember(
  corporationId: string,
  playerId: string,
): Promise<CorporationMember | null> {
  const rows = await db
    .select()
    .from(corporationMembers)
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * Requires a membership, else throws 403.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 * @returns Membership.
 */
export async function requireCorporationMember(
  corporationId: string,
  playerId: string,
): Promise<CorporationMember> {
  const member = await getCorporationMember(corporationId, playerId);
  if (!member) throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'You are not a member of this corporation');
  return member;
}

/**
 * Requires a membership holding at least `min`.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 * @param min - Minimum role.
 * @returns Membership.
 */
export async function requireCorporationRole(
  corporationId: string,
  playerId: string,
  min: CorporationRole,
): Promise<CorporationMember> {
  const member = await requireCorporationMember(corporationId, playerId);
  if (!hasCorporationRole(member, min)) {
    throw new HttpError(403, 'FORBIDDEN', `Requires the ${min} role in this corporation`);
  }
  return member;
}

/**
 * Sets (add or update) a member with a role.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 * @param role - Role to grant.
 * @returns The membership.
 */
export async function setCorporationMember(
  corporationId: string,
  playerId: string,
  role: CorporationRole,
): Promise<CorporationMember> {
  const [row] = await db
    .insert(corporationMembers)
    .values({ corporationId, playerId, role })
    .onConflictDoUpdate({
      target: [corporationMembers.corporationId, corporationMembers.playerId],
      set: { role },
    })
    .returning();
  return row;
}

/**
 * Removes a member from a corporation's treasury.
 * @param corporationId - Corporation id.
 * @param playerId - Player id.
 */
export async function removeCorporationMember(corporationId: string, playerId: string): Promise<void> {
  await db
    .delete(corporationMembers)
    .where(and(eq(corporationMembers.corporationId, corporationId), eq(corporationMembers.playerId, playerId)));
}

/** All members of a corporation. */
export function listCorporationMembers(corporationId: string): Promise<CorporationMember[]> {
  return db
    .select()
    .from(corporationMembers)
    .where(eq(corporationMembers.corporationId, corporationId))
    .orderBy(corporationMembers.role, corporationMembers.joinedAt);
}

/**
 * Returns the corporation's economic settings, creating the default row on first contact.
 * @param corporationId - Corporation id.
 * @returns Settings.
 */
export async function getCorporationSettings(corporationId: string): Promise<CorporationSettings> {
  const existing = await db
    .select()
    .from(corporationSettings)
    .where(eq(corporationSettings.corporationId, corporationId))
    .limit(1);
  if (existing[0]) return existing[0];
  const [created] = await db
    .insert(corporationSettings)
    .values({ corporationId })
    .onConflictDoNothing()
    .returning();
  if (created) return created;
  const raced = await db
    .select()
    .from(corporationSettings)
    .where(eq(corporationSettings.corporationId, corporationId))
    .limit(1);
  return raced[0];
}

/**
 * Updates the internal tax rate and/or donation policy.
 * @param corporationId - Corporation id.
 * @param patch - Fields to change.
 * @returns Updated settings.
 */
export async function updateCorporationSettings(
  corporationId: string,
  patch: { taxRateBps?: number; allowDonations?: boolean },
): Promise<CorporationSettings> {
  const [updated] = await db
    .update(corporationSettings)
    .set({ ...patch, updatedAt: new Date() })
    .where(eq(corporationSettings.corporationId, corporationId))
    .returning();
  return updated;
}

/**
 * Donation from a member's wallet to the corporation treasury. The configured internal
 * tax is deducted from the payer and stays within the corporation account.
 * @param fromPlayerId - Donor player id (must be a member).
 * @param corporationId - Corporation id.
 * @param amount - Amount in minor units.
 * @param memo - Optional note.
 * @returns The ledger row and balances.
 */
export async function donate(
  fromPlayerId: string,
  corporationId: string,
  amount: number,
  memo?: string,
): Promise<MovementResult> {
  await requireCorporationMember(corporationId, fromPlayerId);
  const settings = await getCorporationSettings(corporationId);
  if (!settings.allowDonations) {
    throw new HttpError(403, 'DONATIONS_DISABLED', 'Donations are disabled for this corporation');
  }
  const [fromAccount, toAccount] = await Promise.all([
    ensurePlayerAccount(fromPlayerId),
    ensureCorporationAccount(corporationId),
  ]);
  return transfer({
    fromAccountId: fromAccount.id,
    toAccountId: toAccount.id,
    amount,
    type: 'donation',
    taxBps: settings.taxRateBps,
    taxToAccountId: toAccount.id,
    reference: memo ? 'member_donation' : undefined,
    details: memo ? { memo } : undefined,
  });
}

/** Totals per currency over a report period. */
export interface CorporationTotals {
  currency: string;
  inflow: number;
  outflow: number;
}

/** Per-direction breakdown tracked for the report. */
interface DirectionBucket {
  count: number;
  inflow: number;
  outflow: number;
}

/** Financial report of a corporation: inflow/outflow totals and a breakdown by type. */
export interface CorporationReport {
  corporationId: string;
  from?: Date;
  to?: Date;
  totals: CorporationTotals[];
  byType: { type: string; currency: string; count: number; inflow: number; outflow: number }[];
}

/**
 * Builds the treasury report over every account of the corporation (all currencies).
 * Inflow counts the credited amount (plus any tax routed back to the corporation),
 * outflow the full amount debited from the corporation (tax included).
 * @param corporationId - Corporation id.
 * @param from - Start of the period (inclusive).
 * @param to - End of the period (inclusive).
 * @returns Aggregated report.
 */
export async function getCorporationReport(
  corporationId: string,
  from?: Date,
  to?: Date,
): Promise<CorporationReport> {
  const corporationAccounts = await getCorporationAccounts(corporationId);
  const ids = corporationAccounts.map((a) => a.id);
  if (ids.length === 0) return { corporationId, from, to, totals: [], byType: [] };

  const conditions = [or(inArray(transactions.toAccountId, ids), inArray(transactions.fromAccountId, ids))];
  if (from) conditions.push(gte(transactions.createdAt, from));
  if (to) conditions.push(lte(transactions.createdAt, to));

  const rows = await db
    .select()
    .from(transactions)
    .where(and(...conditions))
    .orderBy(desc(transactions.createdAt));

  /** True when the collected tax of this row was routed back to the corporation account. */
  function taxStayed(row: Transaction): boolean {
    return row.details?.taxTo === row.toAccountId;
  }

  const byType = new Map<string, DirectionBucket>();
  const byCurrency = new Map<string, DirectionBucket>();
  const bump = (map: Map<string, DirectionBucket>, key: string, inflow: number, outflow: number) => {
    const bucket = map.get(key) ?? { count: 0, inflow: 0, outflow: 0 };
    bucket.count += 1;
    bucket.inflow += inflow;
    bucket.outflow += outflow;
    map.set(key, bucket);
  };

  for (const row of rows) {
    const inflow = row.toAccountId && ids.includes(row.toAccountId) ? row.amount + (taxStayed(row) ? row.taxAmount : 0) : 0;
    const outflow = row.fromAccountId && ids.includes(row.fromAccountId) ? row.amount + row.taxAmount : 0;
    if (inflow > 0 || outflow > 0) {
      bump(byType, `${row.type}|${row.currency}`, inflow, outflow);
      bump(byCurrency, row.currency, inflow, outflow);
    }
  }

  const totals = [...byCurrency.entries()].map(([currency, b]) => ({ currency, inflow: b.inflow, outflow: b.outflow }));
  const typeRows = [...byType.entries()].map(([key, b]) => {
    const [type, currency] = key.split('|');
    return { type, currency, count: b.count, inflow: b.inflow, outflow: b.outflow };
  });
  return { corporationId, from, to, totals, byType: typeRows };
}