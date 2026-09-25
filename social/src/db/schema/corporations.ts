/**
 * Corporations: identity, ranks with permissions, members, join requests and internal journal.
 * A player or NPC belongs to at most one corporation.
 */
import { sql } from 'drizzle-orm';
import {
  boolean,
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

import { playerProfiles } from './profiles.js';

export const CORPORATION_RECRUITMENT_MODES = ['open', 'apply', 'closed'] as const;
export type CorporationRecruitmentMode = (typeof CORPORATION_RECRUITMENT_MODES)[number];

/** Permissions a rank can grant. The CEO rank implicitly has all of them. */
export const CORPORATION_PERMISSIONS = [
  /** Edit name, ticker, logo, description, recruitment mode. */
  'manage_corporation',
  /** Create, edit and delete ranks. */
  'manage_ranks',
  /** Kick members and change their rank (only below one's own rank). */
  'manage_members',
  /** Invite players. */
  'invite',
  /** Accept or decline applications. */
  'recruit',
] as const;
export type CorporationPermission = (typeof CORPORATION_PERMISSIONS)[number];

export const corporations = pgTable('corporations', {
  id: uuid('id').primaryKey().defaultRandom(),
  name: text('name').notNull().unique(),
  ticker: text('ticker').notNull().unique(),
  logoUrl: text('logo_url'),
  description: text('description'),
  recruitment: text('recruitment').$type<CorporationRecruitmentMode>().notNull().default('apply'),
  ceoId: uuid('ceo_id')
    .notNull()
    .references(() => playerProfiles.playerId),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

export type Corporation = typeof corporations.$inferSelect;

export const corporationRanks = pgTable(
  'corporation_ranks',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    corporationId: uuid('corporation_id')
      .notNull()
      .references(() => corporations.id, { onDelete: 'cascade' }),
    name: text('name').notNull(),
    /** Higher wins; a member may only act on ranks strictly below their own. */
    priority: integer('priority').notNull().default(0),
    permissions: text('permissions')
      .array()
      .$type<CorporationPermission[]>()
      .notNull()
      .default(sql`'{}'::text[]`),
    /** Exactly one per corporation; held by the CEO, undeletable, all permissions. */
    isCeo: boolean('is_ceo').notNull().default(false),
    /** Rank given to new members; exactly one per corporation. */
    isDefault: boolean('is_default').notNull().default(false),
  },
  (t) => [unique('corporation_ranks_name_unique').on(t.corporationId, t.name)],
);

export type CorporationRank = typeof corporationRanks.$inferSelect;

export const corporationMembers = pgTable(
  'corporation_members',
  {
    corporationId: uuid('corporation_id')
      .notNull()
      .references(() => corporations.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    rankId: integer('rank_id')
      .notNull()
      .references(() => corporationRanks.id),
    joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.corporationId, t.playerId] }),
    // One corporation per player/NPC.
    unique('corporation_members_player_unique').on(t.playerId),
  ],
);

export type CorporationMember = typeof corporationMembers.$inferSelect;

export const CORPORATION_REQUEST_KINDS = ['invitation', 'application'] as const;
export type CorporationRequestKind = (typeof CORPORATION_REQUEST_KINDS)[number];

/** Pending invitations (corporation → player) and applications (player → corporation). */
export const corporationJoinRequests = pgTable(
  'corporation_join_requests',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    corporationId: uuid('corporation_id')
      .notNull()
      .references(() => corporations.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    kind: text('kind').$type<CorporationRequestKind>().notNull(),
    message: text('message'),
    /** Inviting member for invitations; null for applications. */
    createdBy: uuid('created_by').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [unique('corporation_join_requests_pair_unique').on(t.corporationId, t.playerId)],
);

export type CorporationJoinRequest = typeof corporationJoinRequests.$inferSelect;

export const corporationActivity = pgTable(
  'corporation_activity',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    corporationId: uuid('corporation_id')
      .notNull()
      .references(() => corporations.id, { onDelete: 'cascade' }),
    actorId: uuid('actor_id').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    type: text('type').notNull(),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('corporation_activity_corporation_created_idx').on(t.corporationId, t.createdAt.desc())],
);

export type CorporationActivityEntry = typeof corporationActivity.$inferSelect;