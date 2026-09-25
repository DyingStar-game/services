/**
 * Sanctions: issue (manual or automatic), revoke, and query active ones.
 */
import { and, desc, eq, gt, isNull, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { sanctions, type Sanction, type SanctionType } from '../db/schema/index.js';
import { notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { logModeration } from './moderationLog.service.js';
import { requireNotNpc, requireProfile } from './profiles.service.js';

/** Sanction types that close the social API for the player. */
const BLOCKING: SanctionType[] = ['suspension', 'ban'];

function activeCondition(playerId: string) {
  return and(
    eq(sanctions.playerId, playerId),
    isNull(sanctions.revokedAt),
    or(isNull(sanctions.expiresAt), gt(sanctions.expiresAt, new Date())),
  );
}

/**
 * Active (not revoked, not expired) sanctions of a player.
 * @param playerId - Player id.
 * @returns Active sanctions, newest first.
 */
export async function listActiveSanctions(playerId: string): Promise<Sanction[]> {
  return db.select().from(sanctions).where(activeCondition(playerId)).orderBy(desc(sanctions.createdAt));
}

/**
 * Full sanction history of a player.
 * @param playerId - Player id.
 * @param limit - Max rows.
 * @returns Sanctions, newest first.
 */
export async function listSanctionHistory(playerId: string, limit: number): Promise<Sanction[]> {
  return db.select().from(sanctions).where(eq(sanctions.playerId, playerId)).orderBy(desc(sanctions.createdAt)).limit(limit);
}

/**
 * Active suspension or ban, if any.
 * @param playerId - Player id.
 * @returns The blocking sanction, or null.
 */
export async function getBlockingSanction(playerId: string): Promise<Sanction | null> {
  const active = await listActiveSanctions(playerId);
  return active.find((s) => BLOCKING.includes(s.type)) ?? null;
}

/**
 * Whether the player already has an active sanction of this type.
 * @param playerId - Player id.
 * @param type - Sanction type.
 * @returns True if one is active.
 */
export async function hasActiveSanction(playerId: string, type: SanctionType): Promise<boolean> {
  const active = await listActiveSanctions(playerId);
  return active.some((s) => s.type === type);
}

/**
 * Issues a sanction.
 * @param playerId - Target player.
 * @param input - Type, reason, optional duration in hours (null/undefined = permanent).
 * @param issuedBy - Moderator id, or null for automatic sanctions.
 * @returns Created sanction.
 */
export async function issueSanction(
  playerId: string,
  input: { type: SanctionType; reason: string; durationHours?: number | null },
  issuedBy: string | null,
): Promise<Sanction> {
  await requireProfile(playerId);
  await requireNotNpc(playerId);
  const expiresAt =
    input.type === 'warning'
      ? new Date()
      : input.durationHours
        ? new Date(Date.now() + input.durationHours * 3_600_000)
        : null;
  const [sanction] = await db
    .insert(sanctions)
    .values({ playerId, type: input.type, reason: input.reason, automatic: issuedBy === null, issuedBy, expiresAt })
    .returning();
  await recordActivity(playerId, 'sanction_received', { sanctionId: sanction.id, type: sanction.type, expiresAt });
  await logModeration(issuedBy, 'sanction_issued', playerId, { sanctionId: sanction.id, type: sanction.type, reason: input.reason });
  return sanction;
}

/**
 * Revokes an active sanction.
 * @param sanctionId - Sanction id.
 * @param actorId - Moderator id.
 * @returns Updated sanction.
 */
export async function revokeSanction(sanctionId: number, actorId: string): Promise<Sanction> {
  const [existing] = await db.select().from(sanctions).where(eq(sanctions.id, sanctionId)).limit(1);
  if (!existing || existing.revokedAt) throw notFound(`Active sanction ${sanctionId} not found`);
  const [updated] = await db
    .update(sanctions)
    .set({ revokedAt: new Date(), revokedBy: actorId })
    .where(eq(sanctions.id, sanctionId))
    .returning();
  await recordActivity(existing.playerId, 'sanction_revoked', { sanctionId, type: existing.type });
  await logModeration(actorId, 'sanction_revoked', existing.playerId, { sanctionId, type: existing.type });
  return updated;
}

/**
 * Sanctions listing for the moderation dashboard.
 * @param filter - Optional player filter and active-only flag.
 * @param limit - Max rows.
 * @returns Sanctions, newest first.
 */
export async function listSanctions(filter: { playerId?: string; activeOnly: boolean }, limit: number): Promise<Sanction[]> {
  const conditions = [];
  if (filter.playerId) conditions.push(eq(sanctions.playerId, filter.playerId));
  if (filter.activeOnly) {
    conditions.push(isNull(sanctions.revokedAt), or(isNull(sanctions.expiresAt), gt(sanctions.expiresAt, new Date())));
  }
  const query = db.select().from(sanctions);
  const filtered = conditions.length ? query.where(and(...conditions)) : query;
  return filtered.orderBy(desc(sanctions.createdAt)).limit(limit);
}
