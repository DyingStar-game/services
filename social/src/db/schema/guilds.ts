/**
 * Guilds: identity, ranks with permissions, members, join requests and internal journal.
 * A player belongs to at most one guild.
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

export const GUILD_RECRUITMENT_MODES = ['open', 'apply', 'closed'] as const;
export type GuildRecruitmentMode = (typeof GUILD_RECRUITMENT_MODES)[number];

/** Permissions a rank can grant. The leader rank implicitly has all of them. */
export const GUILD_PERMISSIONS = [
  /** Edit name, tag, logo, description, recruitment mode. */
  'manage_guild',
  /** Create, edit and delete ranks. */
  'manage_ranks',
  /** Kick members and change their rank (only below one's own rank). */
  'manage_members',
  /** Invite players. */
  'invite',
  /** Accept or decline applications. */
  'recruit',
] as const;
export type GuildPermission = (typeof GUILD_PERMISSIONS)[number];

export const guilds = pgTable('guilds', {
  id: uuid('id').primaryKey().defaultRandom(),
  name: text('name').notNull().unique(),
  tag: text('tag').notNull().unique(),
  logoUrl: text('logo_url'),
  description: text('description'),
  recruitment: text('recruitment').$type<GuildRecruitmentMode>().notNull().default('apply'),
  ownerId: uuid('owner_id')
    .notNull()
    .references(() => playerProfiles.playerId),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

export type Guild = typeof guilds.$inferSelect;

export const guildRanks = pgTable(
  'guild_ranks',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    guildId: uuid('guild_id')
      .notNull()
      .references(() => guilds.id, { onDelete: 'cascade' }),
    name: text('name').notNull(),
    /** Higher wins; a member may only act on ranks strictly below their own. */
    priority: integer('priority').notNull().default(0),
    permissions: text('permissions')
      .array()
      .$type<GuildPermission[]>()
      .notNull()
      .default(sql`'{}'::text[]`),
    /** Exactly one per guild; held by the owner, undeletable, all permissions. */
    isLeader: boolean('is_leader').notNull().default(false),
    /** Rank given to new members; exactly one per guild. */
    isDefault: boolean('is_default').notNull().default(false),
  },
  (t) => [unique('guild_ranks_name_unique').on(t.guildId, t.name)],
);

export type GuildRank = typeof guildRanks.$inferSelect;

export const guildMembers = pgTable(
  'guild_members',
  {
    guildId: uuid('guild_id')
      .notNull()
      .references(() => guilds.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    rankId: integer('rank_id')
      .notNull()
      .references(() => guildRanks.id),
    joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.guildId, t.playerId] }),
    // One guild per player.
    unique('guild_members_player_unique').on(t.playerId),
  ],
);

export type GuildMember = typeof guildMembers.$inferSelect;

export const GUILD_REQUEST_KINDS = ['invitation', 'application'] as const;
export type GuildRequestKind = (typeof GUILD_REQUEST_KINDS)[number];

/** Pending invitations (guild → player) and applications (player → guild). */
export const guildJoinRequests = pgTable(
  'guild_join_requests',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    guildId: uuid('guild_id')
      .notNull()
      .references(() => guilds.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    kind: text('kind').$type<GuildRequestKind>().notNull(),
    message: text('message'),
    /** Inviting member for invitations; null for applications. */
    createdBy: uuid('created_by').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [unique('guild_join_requests_pair_unique').on(t.guildId, t.playerId)],
);

export type GuildJoinRequest = typeof guildJoinRequests.$inferSelect;

export const guildActivity = pgTable(
  'guild_activity',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    guildId: uuid('guild_id')
      .notNull()
      .references(() => guilds.id, { onDelete: 'cascade' }),
    actorId: uuid('actor_id').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    type: text('type').notNull(),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('guild_activity_guild_created_idx').on(t.guildId, t.createdAt.desc())],
);

export type GuildActivityEntry = typeof guildActivity.$inferSelect;
