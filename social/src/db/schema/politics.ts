/**
 * Politics: hierarchical political entities (commune → agglomeration → department → region →
 * country → federation), their offices (mayor, president, councilor, …), members (players and
 * NPCs) and internal journal. Modelled on `corporations.ts`: a single table for every level,
 * discriminated by `type`, and a self-referencing `parentId` for the hierarchy.
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
  type AnyPgColumn,
} from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

/** Political levels, lowest to highest. `commune` covers both villages and towns. */
export const POLITICAL_ENTITY_TYPES = [
  'commune',
  'agglomeration',
  'department',
  'region',
  'country',
  'federation',
] as const;
export type PoliticalEntityType = (typeof POLITICAL_ENTITY_TYPES)[number];

/** Numeric level of each type; a parent must sit at a strictly higher level. */
export const POLITICAL_ENTITY_ORDER: Record<PoliticalEntityType, number> = {
  commune: 0,
  agglomeration: 1,
  department: 2,
  region: 3,
  country: 4,
  federation: 5,
};

/** Permissions an office can grant. The head office implicitly has all of them. */
export const POLITICAL_PERMISSIONS = [
  /** Edit name, description, banner. */
  'manage_entity',
  /** Create, edit and delete offices. */
  'manage_offices',
  /** Appoint, remove and reassign members. */
  'manage_members',
  /** Attach/detach the entity to a higher-level entity. */
  'manage_hierarchy',
  /** Spend from and configure the entity's treasury (taxes, transfers). */
  'manage_treasury',
  /** Create currency for the entity (countries and federations only). */
  'issue_currency',
] as const;
export type PoliticalPermission = (typeof POLITICAL_PERMISSIONS)[number];

export const politicalEntities = pgTable(
  'political_entities',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    type: text('type').$type<PoliticalEntityType>().notNull(),
    name: text('name').notNull().unique(),
    description: text('description'),
    bannerUrl: text('banner_url'),
    /** Higher-level entity this one belongs to (null = independent). */
    parentId: uuid('parent_id').references((): AnyPgColumn => politicalEntities.id, {
      onDelete: 'set null',
    }),
    /** Profile (player or NPC) holding the head office. */
    headId: uuid('head_id').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index('political_entities_parent_idx').on(t.parentId),
    index('political_entities_type_idx').on(t.type),
  ],
);

export type PoliticalEntity = typeof politicalEntities.$inferSelect;

export const politicalOffices = pgTable(
  'political_offices',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    entityId: uuid('entity_id')
      .notNull()
      .references(() => politicalEntities.id, { onDelete: 'cascade' }),
    name: text('name').notNull(),
    /** Higher wins; a member may only act on offices strictly below their own. */
    priority: integer('priority').notNull().default(0),
    permissions: text('permissions')
      .array()
      .$type<PoliticalPermission[]>()
      .notNull()
      .default(sql`'{}'::text[]`),
    /** Exactly one per entity; held by the head, undeletable, all permissions. */
    isHead: boolean('is_head').notNull().default(false),
    /** Office given to newly appointed members; exactly one per entity. */
    isDefault: boolean('is_default').notNull().default(false),
  },
  (t) => [unique('political_offices_name_unique').on(t.entityId, t.name)],
);

export type PoliticalOffice = typeof politicalOffices.$inferSelect;

export const politicalMembers = pgTable(
  'political_members',
  {
    entityId: uuid('entity_id')
      .notNull()
      .references(() => politicalEntities.id, { onDelete: 'cascade' }),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    officeId: integer('office_id')
      .notNull()
      .references(() => politicalOffices.id),
    joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.entityId, t.playerId] }),
    index('political_members_player_idx').on(t.playerId),
  ],
);

export type PoliticalMember = typeof politicalMembers.$inferSelect;

export const politicalActivity = pgTable(
  'political_activity',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    entityId: uuid('entity_id')
      .notNull()
      .references(() => politicalEntities.id, { onDelete: 'cascade' }),
    actorId: uuid('actor_id').references(() => playerProfiles.playerId, { onDelete: 'set null' }),
    type: text('type').notNull(),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('political_activity_entity_created_idx').on(t.entityId, t.createdAt.desc())],
);

export type PoliticalActivityEntry = typeof politicalActivity.$inferSelect;
