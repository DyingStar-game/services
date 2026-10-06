/**
 * Missions and their verifiable objectives.
 *
 * A mission is a **declarative spec**: category, rewards, prerequisites and objectives
 * whose shapes are validated against the kind registry (`src/kinds`). Objectives are
 * either reported by the game server (`game` kinds), measured by this service against
 * Inventory/Economy/Social state (`service` kinds), or confirmed by the mission issuer
 * (`issuer` kinds).
 */
import { boolean, index, integer, jsonb, pgTable, text, timestamp, uniqueIndex, uuid } from 'drizzle-orm/pg-core';

/** How a mission came to be. */
export const MISSION_KINDS = ['dynamic', 'scenario', 'player'] as const;
export type MissionKind = (typeof MISSION_KINDS)[number];

/**
 * Mission category. Text column validated against this list at creation; extending the
 * game's catalogue is a one-line change here (no migration).
 */
export const MISSION_CATEGORIES = [
  'delivery',
  'transport',
  'generic',
  'mining',
  'farming',
  'crafting',
  'construction',
  'trading',
  'exploration',
  'salvage',
  'combat',
  'reception',
] as const;
export type MissionCategory = (typeof MISSION_CATEGORIES)[number];

/**
 * Who issued the mission. `corporation` ids are opaque (owned by Social); `politics` is a
 * political entity (commune … federation, owned by Social); `city` kept for the game.
 */
export const MISSION_ISSUER_TYPES = ['system', 'corporation', 'city', 'politics', 'player'] as const;
export type MissionIssuerType = (typeof MISSION_ISSUER_TYPES)[number];

/** Whose account funded the escrow (drives the refund target on cancel/expire). */
export const ESCROW_PAYER_TYPES = ['player', 'corporation', 'politics'] as const;
export type EscrowPayerType = (typeof ESCROW_PAYER_TYPES)[number];

/** Lifecycle of a mission as a whole. */
export const MISSION_STATUSES = ['available', 'active', 'completed', 'cancelled', 'expired'] as const;
export type MissionStatus = (typeof MISSION_STATUSES)[number];

/** Who may accept a mission. Corporation missions are restricted to members (checked in Social). */
export const MISSION_VISIBILITIES = ['public', 'corporation'] as const;
export type MissionVisibility = (typeof MISSION_VISIBILITIES)[number];

/** Escrow lifecycle for player-funded rewards (economic only). */
export const ESCROW_STATUSES = ['none', 'pending', 'held', 'refunded', 'claimed'] as const;
export type EscrowStatus = (typeof ESCROW_STATUSES)[number];

/** Lifecycle of a single objective. */
export const OBJECTIVE_STATUSES = ['pending', 'in_progress', 'completed', 'failed'] as const;
export type ObjectiveStatus = (typeof OBJECTIVE_STATUSES)[number];

/** A credit reward component (at most one per mission). */
export interface RewardCredits {
  type: 'credits';
  currency: string;
  amount: number;
}

/** An item reward component (any number, distinct items). */
export interface RewardItem {
  type: 'item';
  itemId: string;
  quantity: number;
  /** Unique-instance reward (single assignee only). */
  instanceId?: string;
}

/** One entry of a mission's reward list (also used for frozen per-assignee shares). */
export type RewardComponent = RewardCredits | RewardItem;

/** A prerequisite gate evaluated when a player accepts a mission. */
export interface PrerequisiteSpec {
  /** Prerequisite kind name (registry-validated). */
  kind: string;
  /** Kind-specific structured params. */
  params?: Record<string, unknown>;
}

/** Item escrow lifecycle for player-funded item rewards. */
export const ITEM_ESCROW_STATUSES = ['none', 'held', 'claimed', 'released'] as const;
export type ItemEscrowStatus = (typeof ITEM_ESCROW_STATUSES)[number];

/**
 * Where a mission is available, matched against the player's presence location
 * (`{system, scene, position}` reported by the game server):
 * - `system`: same system;
 * - `poi`: inside a POI from the Inventory registry — a zone when the POI carries a
 *   `radiusM` (or the zone overrides it), otherwise the POI's scene subtree, otherwise
 *   the exact POI coordinates. POIs are resolved through Inventory's internal API; a
 *   missing POI never matches (fail-closed, like an unknown location).
 * Geometry (scenes, areas) only lives in POIs now; an empty `zones` array means the
 * mission is available everywhere (global).
 */
export type MissionZone =
  | { kind: 'system'; system: string }
  | { kind: 'poi'; poiId: string; radiusM?: number };

/** Player location subset used for zone matching (from Social's presence). */
export interface ZoneLocation {
  system?: string | null;
  scene?: string | null;
  position?: { x: number; y: number; z: number } | null;
}

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
    /**
     * Reward components; immutable after creation (the escrow is taken against them).
     * Null/empty when the mission only grants reputation or is manually rewarded.
     */
    rewards: jsonb('rewards').$type<RewardComponent[]>(),
    /**
     * Prerequisite gates checked at acceptance (kind registry). Immutable after creation.
     */
    prerequisites: jsonb('prerequisites').$type<PrerequisiteSpec[]>()
      .notNull()
      .$default(() => []),
    /** Public, or restricted to the members of `issuerId` (a corporation). */
    visibility: text('visibility').$type<MissionVisibility>().notNull().default('public'),
    /**
     * Group this mission is shared with / claimed by (opaque UUID owned by Social). When
     * set, only its members may see and accept the mission.
     */
    groupId: uuid('group_id'),
    /**
     * Limited to a single group: the first group to accept claims the mission, then only
     * its members may join (until `maxAssignees`).
     */
    groupClaimable: boolean('group_claimable').notNull().default(false),
    /** Big event mission declared by the game server or a corporation (open multijoueur). */
    isEvent: boolean('is_event').notNull().default(false),
    /**
     * Availability zones (see `MissionZone`); empty = global. Mutable through the
     * internal API only (the game re-zones live events; no escrow impact).
     */
    zones: jsonb('zones').$type<MissionZone[]>()
      .notNull()
      .$default(() => []),
    /** Maximum number of simultaneous assignees (1 = single-player mission). */
    maxAssignees: integer('max_assignees').notNull().default(1),
    /** Escrow state of a player-funded economic reward (`none` for system/scenario missions). */
    escrowStatus: text('escrow_status').$type<EscrowStatus>().notNull().default('none'),
    /** Total amount debited from the creator and held until settlement/refund. */
    escrowAmount: integer('escrow_amount'),
    escrowCurrency: text('escrow_currency'),
    /** Player whose wallet funded the escrow (refunded on cancel/expire). */
    escrowPayerId: uuid('escrow_payer_id'),
    /** Holder kind of `escrowPayerId`: who gets the refund (player wallet or org treasury). */
    escrowPayerType: text('escrow_payer_type').$type<EscrowPayerType>().notNull().default('player'),
    /** Idempotency key of the Economy escrow debit (`mission-escrow:<missionId>`). */
    escrowExternalId: text('escrow_external_id'),
    /** Item escrow lifecycle for player-funded item rewards (all item components). */
    escrowItemStatus: text('escrow_item_status').$type<ItemEscrowStatus>().notNull().default('none'),
    /**
     * Inventory hold ids of the item reward components, aligned with the `item` components
     * of `rewards` (same order). Created at mission creation, consumed at settlement.
     */
    escrowItemHoldIds: jsonb('escrow_item_hold_ids').$type<string[]>(),
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
    /** Objective kind (validated against the kind registry at creation). */
    type: text('type').notNull(),
    title: text('title').notNull(),
    description: text('description'),
    /** Quantity to reach before the objective is met (`quantity` kinds only). */
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
    /** Kind-specific structured params, validated by the kind's params schema. */
    params: jsonb('params').$type<Record<string, unknown>>(),
    /** Free-form extras owned by the game server (script step, NPC id, ...). */
    payload: jsonb('payload').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('mission_objectives_mission_order_idx').on(t.missionId, t.order)],
);

export type MissionObjective = typeof missionObjectives.$inferSelect;
export type NewMissionObjective = typeof missionObjectives.$inferInsert;
