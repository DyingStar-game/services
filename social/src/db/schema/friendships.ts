/**
 * Friend requests and accepted friendships. One row per pair, whichever direction:
 * the service layer checks both orderings before inserting.
 */
import { sql } from 'drizzle-orm';
import { check, integer, pgTable, text, timestamp, unique, uuid } from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

export const FRIENDSHIP_STATUSES = ['pending', 'accepted'] as const;
export type FriendshipStatus = (typeof FRIENDSHIP_STATUSES)[number];

export const friendships = pgTable(
  'friendships',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    requesterId: uuid('requester_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    addresseeId: uuid('addressee_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    status: text('status').$type<FriendshipStatus>().notNull().default('pending'),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    unique('friendships_pair_unique').on(t.requesterId, t.addresseeId),
    check('friendships_not_self', sql`${t.requesterId} <> ${t.addresseeId}`),
  ],
);

export type Friendship = typeof friendships.$inferSelect;
