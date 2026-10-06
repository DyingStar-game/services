/**
 * Political entity internal journal (`political_activity`).
 */
import { count, desc, eq } from 'drizzle-orm';

import { db, type Db } from '../db/connection.js';
import { politicalActivity, type PoliticalActivityEntry } from '../db/schema/index.js';
import { page, type Page } from '../lib/pagination.js';

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
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of entries, newest first.
 */
export async function listPoliticalActivity(entityId: string, limit: number, offset: number): Promise<Page<PoliticalActivityEntry>> {
  const condition = eq(politicalActivity.entityId, entityId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(politicalActivity).where(condition),
    db
      .select()
      .from(politicalActivity)
      .where(condition)
      .orderBy(desc(politicalActivity.createdAt), desc(politicalActivity.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(rows, totalRow[0].total, limit, offset);
}
