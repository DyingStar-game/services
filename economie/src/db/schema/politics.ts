/**
 * Political treasury: the mirrored membership (with roles), the per-entity tax/minting
 * configuration and the tax debts owed by corporations and citizens.
 *
 * Political entities themselves live in the Social service; `entityId` is an opaque UUID here.
 * Taxes are like rent: an assessment computes and books a debt, then the debtor (corporation
 * for its corporate tax, player/NPC for its income tax) triggers the payment.
 */
import { sql } from 'drizzle-orm';
import {
  bigint,
  boolean,
  check,
  index,
  integer,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  unique,
  uuid,
} from 'drizzle-orm/pg-core';

/** Roles in a political entity's treasury, ranked: head > treasurer > member. */
export const POLITICAL_MEMBER_ROLES = ['head', 'treasurer', 'member'] as const;
export type PoliticalMemberRole = (typeof POLITICAL_MEMBER_ROLES)[number];

/** Kinds of political member (players and NPCs have wallets). */
export const POLITICAL_MEMBER_HOLDER_TYPES = ['player', 'npc'] as const;
export type PoliticalMemberHolderType = (typeof POLITICAL_MEMBER_HOLDER_TYPES)[number];

export const politicalMembers = pgTable(
  'political_members',
  {
    entityId: uuid('entity_id').notNull(),
    /** Player or NPC id — opaque here, owned by the Social service. */
    playerId: uuid('player_id').notNull(),
    holderType: text('holder_type')
      .$type<PoliticalMemberHolderType>()
      .notNull()
      .default('player'),
    role: text('role').$type<PoliticalMemberRole>().notNull().default('member'),
    joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.entityId, t.playerId] }),
    index('political_members_player_idx').on(t.playerId),
  ],
);

export type PoliticalMember = typeof politicalMembers.$inferSelect;
export type NewPoliticalMember = typeof politicalMembers.$inferInsert;

/** Per-entity economic configuration (tax rates and minting policy). */
export const politicalSettings = pgTable(
  'political_settings',
  {
    entityId: uuid('entity_id').primaryKey(),
    /** Corporate tax on the corporation treasury balance, in basis points (0-10000). */
    corporateTaxBps: integer('corporate_tax_bps').notNull().default(0),
    /** Income tax on citizen income (salaries/rewards) since the last assessment, in basis points. */
    incomeTaxBps: integer('income_tax_bps').notNull().default(0),
    /** Whether this entity may create currency (countries and federations only, enforced upstream). */
    allowMinting: boolean('allow_minting').notNull().default(false),
    /** Absolute ceiling of a single mint; 0 = no ceiling. */
    mintCeiling: bigint('mint_ceiling', { mode: 'number' }).notNull().default(0),
    /** Start of the next income-tax window (set by each assessment). */
    lastAssessedAt: timestamp('last_assessed_at', { withTimezone: true }),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    check('political_settings_corporate_tax_bps_range', sql`${t.corporateTaxBps} >= 0 AND ${t.corporateTaxBps} <= 10000`),
    check('political_settings_income_tax_bps_range', sql`${t.incomeTaxBps} >= 0 AND ${t.incomeTaxBps} <= 10000`),
    check('political_settings_mint_ceiling_positive', sql`${t.mintCeiling} >= 0`),
  ],
);

export type PoliticalSettings = typeof politicalSettings.$inferSelect;
export type NewPoliticalSettings = typeof politicalSettings.$inferInsert;

/** Debtors that can owe a political tax. */
export const TAX_DEBTOR_TYPES = ['player', 'npc', 'corporation'] as const;
export type TaxDebtorType = (typeof TAX_DEBTOR_TYPES)[number];

/** Kinds of tax debt. */
export const TAX_TYPES = ['corporate_tax', 'income_tax'] as const;
export type TaxType = (typeof TAX_TYPES)[number];

export const TAX_DEBT_STATUSES = ['due', 'paid', 'cancelled'] as const;
export type TaxDebtStatus = (typeof TAX_DEBT_STATUSES)[number];

/** A tax debt booked by an assessment and settled by its debtor. */
export const politicalTaxDebts = pgTable(
  'political_tax_debts',
  {
    id: bigint('id', { mode: 'number' }).primaryKey().generatedAlwaysAsIdentity(),
    /** Groups every debt booked by one assessment run (idempotency key). */
    assessmentId: uuid('assessment_id').notNull(),
    entityId: uuid('entity_id').notNull(),
    debtorType: text('debtor_type').$type<TaxDebtorType>().notNull(),
    debtorId: uuid('debtor_id').notNull(),
    currency: text('currency').notNull().default('credits'),
    /** Amount owed, in integer minor units; never zero. */
    amount: bigint('amount', { mode: 'number' }).notNull(),
    taxType: text('tax_type').$type<TaxType>().notNull(),
    status: text('status').$type<TaxDebtStatus>().notNull().default('due'),
    /** Assessment base and rate used to compute the debt. */
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    paidAt: timestamp('paid_at', { withTimezone: true }),
  },
  (t) => [
    unique('political_tax_debts_assessment_unique').on(t.assessmentId, t.debtorType, t.debtorId),
    index('political_tax_debts_debtor_idx').on(t.debtorType, t.debtorId, t.status),
    index('political_tax_debts_entity_idx').on(t.entityId, t.status),
    check('political_tax_debts_amount_positive', sql`${t.amount} > 0`),
  ],
);

export type PoliticalTaxDebt = typeof politicalTaxDebts.$inferSelect;
export type NewPoliticalTaxDebt = typeof politicalTaxDebts.$inferInsert;
