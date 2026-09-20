/**
 * Guild internal journal (`guild_activity`).
 */
import { desc, eq } from 'drizzle-orm';

import { db, type Db } from '../db/connection.js';
import { guildActivity, type GuildActivityEntry } from '../db/schema/index.js';

/** Transaction handle or the shared db. */
type Executor = Db | Parameters<Parameters<Db['transaction']>[0]>[0];

/**
 * Appends an entry to a guild's journal.
 * @param guildId - Guild id.
 * @param actorId - Acting player, or null for system events.
 * @param type - Event type (e.g. `member_joined`, `rank_changed`).
 * @param details - Optional structured payload.
 * @param executor - Transaction to run in (defaults to the shared db).
 */
export async function recordGuildActivity(
  guildId: string,
  actorId: string | null,
  type: string,
  details?: Record<string, unknown>,
  executor: Executor = db,
): Promise<void> {
  await executor.insert(guildActivity).values({ guildId, actorId, type, details });
}

/**
 * Most recent journal entries of a guild.
 * @param guildId - Guild id.
 * @param limit - Max entries.
 * @returns Entries, newest first.
 */
export async function listGuildActivity(guildId: string, limit: number): Promise<GuildActivityEntry[]> {
  return db
    .select()
    .from(guildActivity)
    .where(eq(guildActivity.guildId, guildId))
    .orderBy(desc(guildActivity.createdAt), desc(guildActivity.id))
    .limit(limit);
}
