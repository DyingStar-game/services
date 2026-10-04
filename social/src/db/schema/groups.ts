/**
 * Temporary groups: small player collectives used to claim and share missions.
 *
 * A player belongs to at most one group at a time (enforced by a unique index on
 * `group_members.playerId`). Groups have no expiry: they are disbanded manually by their
 * owner, and when the owner leaves, ownership passes to the oldest remaining member (an
 * empty group is deleted).
 */
import { index, integer, pgTable, primaryKey, text, timestamp, unique, uuid } from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

/** Minimum and maximum number of members (owner included) of a group. */
export const GROUP_MIN_MEMBERS = 2;
export const GROUP_MAX_MEMBERS = 100;

export const groups = pgTable(
  'groups',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    name: text('name').notNull().unique(),
    description: text('description'),
    /** Owner (leader); passes to the oldest member when they leave. */
    ownerId: uuid('owner_id')
      .notNull()
      .references(() => playerProfiles.playerId),
    /** Maximum number of members, owner included. */
    maxMembers: integer('max_members').notNull().default(10),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
);

export type Group = typeof groups.$inferSelect;

export const groupMembers = pgTable(
  'group_members',
  {
    groupId: uuid('group_id')
      .notNull()
      .references(() => groups.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.groupId, t.playerId] }),
    /** A player belongs to at most one group at a time. */
    unique('group_members_player_one_group').on(t.playerId),
    index('group_members_group_idx').on(t.groupId),
  ],
);

export type GroupMember = typeof groupMembers.$inferSelect;

/** Pending invitations from a group owner to a player. */
export const groupInvitations = pgTable(
  'group_invitations',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    groupId: uuid('group_id')
      .notNull()
      .references(() => groups.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    /** Inviting owner; null when their profile was removed. */
    invitedBy: uuid('invited_by').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [unique('group_invitations_pair_unique').on(t.groupId, t.playerId)],
);

export type GroupInvitation = typeof groupInvitations.$inferSelect;
