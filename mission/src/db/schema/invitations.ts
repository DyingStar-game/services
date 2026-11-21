/**
 * Invitations to join a mission already taken: a participant shares it with their group (every
 * member may join) or with one player (who accepts or declines). Joining opens an assignment of the
 * SAME mission: progress is shared, the reward split equally, any participant may turn it in.
 */
import { index, jsonb, pgTable, text, timestamp, uuid } from 'drizzle-orm/pg-core';

import { missions } from './missions.js';

export const INVITATION_STATUSES = ['pending', 'accepted', 'declined', 'revoked'] as const;
export type InvitationStatus = (typeof INVITATION_STATUSES)[number];

export const missionInvitations = pgTable(
  'mission_invitations',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    missionId: uuid('mission_id')
      .notNull()
      .references(() => missions.id, { onDelete: 'cascade' }),
    /** The participant who shared it. */
    fromPlayerId: uuid('from_player_id').notNull(),
    /** The invited player (a one-player invitation), or null for a group's. */
    toPlayerId: uuid('to_player_id'),
    /** The invited group (every member may join), or null for a one-player invitation. */
    groupId: uuid('group_id'),
    /** pending until answered; a group's stays pending, each member joining on their own. */
    status: text('status').$type<InvitationStatus>().notNull().default('pending'),
    /** The members who declined a group invitation (it stays open to the others). */
    declinedBy: jsonb('declined_by').$type<string[]>().notNull().default([]),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index('mission_invitations_to_player_idx').on(t.toPlayerId, t.status),
    index('mission_invitations_group_idx').on(t.groupId, t.status),
    index('mission_invitations_mission_idx').on(t.missionId),
  ],
);

export type MissionInvitation = typeof missionInvitations.$inferSelect;
