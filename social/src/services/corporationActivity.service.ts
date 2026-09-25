/**
 * Corporation internal journal (`corporation_activity`).
 */
import { desc, eq } from 'drizzle-orm';

import { db, type Db } from '../db/connection.js';
import { corporationActivity, type CorporationActivityEntry } from '../db/schema/index.js';

/** Transaction handle or the shared db. */
type Executor = Db | Parameters<Parameters<Db['transaction']>[0]>[0];

/**
 * Appends an entry to a corporation's journal.
 * @param corporationId - Corporation id.
 * @param actorId - Acting player, or null for system events.
 * @param type - Event type (e.g. `member_joined`, `rank_changed`).
 * @param details - Optional structured payload.
 * @param executor - Transaction to run in (defaults to the shared db).
 */
export async function recordCorporationActivity(
  corporationId: string,
  actorId: string | null,
  type: string,
  details?: Record<string, unknown>,
  executor: Executor = db,
): Promise<void> {
  await executor.insert(corporationActivity).values({ corporationId, actorId, type, details });
}

/**
 * Most recent journal entries of a corporation.
 * @param corporationId - Corporation id.
 * @param limit - Max entries.
 * @returns Entries, newest first.
 */
export async function listCorporationActivity(
  corporationId: string,
  limit: number,
): Promise<CorporationActivityEntry[]> {
  return db
    .select()
    .from(corporationActivity)
    .where(eq(corporationActivity.corporationId, corporationId))
    .orderBy(desc(corporationActivity.createdAt), desc(corporationActivity.id))
    .limit(limit);
}