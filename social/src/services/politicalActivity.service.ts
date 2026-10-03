/**
 * Political entity internal journal (`political_activity`).
 */
import { desc, eq } from 'drizzle-orm';

import { db, type Db } from '../db/connection.js';
import { politicalActivity, type PoliticalActivityEntry } from '../db/schema/index.js';

/** Transaction handle or the shared db. */
type Executor = Db | Parameters<Parameters<Db['transaction']>[0]>[0];

/**
 * Appends an entry to a political entity's journal.
 * @param entityId - Political entity id.
 * @param actorId - Acting profile, or null for system events.
 * @param type - Event type (e.g. `member_appointed`, `parent_set`).
 * @param details - Optional structured payload.
 * @param executor - Transaction to run in (defaults to the shared db).
 */
export async function recordPoliticalActivity(
  entityId: string,
  actorId: string | null,
  type: string,
  details?: Record<string, unknown>,
  executor: Executor = db,
): Promise<void> {
  await executor.insert(politicalActivity).values({ entityId, actorId, type, details });
}

/**
 * Most recent journal entries of a political entity.
 * @param entityId - Political entity id.
 * @param limit - Max entries.
 * @returns Entries, newest first.
 */
export async function listPoliticalActivity(entityId: string, limit: number): Promise<PoliticalActivityEntry[]> {
  return db
    .select()
    .from(politicalActivity)
    .where(eq(politicalActivity.entityId, entityId))
    .orderBy(desc(politicalActivity.createdAt), desc(politicalActivity.id))
    .limit(limit);
}
