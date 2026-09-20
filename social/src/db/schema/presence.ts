/**
 * Live presence of a player (status + location), updated by the game server.
 * Kept apart from the profile so frequent updates do not churn profile rows.
 */
import { jsonb, pgTable, text, timestamp, uuid } from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

export const PRESENCE_STATUSES = ['online', 'mission', 'offline'] as const;
export type PresenceStatus = (typeof PRESENCE_STATUSES)[number];

/** Where a player currently is in the persistent universe. */
export interface PlayerLocation {
  system?: string;
  scene?: string;
  position?: { x: number; y: number; z: number };
}

export const playerPresence = pgTable('player_presence', {
  playerId: uuid('player_id')
    .primaryKey()
    .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
  status: text('status').$type<PresenceStatus>().notNull().default('offline'),
  location: jsonb('location').$type<PlayerLocation>(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

export type PlayerPresence = typeof playerPresence.$inferSelect;
