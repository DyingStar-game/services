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
import { and, asc, eq, inArray, sql } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  missionAssignments,
  missions,
  type Mission,
  type MissionAssignment,
  type MissionObjective,
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
import { addStanding } from './social.client.js';

/** Completion ratio of a full hand-in, in basis points (10000 = every required objective). */
export const RATIO_FULL = 10000;

/**
 * Extra reward components carried by the bonus objectives of a mission that are already
 * completed. Bonus rewards are only granted on a full hand-in (never on a partial one).
 * @param objectives - All objectives of the mission.
 * @returns The bonus components, in objective order.
 */
export function bonusComponents(objectives: MissionObjective[]): RewardComponent[] {
  return objectives.flatMap((objective) =>
    objective.isBonus && objective.status === 'completed' && objective.bonusReward ? [objective.bonusReward] : [],
  );
}

/**
 * Folds bonus components into a reward list: a credits bonus tops up the single credits
 * component (settlement handles exactly one of them), an item bonus is appended.
 * @param base - The mission's own reward components.
 * @param bonuses - Components granted by completed bonus objectives.
 * @returns The merged list.
 */
export function mergeBonusComponents(
  base: RewardComponent[] | null,
  bonuses: RewardComponent[],
): RewardComponent[] {
  const merged: RewardComponent[] = [...(base ?? [])];
  for (const bonus of bonuses) {
    if (bonus.type === 'credits') {
      const existing = merged.find(
        (component): component is Extract<RewardComponent, { type: 'credits' }> => component.type === 'credits',
      );
      if (existing) {
        existing.amount += bonus.amount;
        continue;
      }
    }
    merged.push(bonus);
  }
  return merged;
}

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
  if (component.type === 'credits') return 'credits';
  if (component.type === 'standing') return `standing:${component.domain}`;
  return `item:${component.itemId}`;
}

function isItemComponent(component: RewardComponent): component is RewardItem {
  return component.type === 'item';
}

/** Item components of a mission, in reward-list order (aligned with `escrowItemHoldIds`). */
export function itemComponents(rewards: RewardComponent[] | null): RewardItem[] {
  return (rewards ?? []).filter(isItemComponent);
}

/** Standing components of a mission (domain deltas held by Social), in reward-list order. */
export function standingComponents(rewards: RewardComponent[] | null): Extract<RewardComponent, { type: 'standing' }>[] {
  return (rewards ?? []).filter(
    (component): component is Extract<RewardComponent, { type: 'standing' }> => component.type === 'standing',
  );
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
 * are not split (single assignee by validation); standing components are never split
 * either (each participant gets the full delta) and never granted on a partial hand-in.
 *
 * A partial hand-in (`ratioBp` < 10000) freezes a proportional credits share and no item
 * or standing component at all: goods are never paid against a partial completion, and
 * the bonus components of the mission are left out as well.
 * @param mission - The mission (its `rewards` components).
 * @param assignment - The assignee being completed.
 * @param participants - Every active assignee sharing the mission.
 * @param opts.ratioBp - Weighted progress actually completed, in basis points (default 10000).
 * @param opts.bonusComponents - Extra components granted by completed bonus objectives.
 * @returns The frozen shares for this assignee.
 */
export function freezeShares(
  mission: Mission,
  assignment: MissionAssignment,
  participants: MissionAssignment[],
  opts: { ratioBp?: number; bonusComponents?: RewardComponent[] } = {},
): RewardComponent[] {
  const ratioBp = opts.ratioBp ?? RATIO_FULL;
  const partial = ratioBp < RATIO_FULL;
  const components: RewardComponent[] = partial
    ? [...(mission.rewards ?? [])]
    : mergeBonusComponents(mission.rewards, opts.bonusComponents ?? []);
  return components.flatMap((component): RewardComponent[] => {
    if (component.type === 'credits') {
      const shares = splitReward(component.amount, participants);
      const share = shares.get(assignment.id) ?? 0;
      const amount = partial ? Math.floor((share * ratioBp) / RATIO_FULL) : share;
      return amount > 0 ? [{ ...component, amount }] : [];
    }
    if (partial) return [];
    // Standing is personal and never split: each participant receives the full delta.
    if (component.type === 'standing') return [component];
    if (component.instanceId) return [component];
    const shares = splitReward(component.quantity, participants);
    const share = shares.get(assignment.id) ?? 0;
    return share > 0 ? [{ ...component, quantity: share }] : [];
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
  // A partial hand-in never pays items or standing: there is nothing left beyond credits.
  if (assignment.completionRatio < RATIO_FULL) return true;
  const settled = new Set(assignment.settledComponents);
  // Standing is personal: NPC assignees never receive it, so their settlement is complete
  // as soon as credits (and items, if any) are paid.
  const goods = (mission.rewards ?? []).filter(
    (component): component is Extract<RewardComponent, { type: 'item' } | { type: 'standing' }> =>
      component.type === 'item' || (component.type === 'standing' && assignment.holderType === 'player'),
  );
  return goods.every((component) => settled.has(componentKey(component)));
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
  const standings = standingComponents(mission.rewards);
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
  // Goods are never paid against a partial hand-in: the credits share was already scaled
  // at freeze time, and there is nothing left to transfer.
  if (current.completionRatio < RATIO_FULL) {
    return { settled: true, skipped: false, reason: 'partial_settled', assignment: current };
  }
  if ((items.length > 0 || standings.length > 0) && !opts.force && !env.mission.rewardAutoSettle) {
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

  // ── Standing components ──
  // NPC assignees have no character standing: the domain delta is simply skipped.
  if (current.holderType === 'npc') {
    return { settled: true, skipped: false, reason: 'settled', assignment: current };
  }
  // Standing is read from the frozen share (never split): each participant gets the full
  // delta declared on the mission.
  const standingOutcomes = { ...((current.rewardDetails?.standing as Record<string, unknown> | undefined) ?? {}) };
  for (const component of standings) {
    const key = componentKey(component);
    if (settled.has(key)) continue;
    if (!(await claimItemComponent(current.id, key))) continue;
    const frozen = current.rewardShares?.find((c) => c.type === 'standing' && c.domain === component.domain);
    const amount = frozen?.type === 'standing' ? frozen.amount : component.amount;
    try {
      const map = await addStanding(current.playerId, component.domain, amount);
      standingOutcomes[component.domain] = { amount, standing: map };
      await mergeDetails(current.id, { standing: { ...standingOutcomes } });
      settled.add(key);
    } catch (err) {
      await releaseItemClaim(current.id, key);
      standingOutcomes[component.domain] = {
        error: err instanceof HttpError ? err.code : 'SOCIAL_STANDING_FAILED',
        message: (err as Error).message,
        at: new Date().toISOString(),
      };
      await mergeDetails(current.id, { standing: { ...standingOutcomes } });
      // Social errors (auth, unavailable, …) are wrapped so callers always see a standing
      // settlement failure; the claim was released, so the component stays retryable.
      throw new HttpError(502, 'SOCIAL_STANDING_FAILED', (err as Error).message);
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
    .where(
      and(
        eq(missionAssignments.missionId, mission.id),
        inArray(missionAssignments.status, ['completed', 'partially_completed']),
      ),
    )
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
  // A partial hand-in leaves a remainder (and ungranted item holds): the escrow is not
  // claimed, the caller refunds the remainder through `refundEscrow`.
  const hasPartial = completed.some((assignment) => assignment.status === 'partially_completed');
  if (mission.escrowStatus === 'held' && allSettled && !hasPartial) {
    await db
      .update(missions)
      .set({ escrowStatus: 'claimed', updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowStatus, 'held')));
  }
  if (mission.escrowItemStatus === 'held' && allSettled && !hasPartial) {
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
