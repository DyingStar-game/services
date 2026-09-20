/**
 * Per-player activity history (social events and game-server reported actions).
 */
import { index, integer, jsonb, pgTable, text, timestamp, uuid } from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

export const playerActivity = pgTable(
  'player_activity',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    type: text('type').notNull(),
    details: jsonb('details').$type<Record<string, unknown>>(),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [index('player_activity_player_created_idx').on(t.playerId, t.createdAt.desc())],
);

export type PlayerActivityEntry = typeof playerActivity.$inferSelect;
