/**
 * Player assignments: a player accepts a mission, progresses it and is rewarded on
 * completion. `rewardExternalId` is the idempotency key handed to the Economy service so a
 * reward can never be paid twice (see `rewards.service`). Per-component settlement state
 * (`settledComponents`) makes partial retries safe when a mission has several reward
 * components.
 */
import { boolean, index, integer, jsonb, pgTable, text, timestamp, unique, uuid } from 'drizzle-orm/pg-core';

import { missions, type RewardComponent } from './missions.js';

/** Lifecycle of an assignee's participation in a mission. */
export const ASSIGNMENT_STATUSES = ['active', 'completed', 'abandoned', 'expired'] as const;
export type AssignmentStatus = (typeof ASSIGNMENT_STATUSES)[number];

/** Kinds of assignee (players and NPCs both have Economy wallets and Inventory holdings). */
export const ASSIGNEE_HOLDER_TYPES = ['player', 'npc'] as const;
export type AssigneeHolderType = (typeof ASSIGNEE_HOLDER_TYPES)[number];

export const missionAssignments = pgTable(
  'mission_assignments',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    missionId: uuid('mission_id')
      .notNull()
      .references(() => missions.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id').notNull(),
    /** Whether the assignee is a player or an NPC (drives reward routing). */
    holderType: text('holder_type').$type<AssigneeHolderType>().notNull().default('player'),
    status: text('status').$type<AssignmentStatus>().notNull().default('active'),
    acceptedAt: timestamp('accepted_at', { withTimezone: true }).notNull().defaultNow(),
    completedAt: timestamp('completed_at', { withTimezone: true }),
    /**
     * Reward shares frozen at completion, one entry per mission reward component
     * (multiplayer splits each component deterministically), so a settlement retry
     * credits/transfers the exact same amounts.
     */
    rewardShares: jsonb('reward_shares').$type<RewardComponent[]>(),
    /** Credits share of the frozen `rewardShares` entry (used by the escrow refund math). */
    rewardAmount: integer('reward_amount'),
    /** True once the economic reward has been credited by the Economy service. */
    rewardSettled: boolean('reward_settled').notNull().default(false),
    /** Idempotency key sent to Economy as the transaction `externalId`. */
    rewardExternalId: text('reward_external_id').notNull(),
    rewardSettledAt: timestamp('reward_settled_at', { withTimezone: true }),
    /** Raw result/error of the last settlement attempt. */
    rewardDetails: jsonb('reward_details').$type<Record<string, unknown>>(),
    /**
     * Reward components already granted (`credits`, `item:<itemId>`); each is settled at
     * most once, so a partial failure can be retried without double-paying.
     */
    settledComponents: text('settled_components')
      .array()
      .notNull()
      .$default(() => []),
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
