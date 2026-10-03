/**
 * Corporate payroll: the salary paid to members of a corporation's treasury. A default
 * amount is configured per role (leader / treasurer / member) and can be overridden per
 * member. Amounts are integer minor units, in a given currency.
 *
 * Membership itself lives in `corporation_members` (shared with the Social service);
 * these tables only carry the pay configuration.
 */
import { sql } from 'drizzle-orm';
import {
  bigint,
  boolean,
  check,
  foreignKey,
  index,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uuid,
} from 'drizzle-orm/pg-core';

import { corporationMembers, type CorporationRole } from './corporations.js';

/** Default salary attached to a role of a corporation. */
export const corporationSalaryRoles = pgTable(
  'corporation_salary_roles',
  {
    corporationId: uuid('corporation_id').notNull(),
    role: text('role').$type<CorporationRole>().notNull(),
    currency: text('currency').notNull().default('credits'),
    /** Salary in integer minor units; never negative. */
    amount: bigint('amount', { mode: 'number' }).notNull().default(0),
    enabled: boolean('enabled').notNull().default(true),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.corporationId, t.role, t.currency] }),
    check('corporation_salary_roles_amount_positive', sql`${t.amount} >= 0`),
  ],
);

export type CorporationSalaryRole = typeof corporationSalaryRoles.$inferSelect;
export type NewCorporationSalaryRole = typeof corporationSalaryRoles.$inferInsert;

/** Per-member salary override; takes precedence over the role default. */
export const corporationMemberSalaries = pgTable(
  'corporation_member_salaries',
  {
    corporationId: uuid('corporation_id').notNull(),
    playerId: uuid('player_id').notNull(),
    currency: text('currency').notNull().default('credits'),
    amount: bigint('amount', { mode: 'number' }).notNull().default(0),
    enabled: boolean('enabled').notNull().default(true),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.corporationId, t.playerId, t.currency] }),
    foreignKey({
      columns: [t.corporationId, t.playerId],
      foreignColumns: [corporationMembers.corporationId, corporationMembers.playerId],
    }).onDelete('cascade'),
    index('corporation_member_salaries_player_idx').on(t.playerId),
    check('corporation_member_salaries_amount_positive', sql`${t.amount} >= 0`),
  ],
);

export type CorporationMemberSalary = typeof corporationMemberSalaries.$inferSelect;
export type NewCorporationMemberSalary = typeof corporationMemberSalaries.$inferInsert;
