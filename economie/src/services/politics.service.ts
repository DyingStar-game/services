/**
 * Political treasury: per-entity economic settings, mirrored membership (roles) and the
 * issuance of new currency. Political entities are owned by the Social service; `entityId`
 * is an opaque UUID. Tax debts are booked and settled by `taxation.service`.
 */
import { and, count, desc, eq } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { db } from '../db/connection.js';
import {
  politicalMembers,
  politicalSettings,
  politicalTaxDebts,
  type PoliticalMember,
  type PoliticalMemberHolderType,
  type PoliticalMemberRole,
  type PoliticalSettings,
  type PoliticalTaxDebt,
  type TaxDebtStatus,
  type TaxDebtorType,
} from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { ensurePoliticalAccount } from './accounts.service.js';
import { creditAccount, type MovementResult } from './transactions.service.js';

const ROLE_RANK: Record<PoliticalMemberRole, number> = { member: 1, treasurer: 2, head: 3 };

/**
 * Whether a membership grants at least `min` role (head > treasurer > member).
 * @param member - Current membership.
 * @param min - Minimum required role.
 * @returns True when allowed.
 */
export function hasPoliticalRole(member: PoliticalMember, min: PoliticalMemberRole): boolean {
  return ROLE_RANK[member.role] >= ROLE_RANK[min];
}

/**
 * Returns the entity's economic settings, creating the default row on first contact.
 * @param entityId - Political entity id.
 * @returns Settings.
 */
export async function getPoliticalSettings(entityId: string): Promise<PoliticalSettings> {
  const existing = await db
    .select()
    .from(politicalSettings)
    .where(eq(politicalSettings.entityId, entityId))
    .limit(1);
  if (existing[0]) return existing[0];
  const [created] = await db
    .insert(politicalSettings)
    .values({ entityId })
    .onConflictDoNothing()
    .returning();
  if (created) return created;
  const raced = await db
    .select()
    .from(politicalSettings)
    .where(eq(politicalSettings.entityId, entityId))
    .limit(1);
  return raced[0];
}

/**
 * Updates the tax rates and/or minting policy.
 * @param entityId - Political entity id.
 * @param patch - Fields to change.
 * @returns Updated settings.
 */
export async function updatePoliticalSettings(
  entityId: string,
  patch: { corporateTaxBps?: number; incomeTaxBps?: number; allowMinting?: boolean; mintCeiling?: number },
): Promise<PoliticalSettings> {
  await getPoliticalSettings(entityId);
  const [updated] = await db
    .update(politicalSettings)
    .set({ ...patch, updatedAt: new Date() })
    .where(eq(politicalSettings.entityId, entityId))
    .returning();
  return updated;
}

/**
 * Sets (add or update) a political member with a role.
 * @param entityId - Political entity id.
 * @param playerId - Player or NPC id.
 * @param role - Role to grant.
 * @param holderType - Whether the member is a player or an NPC (defaults to player).
 * @returns The membership.
 */
export async function setPoliticalMember(
  entityId: string,
  playerId: string,
  role: PoliticalMemberRole,
  holderType: PoliticalMemberHolderType = 'player',
): Promise<PoliticalMember> {
  const [row] = await db
    .insert(politicalMembers)
    .values({ entityId, playerId, role, holderType })
    .onConflictDoUpdate({
      target: [politicalMembers.entityId, politicalMembers.playerId],
      set: { role, holderType },
    })
    .returning();
  return row;
}

/**
 * Removes a member from a political entity's treasury.
 * @param entityId - Political entity id.
 * @param playerId - Player id.
 */
export async function removePoliticalMember(entityId: string, playerId: string): Promise<void> {
  await db
    .delete(politicalMembers)
    .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)));
}

/**
 * Members of a political entity, ordered by role then join date.
 * @param entityId - Political entity id.
 * @param limit - Max members in the page.
 * @param offset - Members to skip.
 * @returns Page of members.
 */
export async function listPoliticalMembers(entityId: string, limit: number, offset: number): Promise<Page<PoliticalMember>> {
  const condition = eq(politicalMembers.entityId, entityId);
  const [items, totalRows] = await Promise.all([
    db
      .select()
      .from(politicalMembers)
      .where(condition)
      .orderBy(politicalMembers.role, politicalMembers.joinedAt, politicalMembers.playerId)
      .limit(limit)
      .offset(offset),
    db.select({ total: count() }).from(politicalMembers).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/**
 * Tax debts of a debtor, newest first.
 * @param debtorType - Debtor kind.
 * @param debtorId - Debtor id.
 * @param opts - Optional status filter and page bounds.
 * @returns Page of debts.
 */
export async function getTaxDebts(
  debtorType: TaxDebtorType,
  debtorId: string,
  opts?: { status?: TaxDebtStatus; limit?: number; offset?: number },
): Promise<Page<PoliticalTaxDebt>> {
  const limit = opts?.limit ?? 20;
  const offset = opts?.offset ?? 0;
  const conditions = [eq(politicalTaxDebts.debtorType, debtorType), eq(politicalTaxDebts.debtorId, debtorId)];
  if (opts?.status) conditions.push(eq(politicalTaxDebts.status, opts.status));
  const condition = and(...conditions);
  const [items, totalRows] = await Promise.all([
    db
      .select()
      .from(politicalTaxDebts)
      .where(condition)
      .orderBy(desc(politicalTaxDebts.createdAt), desc(politicalTaxDebts.id))
      .limit(limit)
      .offset(offset),
    db.select({ total: count() }).from(politicalTaxDebts).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/**
 * Creates new currency for a political entity (a country or federation), crediting its
 * treasury. This increases the money supply and is recorded as an `issuance` transaction.
 * @param entityId - Political entity id.
 * @param amount - Amount to create, in minor units.
 * @param currency - Currency (default `credits`).
 * @param reason - Optional reference.
 * @param caller - Keycloak client id of the calling service.
 * @returns The ledger row and new balance.
 */
export async function issueCurrency(
  entityId: string,
  amount: number,
  currency = 'credits',
  reason?: string,
  caller?: string,
): Promise<MovementResult> {
  if (amount <= 0) throw new HttpError(400, 'INVALID_AMOUNT', t('amount.positive'));
  const settings = await getPoliticalSettings(entityId);
  if (!settings.allowMinting) {
    throw new HttpError(403, 'MINTING_DISABLED', 'This political entity is not allowed to create currency');
  }
  if (settings.mintCeiling > 0 && amount > settings.mintCeiling) {
    throw new HttpError(400, 'MINT_CEILING_EXCEEDED', `Amount exceeds the mint ceiling (${settings.mintCeiling})`, { ceiling: settings.mintCeiling });
  }
  const account = await ensurePoliticalAccount(entityId, currency);
  return creditAccount(account.id, amount, {
    currency,
    type: 'issuance',
    reference: reason,
    caller,
  });
}
