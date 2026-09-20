/**
 * Player profile: identity, biography, personal stats and optional RP sheet.
 * `playerId` is the Keycloak subject (UUID) of the player.
 */
import { bigint, integer, jsonb, pgTable, text, timestamp, uuid } from 'drizzle-orm/pg-core';

/** Optional role-play sheet attached to a profile. */
export interface RpSheet {
  characterName?: string;
  story?: string;
  alignment?: string;
}

export const playerProfiles = pgTable('player_profiles', {
  playerId: uuid('player_id').primaryKey(),
  displayName: text('display_name').notNull().unique(),
  avatarUrl: text('avatar_url'),
  faction: text('faction'),
  biography: text('biography'),
  role: text('role'),
  level: integer('level').notNull().default(0),
  reputation: integer('reputation').notNull().default(0),
  playtimeSeconds: bigint('playtime_seconds', { mode: 'number' }).notNull().default(0),
  rpSheet: jsonb('rp_sheet').$type<RpSheet>(),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

export type PlayerProfile = typeof playerProfiles.$inferSelect;
export type NewPlayerProfile = typeof playerProfiles.$inferInsert;
