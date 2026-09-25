/**
 * Player reputation: every change is an event, and negative thresholds trigger automatic sanctions
 * and escalation. Idle players slowly regain reputation (rehabilitation).
 */
import { and, desc, eq, lt, notExists, sql } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  playerProfiles,
  reports,
  reputationEvents,
  type ReputationEvent,
  type ReputationSource,
  type SanctionType,
} from '../db/schema/index.js';
import { recordActivity } from './activity.service.js';
import { logModeration } from './moderationLog.service.js';
import { requireNotNpc, requireProfile } from './profiles.service.js';
import { hasActiveSanction, issueSanction } from './sanctions.service.js';

/** Threshold rules, evaluated from the most severe. */
interface ThresholdRule {
  type: SanctionType;
  at: number;
  durationHours: number | null;
}

function rules(): ThresholdRule[] {
  const r = env.reputation;
  return [
    { type: 'suspension', at: r.suspendAt, durationHours: r.suspendHours },
    { type: 'mute', at: r.muteAt, durationHours: r.muteHours },
    { type: 'warning', at: r.warnAt, durationHours: null },
  ];
}

/**
 * Applies a reputation delta, records the event and evaluates automatic sanctions.
 * @param playerId - Player id.
 * @param delta - Signed change (0 is ignored).
 * @param source - Origin of the change.
 * @param reason - Short reason.
 * @param options - Actor and details.
 * @returns New balance.
 */
export async function adjustReputation(
  playerId: string,
  delta: number,
  source: ReputationSource,
  reason: string,
  options: { actorId?: string | null; details?: Record<string, unknown> } = {},
): Promise<number> {
  const profile = await requireProfile(playerId);
  if (delta === 0) return profile.reputation;
  await requireNotNpc(playerId);

  const balance = await db.transaction(async (tx) => {
    const [updated] = await tx
      .update(playerProfiles)
      .set({ reputation: sql`${playerProfiles.reputation} + ${delta}`, updatedAt: new Date() })
      .where(eq(playerProfiles.playerId, playerId))
      .returning({ reputation: playerProfiles.reputation });
    await tx.insert(reputationEvents).values({
      playerId,
      delta,
      balance: updated.reputation,
      source,
      reason,
      actorId: options.actorId ?? null,
      details: options.details,
    });
    return updated.reputation;
  });

  if (delta < 0) await evaluateThresholds(playerId, balance);
  return balance;
}

/**
 * Issues the automatic sanction matching the balance (if not already active) and escalates
 * to the admin level when the escalation threshold is crossed.
 * @param playerId - Player id.
 * @param balance - Current reputation.
 */
async function evaluateThresholds(playerId: string, balance: number): Promise<void> {
  const rule = rules().find((r) => balance <= r.at);
  if (rule && !(await hasActiveSanction(playerId, rule.type))) {
    await issueSanction(
      playerId,
      { type: rule.type, reason: `Reputation reached ${balance} (threshold ${rule.at})`, durationHours: rule.durationHours },
      null,
    );
    await adjustNoEval(playerId, 'sanction', `automatic ${rule.type}`);
  }

  if (balance <= env.reputation.escalateAt) {
    const open = await db
      .select({ id: reports.id })
      .from(reports)
      .where(and(eq(reports.targetPlayerId, playerId), eq(reports.reason, 'reputation_threshold'), eq(reports.status, 'open')))
      .limit(1);
    if (open.length === 0) {
      await db.insert(reports).values({
        reporterId: null,
        targetType: 'player',
        targetPlayerId: playerId,
        reason: 'reputation_threshold',
        message: `Reputation reached ${balance}; automatic escalation`,
        escalation: 'admin',
      });
      await logModeration(null, 'auto_escalated', playerId, { balance });
    }
  }
}

/** Records a zero-delta marker event so history shows when a sanction fired. */
async function adjustNoEval(playerId: string, source: ReputationSource, reason: string): Promise<void> {
  const profile = await requireProfile(playerId);
  await db.insert(reputationEvents).values({ playerId, delta: 0, balance: profile.reputation, source, reason });
}

/**
 * Reputation history of a player.
 * @param playerId - Player id.
 * @param limit - Max events.
 * @returns Events, newest first.
 */
export async function listReputationEvents(playerId: string, limit: number): Promise<ReputationEvent[]> {
  return db
    .select()
    .from(reputationEvents)
    .where(eq(reputationEvents.playerId, playerId))
    .orderBy(desc(reputationEvents.createdAt), desc(reputationEvents.id))
    .limit(limit);
}

/**
 * Rehabilitation pass: players below zero with no reputation event in the last
 * `rehabAfterDays` days gain `rehabStep` points (capped at zero).
 * @returns Number of players rehabilitated.
 */
export async function rehabilitate(): Promise<number> {
  const { rehabAfterDays, rehabStep } = env.reputation;
  if (rehabStep <= 0) return 0;
  const since = new Date(Date.now() - rehabAfterDays * 86_400_000);
  const idle = await db
    .select({ playerId: playerProfiles.playerId, reputation: playerProfiles.reputation })
    .from(playerProfiles)
    .where(
      and(
        lt(playerProfiles.reputation, 0),
        notExists(
          db
            .select({ id: reputationEvents.id })
            .from(reputationEvents)
            .where(and(eq(reputationEvents.playerId, playerProfiles.playerId), sql`${reputationEvents.createdAt} > ${since}`)),
        ),
      ),
    );
  for (const p of idle) {
    const delta = Math.min(rehabStep, -p.reputation);
    await adjustReputation(p.playerId, delta, 'rehabilitation', `No incident for ${rehabAfterDays} days`);
    await recordActivity(p.playerId, 'reputation_rehabilitated', { delta });
  }
  return idle.length;
}

/**
 * Starts the periodic rehabilitation pass when `rehabIntervalMinutes` > 0.
 * @returns Timer handle or null when disabled.
 */
export function startRehabilitationScheduler(): NodeJS.Timeout | null {
  const minutes = env.reputation.rehabIntervalMinutes;
  if (minutes <= 0) return null;
  const timer = setInterval(() => {
    rehabilitate().catch((err) => console.error('Rehabilitation pass failed:', err));
  }, minutes * 60_000);
  timer.unref();
  return timer;
}
