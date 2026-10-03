/**
 * Reward settlement: turns completed assignments into Economy credits and/or Inventory
 * goods, idempotently, and splits a mission's reward equally between its participants.
 *
 * Each assignment stores the share frozen at completion (`rewardAmount` for credits,
 * `rewardItemQuantity` for fungible items), so retries credit/transfer the exact same
 * amount. Item rewards are moved by the Inventory service: a player-funded item is escrowed
 * (held) at mission creation and consumed at settlement, otherwise the goods come from the
 * system faucet. This service never mints item ownership.
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
import {
  consumeHold,
  isInventoryConfigured,
  systemHolder,
  transferInstance,
  transferStack,
} from './inventory.client.js';

/** Outcome of a settlement attempt. */
export interface SettlementResult {
  /** True when the reward (or the absence of a payable reward) is final. */
  settled: boolean;
  /** True when there was nothing to pay (no reward) or auto-settle is off. */
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

/** Records the final state of an item settlement on the assignment. */
async function markItemSettled(
  assignmentId: string,
  details: Record<string, unknown>,
): Promise<MissionAssignment> {
  const [updated] = await db
    .update(missionAssignments)
    .set({ itemSettled: true, itemSettledAt: new Date(), itemDetails: details, updatedAt: new Date() })
    .where(eq(missionAssignments.id, assignmentId))
    .returning();
  return updated;
}

/** Records the final state of an economic settlement on the assignment. */
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
 * Grants the item reward to an assignee through the Inventory service.
 *
 * A held escrow (player-funded) is consumed to the assignee; otherwise the goods are
 * transferred from the system faucet. Instance rewards transfer the specific instance.
 * @param mission - The mission.
 * @param assignment - The assignee.
 * @returns Details recorded on the assignment.
 */
async function grantItemReward(mission: Mission, assignment: MissionAssignment): Promise<Record<string, unknown>> {
  const item = mission.reward?.item;
  if (!item) return { skipped: true, reason: 'no_item_reward' };
  if (!isInventoryConfigured()) {
    throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'Item rewards require the Inventory service to be configured');
  }

  const quantity = assignment.rewardItemQuantity ?? item.quantity;
  const assignee = { holderType: 'player' as const, holderId: assignment.playerId };

  if (mission.escrowItemStatus === 'held' && mission.escrowItemHoldId) {
    await consumeHold(mission.escrowItemHoldId, assignee);
    return {
      source: 'escrow',
      holdId: mission.escrowItemHoldId,
      itemId: item.itemId,
      instanceId: item.instanceId ?? null,
      quantity: item.instanceId ? 1 : quantity,
    };
  }

  const source = systemHolder();
  if (item.instanceId) {
    await transferInstance(source, assignee, item.instanceId);
    return { source: 'system', itemId: item.itemId, instanceId: item.instanceId, quantity: 1 };
  }
  await transferStack(source, assignee, item.itemId, quantity);
  return { source: 'system', itemId: item.itemId, quantity };
}

/**
 * Pays the economic reward of a mission to its assignee and/or grants its item reward.
 *
 * Uses the assignment's `rewardExternalId` as the Economy idempotency key, so a retry after
 * a partial failure can never double-credit the player. Safe to call repeatedly.
 * @param mission - The completed mission.
 * @param assignment - The player's assignment.
 * @param opts.force - Ignore `MISSION_REWARD_AUTO_SETTLE` (used by the internal settle route).
 * @param opts.amount - Override the amount to pay (defaults to the frozen share, then the
 *   mission reward).
 * @returns Settlement outcome.
 * @throws 502 when Economy rejects the credit (the assignment stays unsettled for retry).
 */
export async function settleAssignment(
  mission: Mission,
  assignment: MissionAssignment,
  opts: { force?: boolean; amount?: number } = {},
): Promise<SettlementResult> {
  const economic = mission.reward?.economic;
  const item = mission.reward?.item;
  if (assignment.rewardSettled && assignment.itemSettled) {
    return { settled: true, skipped: false, reason: 'already_settled', assignment };
  }

  // ── Economic part ──
  let current = assignment;
  const amount = opts.amount ?? assignment.rewardAmount ?? economic?.amount ?? 0;
  const economicPending = Boolean(economic && amount > 0) && !assignment.rewardSettled;
  if (economicPending) {
    if (!opts.force && !env.mission.rewardAutoSettle) {
      return { settled: false, skipped: true, reason: 'auto_settle_disabled', assignment: current };
    }
    try {
      const movement = await creditPlayer(current.playerId, {
        amount,
        currency: economic!.currency,
        reference: `mission:${mission.id}`,
        externalId: current.rewardExternalId,
      });
      current = await markSettled(current.id, {
        amount,
        currency: economic!.currency,
        economy: movement ?? { duplicate: true },
      });
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
        .where(eq(missionAssignments.id, current.id));
      if (err instanceof HttpError) throw err;
      throw new HttpError(502, 'ECONOMY_CREDIT_FAILED', (err as Error).message);
    }
  } else if (!assignment.rewardSettled) {
    current = await markSettled(current.id, { skipped: true, reason: 'no_economic_reward' });
  }

  // ── Item part ──
  if (item && !current.itemSettled) {
    if (!opts.force && !env.mission.rewardAutoSettle) {
      return { settled: false, skipped: true, reason: 'auto_settle_disabled', assignment: current };
    }
    try {
      const details = await grantItemReward(mission, current);
      current = await markItemSettled(current.id, details);
    } catch (err) {
      await db
        .update(missionAssignments)
        .set({
          itemDetails: {
            error: err instanceof HttpError ? err.code : 'INVENTORY_ERROR',
            message: (err as Error).message,
            at: new Date().toISOString(),
          },
          updatedAt: new Date(),
        })
        .where(eq(missionAssignments.id, current.id));
      if (err instanceof HttpError) throw err;
      throw new HttpError(502, 'INVENTORY_TRANSFER_FAILED', (err as Error).message);
    }
  } else if (!current.itemSettled) {
    current = await markItemSettled(current.id, { skipped: true, reason: 'no_item_reward' });
  }

  return { settled: true, skipped: false, reason: 'settled', assignment: current };
}

/**
 * Settles every completed-but-unsettled assignment of a mission (multiplayer aware), then
 * marks escrows as claimed once nothing remains to pay.
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
    if (assignment.rewardSettled && assignment.itemSettled) {
      settlements.push({ settled: true, skipped: false, reason: 'already_settled', assignment });
      continue;
    }
    settlements.push(await settleAssignment(mission, assignment, { force: opts.force }));
  }

  const allSettled = settlements.every((s) => s.settled);
  if (mission.escrowStatus === 'held' && allSettled) {
    await db
      .update(missions)
      .set({ escrowStatus: 'claimed', updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowStatus, 'held')));
  }
  if (mission.escrowItemStatus === 'held' && allSettled) {
    await db
      .update(missions)
      .set({ escrowItemStatus: 'claimed', updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowItemStatus, 'held')));
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
