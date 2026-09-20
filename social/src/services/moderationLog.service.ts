/**
 * Moderator action audit log.
 */
import { desc } from 'drizzle-orm';

import { db } from '../db/connection.js';
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
 * @param limit - Max entries.
 * @returns Entries, newest first.
 */
export async function listModerationLog(limit: number): Promise<ModerationLogEntry[]> {
  return db.select().from(moderationLog).orderBy(desc(moderationLog.createdAt), desc(moderationLog.id)).limit(limit);
}
