/**
 * Reward settlement: turns completed assignments into Economy credits, idempotently, and
 * splits a mission's economic reward equally between its participants.
 *
 * Each assignment stores the share frozen at completion (`rewardAmount`), so retries credit
 * the exact same amount. Item rewards are only recorded here (the inventory service is a
 * future increment).
 */
import { and, asc, eq, sql } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  missionAssignments,
  missions,
  type Mission,
  type MissionAssignment,
} from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { creditPlayer } from './economy.client.js';

/** Outcome of a settlement attempt. */
export interface SettlementResult {
  /** True when the reward (or the absence of a payable reward) is final. */
  settled: boolean;
  /** True when there was nothing to pay (no economic reward) or auto-settle is off. */
  skipped: boolean;
  reason: string;
  assignment: MissionAssignment;
}

/**
 * Splits a total amount equally between participants, distributing the remainder to the
 * earliest participants (sorted by `acceptedAt` then `id`) so the shares always sum to the
 * total and the result is deterministic.
 * @param total - Total amount in minor units.
 * @param participants - Assignments sharing the reward.
 * @returns A map from assignment id to share.
 */
export function splitReward(total: number, participants: MissionAssignment[]): Map<string, number> {
  const ordered = [...participants].sort((a, b) => {
    const t = a.acceptedAt.getTime() - b.acceptedAt.getTime();
    return t !== 0 ? t : a.id.localeCompare(b.id);
  });
  const shares = new Map<string, number>();
  if (ordered.length === 0) return shares;
  const base = Math.floor(total / ordered.length);
  const remainder = total % ordered.length;
  ordered.forEach((assignment, index) => {
    shares.set(assignment.id, base + (index < remainder ? 1 : 0));
  });
  return shares;
}

/** Records the final state of a settlement on the assignment. */
async function markSettled(
  assignmentId: string,
  details: Record<string, unknown>,
): Promise<MissionAssignment> {
  const [updated] = await db
    .update(missionAssignments)
    .set({
      rewardSettled: true,
      rewardSettledAt: new Date(),
      rewardDetails: details,
      updatedAt: new Date(),
    })
    .where(eq(missionAssignments.id, assignmentId))
    .returning();
  return updated;
}

/**
 * Pays the economic reward of a mission to its assignee, if any.
 *
 * Uses the assignment's `rewardExternalId` as the Economy idempotency key, so a retry
 * after a partial failure can never double-credit the player. Safe to call repeatedly.
 * @param mission - The completed mission.
 * @param assignment - The player's assignment.
 * @param opts.force - Ignore `MISSION_REWARD_AUTO_SETTLE` (used by the internal settle route).
 * @param opts.amount - Override the amount to pay (defaults to the frozen share, then to
 *   the mission reward).
 * @returns Settlement outcome.
 * @throws 502 when Economy rejects the credit (the assignment stays unsettled for retry).
 */
export async function settleAssignment(
  mission: Mission,
  assignment: MissionAssignment,
  opts: { force?: boolean; amount?: number } = {},
): Promise<SettlementResult> {
  if (assignment.rewardSettled) {
    return { settled: true, skipped: false, reason: 'already_settled', assignment };
  }

  const economic = mission.reward?.economic;
  const amount = opts.amount ?? assignment.rewardAmount ?? economic?.amount ?? 0;
  if (!economic || amount <= 0) {
    return {
      settled: true,
      skipped: true,
      reason: 'no_economic_reward',
      assignment: await markSettled(assignment.id, { skipped: true, reason: 'no_economic_reward' }),
    };
  }

  if (!opts.force && !env.mission.rewardAutoSettle) {
    return { settled: false, skipped: true, reason: 'auto_settle_disabled', assignment };
  }

  const currency = economic.currency;
  try {
    const movement = await creditPlayer(assignment.playerId, {
      amount,
      currency,
      reference: `mission:${mission.id}`,
      externalId: assignment.rewardExternalId,
    });
    const updated = await markSettled(assignment.id, {
      amount,
      currency,
      // `null` means Economy reported DUPLICATE_EXTERNAL_ID: already paid.
      economy: movement ?? { duplicate: true },
    });
    return { settled: true, skipped: false, reason: 'credited', assignment: updated };
  } catch (err) {
    await db
      .update(missionAssignments)
      .set({
        rewardDetails: {
          error: err instanceof HttpError ? err.code : 'ECONOMY_ERROR',
          message: (err as Error).message,
          at: new Date().toISOString(),
        },
        updatedAt: new Date(),
      })
      .where(eq(missionAssignments.id, assignment.id));
    if (err instanceof HttpError) throw err;
    throw new HttpError(502, 'ECONOMY_CREDIT_FAILED', (err as Error).message);
  }
}

/**
 * Settles every completed-but-unsettled assignment of a mission (multiplayer aware), then
 * marks a `held` escrow as `claimed` once nothing remains to pay.
 * @param mission - The completed mission.
 * @param opts.force - Ignore `MISSION_REWARD_AUTO_SETTLE`.
 * @returns One settlement outcome per completed assignment.
 */
export async function settleMissionRewards(
  mission: Mission,
  opts: { force?: boolean } = {},
): Promise<SettlementResult[]> {
  const completed = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, mission.id), eq(missionAssignments.status, 'completed')))
    .orderBy(asc(missionAssignments.acceptedAt), asc(missionAssignments.id));

  const settlements: SettlementResult[] = [];
  for (const assignment of completed) {
    if (assignment.rewardSettled) {
      settlements.push({ settled: true, skipped: false, reason: 'already_settled', assignment });
      continue;
    }
    settlements.push(await settleAssignment(mission, assignment, { force: opts.force }));
  }

  if (mission.escrowStatus === 'held' && settlements.every((s) => s.settled)) {
    await db
      .update(missions)
      .set({ escrowStatus: 'claimed', updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowStatus, 'held')));
  }
  return settlements;
}

/** Total amount already paid out for a mission (sum of settled assignment shares). */
export async function settledTotal(missionId: string): Promise<number> {
  const [row] = await db
    .select({ total: sql<string>`coalesce(sum(${missionAssignments.rewardAmount}), 0)` })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.rewardSettled, true)));
  return Number(row?.total ?? 0);
}
