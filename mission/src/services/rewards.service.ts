/**
 * Reward settlement: turns completed assignments into Economy credits and/or Inventory
 * goods, idempotently, splitting each mission reward component equally between its
 * participants.
 *
 * Each component is settled at most once: the credits phase is guarded by the assignment's
 * `rewardExternalId` (Economy idempotency) and `rewardSettled`, item components by a
 * claim in `settledComponents` (`item:<itemId>`) taken *before* the transfer, so a retry
 * after a partial failure can never double-pay. Frozen per-component shares live in
 * `rewardShares`. Item rewards are moved by the Inventory service: player-funded items
 * are escrowed (held) at mission creation and consumed at settlement, otherwise the goods
 * come from the system faucet. This service never mints item ownership.
 */
import { and, asc, eq, sql } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  missionAssignments,
  missions,
  type Mission,
  type MissionAssignment,
  type RewardComponent,
  type RewardItem,
} from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { creditHolder } from './economy.client.js';
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

/** `settledComponents` key of a reward component. */
export function componentKey(component: RewardComponent): string {
  return component.type === 'credits' ? 'credits' : `item:${component.itemId}`;
}

function isItemComponent(component: RewardComponent): component is RewardItem {
  return component.type === 'item';
}

/** Item components of a mission, in reward-list order (aligned with `escrowItemHoldIds`). */
export function itemComponents(rewards: RewardComponent[] | null): RewardItem[] {
  return (rewards ?? []).filter(isItemComponent);
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

/**
 * Freezes one assignee's share of every reward component at completion time, so a
 * settlement retry credits/transfers the exact same amounts. Unique-instance components
 * are not split (single assignee by validation).
 * @param mission - The mission (its `rewards` components).
 * @param assignment - The assignee being completed.
 * @param participants - Every active assignee sharing the mission.
 * @returns The frozen shares for this assignee.
 */
export function freezeShares(
  mission: Mission,
  assignment: MissionAssignment,
  participants: MissionAssignment[],
): RewardComponent[] {
  return (mission.rewards ?? []).map((component) => {
    if (component.type === 'credits') {
      const shares = splitReward(component.amount, participants);
      return { ...component, amount: shares.get(assignment.id) ?? 0 };
    }
    if (component.instanceId) return component;
    const shares = splitReward(component.quantity, participants);
    return { ...component, quantity: shares.get(assignment.id) ?? 0 };
  });
}

/** Merges a patch into the assignment's `rewardDetails` jsonb (read-modify-write). */
async function mergeDetails(assignmentId: string, patch: Record<string, unknown>): Promise<void> {
  const [row] = await db
    .select({ details: missionAssignments.rewardDetails })
    .from(missionAssignments)
    .where(eq(missionAssignments.id, assignmentId))
    .limit(1);
  const merged = { ...(row?.details ?? {}), ...patch };
  await db
    .update(missionAssignments)
    .set({ rewardDetails: merged, updatedAt: new Date() })
    .where(eq(missionAssignments.id, assignmentId));
}

/** Records the final state of the credits phase on the assignment (merged into details). */
async function markCreditsSettled(
  assignmentId: string,
  details: Record<string, unknown>,
): Promise<MissionAssignment> {
  const [row] = await db
    .select({ details: missionAssignments.rewardDetails })
    .from(missionAssignments)
    .where(eq(missionAssignments.id, assignmentId))
    .limit(1);
  const [updated] = await db
    .update(missionAssignments)
    .set({
      rewardSettled: true,
      rewardSettledAt: new Date(),
      rewardDetails: { ...(row?.details ?? {}), ...details },
      updatedAt: new Date(),
    })
    .where(eq(missionAssignments.id, assignmentId))
    .returning();
  return updated;
}

/**
 * Claims an item component *before* granting it (atomic, only once per assignment), so
 * concurrent/retried settlements can never double-transfer the same component.
 * @returns True when the caller won the claim; false when it is already settled.
 */
async function claimItemComponent(assignmentId: string, key: string): Promise<boolean> {
  const rows = await db
    .update(missionAssignments)
    .set({ settledComponents: sql`array_append(${missionAssignments.settledComponents}, ${key})`, updatedAt: new Date() })
    .where(
      and(eq(missionAssignments.id, assignmentId), sql`not (${key} = any(${missionAssignments.settledComponents}))`),
    )
    .returning({ id: missionAssignments.id });
  return rows.length > 0;
}

/** Gives a component claim back (grant failed: the component must stay retryable). */
async function releaseItemClaim(assignmentId: string, key: string): Promise<void> {
  await db
    .update(missionAssignments)
    .set({ settledComponents: sql`array_remove(${missionAssignments.settledComponents}, ${key})`, updatedAt: new Date() })
    .where(eq(missionAssignments.id, assignmentId));
}

/**
 * Grants one item component to an assignee through the Inventory service.
 *
 * A held escrow (player-funded) is consumed to the assignee; otherwise the goods are
 * transferred from the system faucet. Instance rewards transfer the specific instance.
 * @param mission - The mission (escrow state).
 * @param assignment - The assignee.
 * @param component - The item component to grant.
 * @param itemIndex - Index of the component among the mission's item components.
 * @returns Details recorded on the assignment.
 */
async function grantItemComponent(
  mission: Mission,
  assignment: MissionAssignment,
  component: RewardItem,
  itemIndex: number,
): Promise<Record<string, unknown>> {
  if (!isInventoryConfigured()) {
    throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'Item rewards require the Inventory service to be configured');
  }

  const share = assignment.rewardShares?.find((c) => isItemComponent(c) && c.itemId === component.itemId);
  const quantity = share && share.type === 'item' ? share.quantity : component.quantity;
  const assignee = { holderType: assignment.holderType, holderId: assignment.playerId };
  const holdId = mission.escrowItemStatus === 'held' ? mission.escrowItemHoldIds?.[itemIndex] : undefined;

  if (holdId) {
    await consumeHold(holdId, assignee);
    return { source: 'escrow', holdId, itemId: component.itemId, instanceId: component.instanceId ?? null, quantity };
  }

  const source = systemHolder();
  if (component.instanceId) {
    await transferInstance(source, assignee, component.instanceId);
    return { source: 'system', itemId: component.itemId, instanceId: component.instanceId, quantity: 1 };
  }
  await transferStack(source, assignee, component.itemId, quantity);
  return { source: 'system', itemId: component.itemId, quantity };
}

/** True when every reward component of the assignment has been settled. */
export function isAssignmentFullySettled(mission: Mission, assignment: MissionAssignment): boolean {
  if (!assignment.rewardSettled) return false;
  const settled = new Set(assignment.settledComponents);
  return itemComponents(mission.rewards).every((component) => settled.has(componentKey(component)));
}

/**
 * Pays the credits component of a mission to its assignee and/or grants its item
 * components, one by one. Safe to call repeatedly: credits use the assignment's
 * `rewardExternalId` as the Economy idempotency key, items claim their component first.
 * @param mission - The completed mission.
 * @param assignment - The player's assignment.
 * @param opts.force - Ignore `MISSION_REWARD_AUTO_SETTLE` (internal settle route).
 * @param opts.amount - Override the credits amount (defaults to the frozen share).
 * @returns Settlement outcome.
 * @throws 502 when Economy/Inventory rejects a movement (assignment stays retryable).
 */
export async function settleAssignment(
  mission: Mission,
  assignment: MissionAssignment,
  opts: { force?: boolean; amount?: number } = {},
): Promise<SettlementResult> {
  const credits = (mission.rewards ?? []).find((component): component is Extract<RewardComponent, { type: 'credits' }> => component.type === 'credits');
  const items = itemComponents(mission.rewards);
  if (isAssignmentFullySettled(mission, assignment)) {
    return { settled: true, skipped: false, reason: 'already_settled', assignment };
  }

  // ── Credits component ──
  let current = assignment;
  const frozenCredits = assignment.rewardShares?.find((component) => component.type === 'credits');
  const amount = opts.amount ?? (frozenCredits?.type === 'credits' ? frozenCredits.amount : assignment.rewardAmount) ?? 0;
  const creditsPending = Boolean(credits && amount > 0) && !assignment.rewardSettled;
  if (creditsPending) {
    if (!opts.force && !env.mission.rewardAutoSettle) {
      return { settled: false, skipped: true, reason: 'auto_settle_disabled', assignment: current };
    }
    try {
      const movement = await creditHolder(current.holderType, current.playerId, {
        amount,
        currency: credits!.currency,
        reference: `mission:${mission.id}`,
        externalId: current.rewardExternalId,
      });
      current = await markCreditsSettled(current.id, {
        amount,
        currency: credits!.currency,
        economy: movement ?? { duplicate: true },
      });
    } catch (err) {
      await mergeDetails(current.id, {
        error: err instanceof HttpError ? err.code : 'ECONOMY_ERROR',
        message: (err as Error).message,
        at: new Date().toISOString(),
      });
      if (err instanceof HttpError) throw err;
      throw new HttpError(502, 'ECONOMY_CREDIT_FAILED', (err as Error).message);
    }
  } else if (!assignment.rewardSettled) {
    current = await markCreditsSettled(current.id, { skipped: true, reason: 'no_credits_reward' });
  }

  // ── Item components ──
  if (items.length > 0 && !opts.force && !env.mission.rewardAutoSettle) {
    return { settled: false, skipped: true, reason: 'auto_settle_disabled', assignment: current };
  }
  const settled = new Set(current.settledComponents);
  const itemOutcomes = { ...((current.rewardDetails?.items as Record<string, unknown> | undefined) ?? {}) };
  for (const [index, component] of items.entries()) {
    const key = componentKey(component);
    if (settled.has(key)) continue;
    if (!(await claimItemComponent(current.id, key))) continue; // another caller settled it
    try {
      const outcome = await grantItemComponent(mission, current, component, index);
      itemOutcomes[component.itemId] = outcome;
      await mergeDetails(current.id, { items: { ...itemOutcomes } });
      settled.add(key);
    } catch (err) {
      await releaseItemClaim(current.id, key);
      itemOutcomes[component.itemId] = {
        error: err instanceof HttpError ? err.code : 'INVENTORY_ERROR',
        message: (err as Error).message,
        at: new Date().toISOString(),
      };
      await mergeDetails(current.id, { items: { ...itemOutcomes } });
      if (err instanceof HttpError) throw err;
      throw new HttpError(502, 'INVENTORY_TRANSFER_FAILED', (err as Error).message);
    }
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
    if (isAssignmentFullySettled(mission, assignment)) {
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

/** Total amount already paid out for a mission (sum of settled assignment credits shares). */
export async function settledTotal(missionId: string): Promise<number> {
  const [row] = await db
    .select({ total: sql<string>`coalesce(sum(${missionAssignments.rewardAmount}), 0)` })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.rewardSettled, true)));
  return Number(row?.total ?? 0);
}
