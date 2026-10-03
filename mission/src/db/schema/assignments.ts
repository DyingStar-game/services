/**
 * Player assignments: a player accepts a mission, progresses it and is rewarded on
 * completion. `rewardExternalId` is the idempotency key handed to the Economy service so a
 * reward can never be paid twice (see `rewards.service`).
 */
import { boolean, index, integer, jsonb, pgTable, text, timestamp, unique, uuid } from 'drizzle-orm/pg-core';

import { missions } from './missions.js';

/** Lifecycle of a player's participation in a mission. */
export const ASSIGNMENT_STATUSES = ['active', 'completed', 'abandoned', 'expired'] as const;
export type AssignmentStatus = (typeof ASSIGNMENT_STATUSES)[number];

export const missionAssignments = pgTable(
  'mission_assignments',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    missionId: uuid('mission_id')
      .notNull()
      .references(() => missions.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id').notNull(),
    status: text('status').$type<AssignmentStatus>().notNull().default('active'),
    acceptedAt: timestamp('accepted_at', { withTimezone: true }).notNull().defaultNow(),
    completedAt: timestamp('completed_at', { withTimezone: true }),
    /**
     * Reward share frozen at completion time (multiplayer splits the mission reward
     * deterministically), so a settlement retry credits the exact same amount.
     */
    rewardAmount: integer('reward_amount'),
    /** True once the economic reward has been credited by the Economy service. */
    rewardSettled: boolean('reward_settled').notNull().default(false),
    /** Idempotency key sent to Economy as the transaction `externalId`. */
    rewardExternalId: text('reward_external_id').notNull(),
    rewardSettledAt: timestamp('reward_settled_at', { withTimezone: true }),
    /** Raw result/error of the last settlement attempt. */
    rewardDetails: jsonb('reward_details').$type<Record<string, unknown>>(),
    /** Item reward share frozen at completion (fungible goods split like the economic share). */
    rewardItemQuantity: integer('reward_item_quantity'),
    /** True once the item reward has been transferred by the Inventory service. */
    itemSettled: boolean('item_settled').notNull().default(false),
    itemSettledAt: timestamp('item_settled_at', { withTimezone: true }),
    /** Raw result/error of the last item settlement attempt. */
    itemDetails: jsonb('item_details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    unique('mission_assignments_mission_player_unique').on(t.missionId, t.playerId),
    unique('mission_assignments_reward_external_id_unique').on(t.rewardExternalId),
    index('mission_assignments_player_idx').on(t.playerId, t.status),
    index('mission_assignments_mission_idx').on(t.missionId, t.status),
  ],
);

export type MissionAssignment = typeof missionAssignments.$inferSelect;
export type NewMissionAssignment = typeof missionAssignments.$inferInsert;
