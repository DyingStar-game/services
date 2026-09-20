/**
 * Players ignored by another player. Blocking removes any friendship and prevents new requests.
 */
import { pgTable, primaryKey, timestamp, uuid } from 'drizzle-orm/pg-core';

import { playerProfiles } from './profiles.js';

export const playerBlocks = pgTable(
  'player_blocks',
  {
    blockerId: uuid('blocker_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    blockedId: uuid('blocked_id')
      .notNull()
      .references(() => playerProfiles.playerId, { onDelete: 'cascade' }),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.blockerId, t.blockedId] })],
);

export type PlayerBlock = typeof playerBlocks.$inferSelect;
