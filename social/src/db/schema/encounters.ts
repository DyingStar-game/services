/**
 * "Recently met players": symmetric counters fed by the game server, used for friend suggestions.
 */
import { integer, pgTable, primaryKey, timestamp, uuid } from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

export const playerEncounters = pgTable(
  'player_encounters',
  {
    playerId: uuid('player_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    otherId: uuid('other_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    count: integer('count').notNull().default(1),
    lastMetAt: timestamp('last_met_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.playerId, t.otherId] })],
);

export type PlayerEncounter = typeof playerEncounters.$inferSelect;
