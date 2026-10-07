/**
 * Corporation internal journal (`corporation_activity`).
 */
import { count, desc, eq } from 'drizzle-orm';

import { db, type Db } from '../db/connection.js';
import { corporationActivity, type CorporationActivityEntry } from '../db/schema/index.js';
import { page, type Page } from '../lib/pagination.js';

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
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of entries, newest first.
 */
export async function listCorporationActivity(
  corporationId: string,
  limit: number,
  offset: number,
): Promise<Page<CorporationActivityEntry>> {
  const condition = eq(corporationActivity.corporationId, corporationId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(corporationActivity).where(condition),
    db
      .select()
      .from(corporationActivity)
      .where(condition)
      .orderBy(desc(corporationActivity.createdAt), desc(corporationActivity.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(rows, totalRow[0].total, limit, offset);
}