/**
 * Moderator action audit log.
 */
import { count, desc } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { page, type Page } from '../lib/pagination.js';
import { moderationLog, type ModerationLogEntry } from '../db/schema/index.js';

/**
 * Records a moderation action.
 * @param actorId - Moderator id (null for automatic actions).
 * @param action - Action name.
 * @param targetPlayerId - Affected player, if any.
 * @param details - Structured payload.
 */
export async function logModeration(
  actorId: string | null,
  action: string,
  targetPlayerId?: string | null,
  details?: Record<string, unknown>,
): Promise<void> {
  await db.insert(moderationLog).values({ actorId, action, targetPlayerId: targetPlayerId ?? null, details });
}

/**
 * Latest moderation log entries.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of entries, newest first.
 */
export async function listModerationLog(limit: number, offset: number): Promise<Page<ModerationLogEntry>> {
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(moderationLog),
    db
      .select()
      .from(moderationLog)
      .orderBy(desc(moderationLog.createdAt), desc(moderationLog.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(rows, totalRow[0].total, limit, offset);
}
