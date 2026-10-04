/**
 * Political treasury: per-entity economic settings, mirrored membership (roles) and the
 * issuance of new currency. Political entities are owned by the Social service; `entityId`
 * is an opaque UUID. Tax debts are booked and settled by `taxation.service`.
 */
import { and, desc, eq } from 'drizzle-orm';
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

/** All members of a political entity. */
export function listPoliticalMembers(entityId: string): Promise<PoliticalMember[]> {
  return db
    .select()
    .from(politicalMembers)
    .where(eq(politicalMembers.entityId, entityId))
    .orderBy(politicalMembers.role, politicalMembers.joinedAt);
}

/**
 * Tax debts of a debtor, newest first.
 * @param debtorType - Debtor kind.
 * @param debtorId - Debtor id.
 * @param status - Optional status filter.
 * @returns Debts.
 */
export function getTaxDebts(
  debtorType: TaxDebtorType,
  debtorId: string,
  status?: TaxDebtStatus,
): Promise<PoliticalTaxDebt[]> {
  const conditions = [eq(politicalTaxDebts.debtorType, debtorType), eq(politicalTaxDebts.debtorId, debtorId)];
  if (status) conditions.push(eq(politicalTaxDebts.status, status));
  return db
    .select()
    .from(politicalTaxDebts)
    .where(and(...conditions))
    .orderBy(desc(politicalTaxDebts.createdAt), desc(politicalTaxDebts.id));
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
