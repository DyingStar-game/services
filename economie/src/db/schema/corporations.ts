/**
 * Corporate treasury: the members with the leader/treasurer roles and the configurable
 * internal tax. Both are keyed on the opaque `corporationId` shared with the Social
 * service, independent of the per-currency wallet accounts.
 */
import { boolean, index, integer, pgTable, primaryKey, text, timestamp, uuid } from 'drizzle-orm/pg-core';

/** Roles in a corporation's treasury, ranked: leader > treasurer > member. */
export const CORPORATION_ROLES = ['leader', 'treasurer', 'member'] as const;
export type CorporationRole = (typeof CORPORATION_ROLES)[number];

export const corporationMembers = pgTable(
  'corporation_members',
  {
    corporationId: uuid('corporation_id').notNull(),
    /** Player id (Keycloak subject) — opaque here, owned by the Social service. */
    playerId: uuid('player_id').notNull(),
    role: text('role').$type<CorporationRole>().notNull().default('member'),
    joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.corporationId, t.playerId] }),
    index('corporation_members_player_idx').on(t.playerId),
  ],
);

export type CorporationMember = typeof corporationMembers.$inferSelect;
export type NewCorporationMember = typeof corporationMembers.$inferInsert;

/** Per-corporation economic configuration. */
export const corporationSettings = pgTable('corporation_settings', {
  corporationId: uuid('corporation_id').primaryKey(),
  /** Internal tax on member donations, in basis points (0-10000). */
  taxRateBps: integer('tax_rate_bps').notNull().default(0),
  allowDonations: boolean('allow_donations').notNull().default(true),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

export type CorporationSettings = typeof corporationSettings.$inferSelect;
export type NewCorporationSettings = typeof corporationSettings.$inferInsert;