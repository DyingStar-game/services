/**
 * Missions and their verifiable objectives.
 *
 * A mission is either **dynamic** (generated, e.g. by an IA corporation or a city later on),
 * **scenario** (hand-authored, possibly a chain of ordered objectives via `scriptId`) or
 * **player**-created. Each objective carries the target quantity the game server (or the
 * player) reports against; a mission is completable only once every objective is met.
 */
import { index, integer, jsonb, pgTable, text, timestamp, uniqueIndex, uuid } from 'drizzle-orm/pg-core';

/** How a mission came to be. */
export const MISSION_KINDS = ['dynamic', 'scenario', 'player'] as const;
export type MissionKind = (typeof MISSION_KINDS)[number];

/** High-level mission category, used for filtering and future reward logic. */
export const MISSION_CATEGORIES = ['delivery', 'transport', 'generic'] as const;
export type MissionCategory = (typeof MISSION_CATEGORIES)[number];

/** Who issued the mission. `corporation`/`city` ids are opaque (owned by Social / the game). */
export const MISSION_ISSUER_TYPES = ['system', 'corporation', 'city', 'player'] as const;
export type MissionIssuerType = (typeof MISSION_ISSUER_TYPES)[number];

/** Lifecycle of a mission as a whole. */
export const MISSION_STATUSES = ['available', 'active', 'completed', 'cancelled', 'expired'] as const;
export type MissionStatus = (typeof MISSION_STATUSES)[number];

/** Who may accept a mission. Corporation missions are restricted to members (checked in Social). */
export const MISSION_VISIBILITIES = ['public', 'corporation'] as const;
export type MissionVisibility = (typeof MISSION_VISIBILITIES)[number];

/** Escrow lifecycle for player-funded rewards (economic only). */
export const ESCROW_STATUSES = ['none', 'pending', 'held', 'refunded', 'claimed'] as const;
export type EscrowStatus = (typeof ESCROW_STATUSES)[number];

/** Kinds of verifiable objectives. */
export const OBJECTIVE_TYPES = ['deliver_material', 'transport', 'visit', 'custom'] as const;
export type ObjectiveType = (typeof OBJECTIVE_TYPES)[number];

/** Lifecycle of a single objective. */
export const OBJECTIVE_STATUSES = ['pending', 'in_progress', 'completed', 'failed'] as const;
export type ObjectiveStatus = (typeof OBJECTIVE_STATUSES)[number];

/**
 * Reward attached to a mission. `economic` is settled by the Economy service, `item` by the
 * Inventory service (ownership transfer from the escrow payer or the system faucet).
 * `instanceId` marks a unique-instance reward (single assignee only).
 */
export interface MissionReward {
  economic?: { currency: string; amount: number };
  item?: { itemId: string; quantity: number; instanceId?: string };
}

/** Item escrow lifecycle for player-funded item rewards. */
export const ITEM_ESCROW_STATUSES = ['none', 'held', 'claimed', 'released'] as const;
export type ItemEscrowStatus = (typeof ITEM_ESCROW_STATUSES)[number];

export const missions = pgTable(
  'missions',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    title: text('title').notNull(),
    description: text('description'),
    kind: text('kind').$type<MissionKind>().notNull().default('dynamic'),
    category: text('category').$type<MissionCategory>().notNull().default('generic'),
    issuerType: text('issuer_type').$type<MissionIssuerType>().notNull().default('system'),
    /** Opaque issuer UUID (corporation, city or player); null for the system. */
    issuerId: uuid('issuer_id'),
    status: text('status').$type<MissionStatus>().notNull().default('available'),
    /** Economic and/or item reward; null when the mission only grants reputation. */
    reward: jsonb('reward').$type<MissionReward>(),
    /** Public, or restricted to the members of `issuerId` (a corporation). */
    visibility: text('visibility').$type<MissionVisibility>().notNull().default('public'),
    /** Maximum number of simultaneous assignees (1 = single-player mission). */
    maxAssignees: integer('max_assignees').notNull().default(1),
    /** Escrow state of a player-funded economic reward (`none` for system/scenario missions). */
    escrowStatus: text('escrow_status').$type<EscrowStatus>().notNull().default('none'),
    /** Total amount debited from the creator and held until settlement/refund. */
    escrowAmount: integer('escrow_amount'),
    escrowCurrency: text('escrow_currency'),
    /** Player whose wallet funded the escrow (refunded on cancel/expire). */
    escrowPayerId: uuid('escrow_payer_id'),
    /** Idempotency key of the Economy escrow debit (`mission-escrow:<missionId>`). */
    escrowExternalId: text('escrow_external_id'),
    /** Item escrow lifecycle for a player-funded item reward. */
    escrowItemStatus: text('escrow_item_status').$type<ItemEscrowStatus>().notNull().default('none'),
    /** Inventory hold reserving the escrowed item (`refType: mission_escrow`). */
    escrowItemHoldId: uuid('escrow_item_hold_id'),
    /** Scenarized missions reference their script/chain here. */
    scriptId: text('script_id'),
    /** When set and passed, the mission is no longer acceptable nor completable. */
    expiresAt: timestamp('expires_at', { withTimezone: true }),
    /** Keycloak client id of the creating service, or player id for player-made missions. */
    createdBy: text('created_by'),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    uniqueIndex('missions_script_unique').on(t.scriptId),
    uniqueIndex('missions_escrow_external_id_unique').on(t.escrowExternalId),
    index('missions_visibility_status_idx').on(t.visibility, t.status),
  ],
);

export type Mission = typeof missions.$inferSelect;
export type NewMission = typeof missions.$inferInsert;

export const missionObjectives = pgTable(
  'mission_objectives',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    missionId: uuid('mission_id')
      .notNull()
      .references(() => missions.id, { onDelete: 'cascade' }),
    type: text('type').$type<ObjectiveType>().notNull(),
    title: text('title').notNull(),
    description: text('description'),
    /** Quantity to deliver/transport/visit before the objective is met. */
    targetQuantity: integer('target_quantity').notNull().default(1),
    /** Free-form unit label (e.g. `t`, `units`). */
    unit: text('unit'),
    /** Transport/delivery origin, opaque to this service. */
    locationFrom: jsonb('location_from').$type<Record<string, unknown>>(),
    /** Transport/delivery destination, opaque to this service. */
    locationTo: jsonb('location_to').$type<Record<string, unknown>>(),
    /** Position in a scenario chain (ascending); independent objectives may all be 0. */
    order: integer('order').notNull().default(0),
    status: text('status').$type<ObjectiveStatus>().notNull().default('pending'),
    /** Progress reported so far, compared against `targetQuantity`. */
    currentProgress: integer('current_progress').notNull().default(0),
    /** Extra parameters (material id, NPC id, script step, ...). */
    payload: jsonb('payload').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('mission_objectives_mission_order_idx').on(t.missionId, t.order)],
);

export type MissionObjective = typeof missionObjectives.$inferSelect;
export type NewMissionObjective = typeof missionObjectives.$inferInsert;
