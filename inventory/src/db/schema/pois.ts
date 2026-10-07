/**
 * Points of interest (POI): a GPS position in the game world, optionally widened into a
 * zone by a radius. A POI is owned by exactly one holder (player, NPC, corporation,
 * political entity or the reserved `system`) and can be shared read-only with other
 * players/entities, or have its ownership transferred.
 *
 * Unlike goods, POIs carry geometry: `x`/`y`/`z` (game units, assumed metres), an optional
 * `system`, an optional hierarchical `scene` path (same prefix convention as mission
 * zones: `tarsis_1` matches `tarsis_1/new-paris`, never `tarsis_10`) and an optional
 * `parentId` pointing at a persistence object (opaque UUID, no foreign key — Persistence
 * is not in this database).
 *
 * `radiusM = null` is a plain point; `radiusM >= 0` turns it into a zone. `visibility`
 * lets the game publish a POI to every player without a per-player share.
 */
import { sql } from 'drizzle-orm';
import {
  check,
  doublePrecision,
  index,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uuid,
} from 'drizzle-orm/pg-core';

/** Who can own a POI (wider than the goods holders: political entities appear here). */
export const POI_OWNER_TYPES = ['player', 'npc', 'corporation', 'political', 'system'] as const;
export type PoiOwnerType = (typeof POI_OWNER_TYPES)[number];

/** Who a POI can be shared with (ownership targets are the owner types, minus `system`). */
export const POI_GRANTEE_TYPES = ['player', 'npc', 'corporation', 'political'] as const;
export type PoiGranteeType = (typeof POI_GRANTEE_TYPES)[number];

/** `private` = only the owner and its grantees; `public` = readable by any player. */
export const POI_VISIBILITIES = ['private', 'public'] as const;
export type PoiVisibility = (typeof POI_VISIBILITIES)[number];

export const inventoryPois = pgTable(
  'inventory_pois',
  {
    id: uuid('id').primaryKey().defaultRandom(),
    ownerType: text('owner_type').$type<PoiOwnerType>().notNull(),
    ownerId: uuid('owner_id').notNull(),
    name: text('name').notNull(),
    description: text('description'),
    /** Star system the POI lives in (optional; `null` = unscoped / unknown). */
    system: text('system'),
    /** Hierarchical scene path (prefix matching, same convention as mission zones). */
    scene: text('scene'),
    /** Parent object in Persistence (opaque UUID, no FK across services). */
    parentId: uuid('parent_id'),
    x: doublePrecision('x').notNull(),
    y: doublePrecision('y').notNull(),
    z: doublePrecision('z').notNull(),
    /** `null` = point; `>= 0` = zone radius in metres. */
    radiusM: doublePrecision('radius_m'),
    visibility: text('visibility').$type<PoiVisibility>().notNull().default('private'),
    metadata: jsonb('metadata').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index('inventory_pois_owner_idx').on(t.ownerType, t.ownerId),
    index('inventory_pois_system_idx').on(t.system),
    index('inventory_pois_scene_idx').on(t.scene),
    index('inventory_pois_visibility_idx').on(t.visibility),
    check(
      'inventory_pois_owner_type_valid',
      sql`${t.ownerType} in ('player','npc','corporation','political','system')`,
    ),
    check('inventory_pois_radius_valid', sql`${t.radiusM} is null or ${t.radiusM} >= 0`),
    check(
      'inventory_pois_visibility_valid',
      sql`${t.visibility} in ('private','public')`,
    ),
    check('inventory_pois_name_not_empty', sql`length(btrim(${t.name})) > 0`),
  ],
);

export type InventoryPoi = typeof inventoryPois.$inferSelect;
export type NewInventoryPoi = typeof inventoryPois.$inferInsert;

/**
 * Read-only grants of a POI to another player/NPC/corporation/political entity.
 * The grantee (and the members of an organization grantee, through the organization
 * routes) may read the POI; only the owner may edit, re-share or transfer it.
 */
export const inventoryPoiShares = pgTable(
  'inventory_poi_shares',
  {
    poiId: uuid('poi_id')
      .notNull()
      .references(() => inventoryPois.id, { onDelete: 'cascade' }),
    granteeType: text('grantee_type').$type<PoiGranteeType>().notNull(),
    granteeId: uuid('grantee_id').notNull(),
    /** Player who created the grant (audit). */
    sharedBy: uuid('shared_by'),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.poiId, t.granteeType, t.granteeId] }),
    index('inventory_poi_shares_grantee_idx').on(t.granteeType, t.granteeId),
    check(
      'inventory_poi_shares_grantee_type_valid',
      sql`${t.granteeType} in ('player','npc','corporation','political')`,
    ),
  ],
);

export type InventoryPoiShare = typeof inventoryPoiShares.$inferSelect;
export type NewInventoryPoiShare = typeof inventoryPoiShares.$inferInsert;
