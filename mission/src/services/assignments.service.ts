/**
 * Player assignments: accepting a mission, reporting objective progress, completing
 * (which settles the reward) and abandoning.
 */
import { and, asc, count, desc, eq, gt, inArray, isNull, lte, or } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  missionAssignments,
  missionObjectives,
  missionPlayerStates,
  missions,
  type AssigneeHolderType,
  type AssignmentStatus,
  type Mission,
  type MissionAssignment,
  type MissionObjective,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { getObjectiveKind, type PrefetchedPlayerState } from '../kinds/index.js';
import {
  allRequiredObjectivesCompleted,
  countActiveAssignments,
  isObjectiveLocked,
  listObjectives,
  markMissionActive,
  refundEscrow,
  requireMission,
  requireOpenMission,
  requiredProgressBp,
} from './missions.service.js';
import {
  RATIO_FULL,
  bonusComponents,
  freezeShares,
  payoutBp,
  settleMissionRewards,
  type SettlementResult,
} from './rewards.service.js';
import { notifyMissionEvent } from './notifications.service.js';
import { evaluatePrerequisites } from './prerequisites.service.js';
import { getPlayerGroup, isCorporationMember, isGroupMember, notifyPlayer } from './social.client.js';
import { assertMissionZone } from './zones.service.js';

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/** A mission and the player's assignment for it. */
export interface PlayerMission {
  assignment: MissionAssignment;
  mission: Mission;
}

/**
 * Fetches a player's assignment on a mission.
 * @param missionId - Mission id.
 * @param playerId - Player id.
 * @returns Assignment, or null.
 */
export async function getAssignment(
  missionId: string,
  playerId: string,
): Promise<MissionAssignment | null> {
  const rows = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.playerId, playerId)))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * Fetches a player's assignment or throws 404.
 * @param missionId - Mission id.
 * @param playerId - Player id.
 * @returns Assignment.
 */
export async function requireAssignment(
  missionId: string,
  playerId: string,
): Promise<MissionAssignment> {
  const assignment = await getAssignment(missionId, playerId);
  if (!assignment) throw notFound(t('not_found.no_assignment', { missionId }));
  return assignment;
}

/**
 * Lists the player's missions (assignments joined with their mission), newest first.
 * @param playerId - Player id.
 * @param status - Optional assignment status filter.
 * @param limit - Max rows.
 * @returns Player missions.
 */
export async function listPlayerMissions(
  playerId: string,
  status: AssignmentStatus | AssignmentStatus[] | undefined,
  limit: number,
  offset: number,
): Promise<Page<PlayerMission>> {
  const conditions = [eq(missionAssignments.playerId, playerId)];
  const statuses = status === undefined ? [] : Array.isArray(status) ? status : [status];
  if (statuses.length > 0) conditions.push(inArray(missionAssignments.status, statuses));
  const condition = and(...conditions);

  const [items, totalRows] = await Promise.all([
    db
      .select({ assignment: missionAssignments, mission: missions })
      .from(missionAssignments)
      .innerJoin(missions, eq(missions.id, missionAssignments.missionId))
      .where(condition)
      .orderBy(desc(missionAssignments.acceptedAt), desc(missionAssignments.id))
      .limit(limit)
      .offset(offset),
    db.select({ total: count() }).from(missionAssignments).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/**
 * Accepts a mission for a player.
 * @param missionId - Mission id.
 * @param playerId - Player id.
 * @returns The assignment, mission and objectives.
 * @throws 409 when the mission is closed, full, or already accepted/completed.
 */
export async function acceptMission(
  missionId: string,
  playerId: string,
  holderType: AssigneeHolderType = 'player',
): Promise<{ assignment: MissionAssignment; mission: Mission; objectives: MissionObjective[] }> {
  const mission = await requireMission(missionId);
  requireOpenMission(mission);

  if (mission.visibility === 'corporation') {
    if (!mission.issuerId || !(await isCorporationMember(playerId, mission.issuerId))) {
      throw new HttpError(403, 'NOT_CORPORATION_MEMBER', t('corp.reserved'));
    }
  }

  const existing = await getAssignment(missionId, playerId);
  if (existing && existing.status === 'active') {
    throw new HttpError(409, 'ALREADY_ASSIGNED', 'You already accepted this mission');
  }
  if (existing && existing.status === 'completed') {
    throw new HttpError(409, 'ALREADY_COMPLETED', 'You already completed this mission');
  }
  if (existing && existing.status === 'partially_completed') {
    throw new HttpError(409, 'ALREADY_HANDED_IN', 'You already handed in this mission');
  }

  // Zone gate: the player must be where the mission is available (players only; a mission
  // without zones costs nothing, an unknown/outage location fails closed).
  if (holderType === 'player') {
    await assertMissionZone(mission, playerId);
  }

  // Scenario chain: every mission listed in `requiresMissionIds` must already be completed
  // by this player. System-driven NPC assignments skip it.
  if (holderType === 'player' && mission.requiresMissionIds.length > 0) {
    await assertMissionChainCompleted(mission, playerId);
  }

  // Anti-stacking: a player may only hold so many missions at once (0 = unlimited).
  if (holderType === 'player' && env.mission.maxActiveAssignments > 0) {
    const [row] = await db
      .select({ total: count() })
      .from(missionAssignments)
      .where(and(eq(missionAssignments.playerId, playerId), eq(missionAssignments.status, 'active')));
    if ((row?.total ?? 0) >= env.mission.maxActiveAssignments) {
      throw new HttpError(
        409,
        'ASSIGNMENT_LIMIT_REACHED',
        `You already hold ${env.mission.maxActiveAssignments} active missions`,
        { limit: env.mission.maxActiveAssignments },
      );
    }
  }

  // Prerequisite gates (kind registry): evaluated before any state change, so a player
  // who cannot afford/qualify never claims the mission for their group. NPCs assigned by
  // the game server skip them.
  if (holderType === 'player' && (mission.prerequisites?.length ?? 0) > 0) {
    await evaluatePrerequisites(mission, playerId);
  }

  // Group rules: a shared mission only accepts its members; a group-claimable mission is
  // claimed by the first group to accept (then only its members may join). NPCs assigned
  // by the game server skip those checks.
  if (holderType === 'player') {
    if (mission.groupId) {
      if (!(await isGroupMember(playerId, mission.groupId))) {
        throw new HttpError(403, 'NOT_GROUP_MEMBER', 'This mission is reserved to a group you are not a member of');
      }
    } else if (mission.groupClaimable) {
      const membership = await getPlayerGroup(playerId);
      if (!membership) {
        throw new HttpError(403, 'NO_GROUP', 'This mission is reserved to a group: create or join a group first');
      }
      const claimed = await db
        .update(missions)
        .set({ groupId: membership.group.id, updatedAt: new Date() })
        .where(and(eq(missions.id, missionId), isNull(missions.groupId)))
        .returning({ id: missions.id });
      if (claimed.length === 0) {
        const current = await requireMission(missionId);
        if (current.groupId !== membership.group.id) {
          throw new HttpError(403, 'GROUP_ALREADY_CLAIMED', 'Another group already claimed this mission');
        }
      }
    }
  }

  const active = await countActiveAssignments(missionId);
  if (active >= mission.maxAssignees) {
    throw new HttpError(409, 'MISSION_FULL', 'Mission has no free slot');
  }
  return openAssignment(mission, playerId, holderType, existing);
}

/**
 * Opens (or reopens) [playerId]'s assignment of [mission] once every gate passed: the half of
 * accepting a mission that joining a shared one also needs (see invitations.service).
 * @param existing - The player's previous assignment of this mission, if any (reactivated).
 */
export async function openAssignment(
  mission: Mission,
  playerId: string,
  holderType: AssigneeHolderType,
  existing: MissionAssignment | null | undefined,
): Promise<{ assignment: MissionAssignment; mission: Mission; objectives: MissionObjective[] }> {
  const missionId = mission.id;
  const rewardExternalId =
    existing?.rewardExternalId ?? `mission:${missionId}:${playerId}`;
  // Per-assignment deadline (B7): the mission may carry a lifetime for every assignee.
  const deadlineAt = mission.assignmentDeadlineHours
    ? new Date(Date.now() + mission.assignmentDeadlineHours * 3600 * 1000)
    : null;

  let assignment: MissionAssignment;
  if (existing) {
    const [updated] = await db
      .update(missionAssignments)
      .set({
        status: 'active',
        acceptedAt: new Date(),
        completedAt: null,
        deadlineAt,
        deadlineNotifiedAt: null,
        completionRatio: 0,
        updatedAt: new Date(),
      })
      .where(eq(missionAssignments.id, existing.id))
      .returning();
    assignment = updated;
  } else {
    try {
      const [created] = await db
        .insert(missionAssignments)
        .values({ missionId, playerId, holderType, rewardExternalId, deadlineAt })
        .returning();
      assignment = created;
    } catch (err) {
      if (isUniqueViolation(err)) {
        throw new HttpError(409, 'ALREADY_ASSIGNED', 'You already accepted this mission');
      }
      throw err;
    }
  }

  await markMissionActive(missionId);
  const objectives = await db
    .select()
    .from(missionObjectives)
    .where(eq(missionObjectives.missionId, missionId));

  const refreshed = mission.status === 'available' ? { ...mission, status: 'active' as const } : mission;
  return { assignment, mission: refreshed, objectives };
}

/**
 * Whether an objective still has something to measure: it is a `service` one, and it is
 * not a completed high-water mark (whose progress is `max(stored, measured)` and
 * therefore cannot regress — measuring it again can only reproduce the stored value).
 * @param objective - Objective row.
 */
function isServiceMeasurable(objective: MissionObjective): boolean {
  const kindDef = getObjectiveKind(objective.type);
  if (!kindDef || kindDef.evaluation !== 'service' || !kindDef.measure) return false;
  if (kindDef.progress !== 'highWater') return true; // snapshots can regress, always measure
  return !(objective.status === 'completed' && objective.currentProgress >= objective.targetQuantity);
}

/** Options for one measurement pass. */
export interface MeasureOptions {
  /**
   * Player states preloaded by the caller (one SELECT for a whole batch). The state-based
   * kinds then issue no query at all, which is what keeps the auto-measure cheap.
   */
  stateCache?: ReadonlyMap<string, PrefetchedPlayerState | null>;
  /**
   * Remaining budget of remote (Inventory/Economy HTTP) measurements. Remote objectives
   * are skipped — progress untouched, picked up by the next pass — once it is exhausted.
   * Left undefined, every objective is measured (`/verify`, and the re-measure that
   * precedes a payout, must not be throttled).
   */
  remoteBudget?: { remaining: number };
}

/**
 * Measures every `service` objective of the mission against external state (Inventory,
 * player-states, ...) and persists the result. Scenario-locked objectives are skipped.
 *
 * Snapshot kinds (`owns_items`) may regress: spending the goods lowers the stored progress,
 * which is why completion re-runs this measurement before paying. High-water kinds
 * (`visit`, `deliver_material`, `transport`, `transport_passenger`) never regress: stored
 * progress is `max(stored, measured)` — so a completed one is never measured again.
 * @param mission - Mission being measured.
 * @param assignment - Assignee whose holder state is measured.
 * @param objectives - Current objectives of the mission.
 * @param opts - Optional batch context (preloaded states, remote-read budget).
 * @returns The objective list with measurements merged in.
 */
async function measureServiceObjectives(
  mission: Mission,
  assignment: MissionAssignment,
  objectives: MissionObjective[],
  opts: MeasureOptions = {},
): Promise<MissionObjective[]> {
  const now = new Date();
  const result: MissionObjective[] = [];
  for (const objective of objectives) {
    if (!isServiceMeasurable(objective)) {
      result.push(objective);
      continue;
    }
    if (isObjectiveLocked(mission, objective, objectives)) {
      result.push(objective);
      continue;
    }
    const kindDef = getObjectiveKind(objective.type);
    if (!kindDef?.measure) {
      result.push(objective);
      continue;
    }
    // Budget the remote reads: they are the only cost that scales with the player count.
    if (kindDef.remoteRead && opts.remoteBudget) {
      if (opts.remoteBudget.remaining <= 0) {
        result.push(objective);
        continue;
      }
      opts.remoteBudget.remaining -= 1;
    }

    const measured = await kindDef.measure({
      playerId: assignment.playerId,
      holderType: assignment.holderType,
      missionId: mission.id,
      objective,
      stateCache: opts.stateCache,
    });
    const raw = Math.max(0, Math.min(measured, objective.targetQuantity));
    // High-water: progress never regresses (a completed delivery stays complete).
    const currentProgress = kindDef.progress === 'highWater' ? Math.max(objective.currentProgress, raw) : raw;
    const status =
      currentProgress >= objective.targetQuantity ? 'completed' : currentProgress > 0 ? 'in_progress' : 'pending';
    if (currentProgress === objective.currentProgress && status === objective.status) {
      result.push(objective);
      continue;
    }
    const [updated] = await db
      .update(missionObjectives)
      .set({ currentProgress, status, updatedAt: now })
      .where(eq(missionObjectives.id, objective.id))
      .returning();
    result.push(updated);
  }
  return result;
}

/**
 * Re-measures the `service` objectives of the player's mission (verify endpoint: also the
 * action that performs `deliver_items` transfers).
 * @param missionId - Mission id.
 * @param playerId - Player whose assignment is verified.
 * @returns All objectives (measured) and whether they are all complete.
 * @throws 409 when the mission is not open or the assignment is not active.
 */
export async function verifyPlayerObjectives(
  missionId: string,
  playerId: string,
): Promise<{ objectives: MissionObjective[]; allComplete: boolean; completed: boolean }> {
  const mission = await requireMission(missionId);
  requireOpenMission(mission);
  const assignment = await requireAssignment(missionId, playerId);
  if (assignment.status !== 'active') {
    throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', `Assignment is ${assignment.status}`, { status: assignment.status });
  }
  await assertAssignmentDeadline(assignment);
  const objectives = await listObjectives(missionId);
  const measured = await measureServiceObjectives(mission, assignment, objectives);
  const allComplete = allRequiredObjectivesCompleted(measured);
  if (allComplete && mission.autoComplete) {
    await completeMission(missionId, playerId);
    return { objectives: measured, allComplete: true, completed: true };
  }
  return { objectives: measured, allComplete, completed: false };
}

/**
 * True while a pass is running. `completeMission` is not reentrant — it settles the
 * rewards — so two overlapping pushes must never both complete the same mission.
 *
 * A push arriving while one runs therefore does not start another: its states are already
 * stored by the upsert, and the next push measures them.
 */
let autoMeasureInFlight = false;

/** Where the next pass starts, so a large batch is served across successive pushes. */
let autoMeasureRotation = 0;

/** Mission statuses whose assignments may still make progress. */
const OPEN_MISSION_STATUSES = ['available', 'active'] as const;

/** Rotates a list so successive passes serve different tails first. */
function rotated<T>(items: T[], offset: number): T[] {
  if (items.length <= 1 || offset === 0) return items;
  const start = offset % items.length;
  return [...items.slice(start), ...items.slice(0, start)];
}

/**
 * Measures the `service` objectives of every active assignment held by the given players
 * and completes the missions that just became fully done — the automatic counterpart of
 * {@link verifyPlayerObjectives}, driven by the player-state push instead of an explicit
 * call. This is what makes a `visit` or a delivery progress by itself.
 *
 * Built to stay cheap when nothing happens, which is the common case:
 * - one indexed SELECT finds whether any of these players holds an active assignment;
 * - when none does, the pass stops there and nothing else runs;
 * - otherwise one SELECT loads the objectives and one preloads the whole batch of player
 *   states, so the state-based kinds (`visit`, `transport_passenger`) cost no query;
 * - remote reads (Inventory/Economy) are budgeted per pass and the order rotates, so a
 *   thousand players sharing the push channel cannot make one pass monopolise the budget.
 *
 * Fail-soft by design: a measurement or completion error is logged and swallowed — the
 * push that triggered the pass must never fail because of it, and the next push retries.
 * @param playerIds - Players whose state was just upserted.
 */
export async function measureActiveAssignmentsForPlayers(playerIds: string[]): Promise<void> {
  if (playerIds.length === 0 || autoMeasureInFlight) return;
  autoMeasureInFlight = true;
  try {
    await autoMeasurePass(playerIds);
  } catch (err) {
    console.error('[auto-measure] pass failed', err);
  } finally {
    autoMeasureInFlight = false;
  }
}

/** One {@link measureActiveAssignmentsForPlayers} pass. */
async function autoMeasurePass(playerIds: string[]): Promise<void> {
  const now = new Date();
  const rows = await db
    .select({ assignment: missionAssignments, mission: missions })
    .from(missionAssignments)
    .innerJoin(missions, eq(missions.id, missionAssignments.missionId))
    .where(
      and(
        inArray(missionAssignments.playerId, playerIds),
        eq(missionAssignments.status, 'active'),
        inArray(missions.status, [...OPEN_MISSION_STATUSES]),
        // An expired mission is swept by the expiry command; do not keep measuring it.
        or(isNull(missions.expiresAt), gt(missions.expiresAt, now)),
      ),
    );
  if (rows.length === 0) return;

  const missionIds = [...new Set(rows.map((row) => row.mission.id))];
  const objectives = await db
    .select()
    .from(missionObjectives)
    .where(inArray(missionObjectives.missionId, missionIds));
  // Nothing left to measure anywhere: every objective is either completed (high-water)
  // or waits for its issuer — the usual state of a mission already handed in.
  if (!objectives.some(isServiceMeasurable)) return;

  const objectivesByMission = new Map<string, MissionObjective[]>();
  for (const objective of objectives) {
    const list = objectivesByMission.get(objective.missionId);
    if (list) list.push(objective);
    else objectivesByMission.set(objective.missionId, [objective]);
  }

  // One SELECT for the whole batch: players with no state are recorded as known-absent
  // so the kinds do not re-query them one by one.
  const involvedPlayers = [...new Set(rows.map((row) => row.assignment.playerId))];
  const stateRows = await db
    .select()
    .from(missionPlayerStates)
    .where(inArray(missionPlayerStates.playerId, involvedPlayers));
  const found = new Map(stateRows.map((row) => [row.playerId, row as PrefetchedPlayerState | null]));
  const stateCache = new Map<string, PrefetchedPlayerState | null>(
    involvedPlayers.map((playerId) => [playerId, found.get(playerId) ?? null]),
  );

  const budgetSize = Math.max(0, env.mission.autoMeasureRemoteBudget);
  const start = autoMeasureRotation % rows.length;
  autoMeasureRotation = (start + budgetSize) % rows.length;
  const budget = { remaining: budgetSize };
  const completed = new Set<string>();

  for (const row of rotated(rows, start)) {
    // `completeMission` closes the whole mission, so the other assignees of the same
    // mission have nothing left to measure.
    if (completed.has(row.mission.id)) continue;
    const missionObjectivesList = objectivesByMission.get(row.mission.id) ?? [];
    if (!missionObjectivesList.some(isServiceMeasurable)) continue;
    try {
      const measured = await measureServiceObjectives(row.mission, row.assignment, missionObjectivesList, {
        stateCache,
        remoteBudget: budget,
      });
      if (!row.mission.autoComplete || !allRequiredObjectivesCompleted(measured)) continue;
      await completeMission(row.mission.id, row.assignment.playerId);
      completed.add(row.mission.id);
    } catch (err) {
      console.error(
        '[auto-measure] mission %s player %s failed: %s',
        row.mission.id,
        row.assignment.playerId,
        err,
      );
    }
  }
}

/** Result of completing a mission (single- or multi-player). */
export interface CompleteMissionResult {
  /** The caller's completed assignment (reward already credited when settled). */
  assignment: MissionAssignment;
  /** Every participant whose assignment was completed by this call. */
  assignments: MissionAssignment[];
  mission: Mission;
  /** The caller's settlement outcome (when settled). */
  settlement?: SettlementResult;
  /** One settlement outcome per participant (when settled). */
  settlements?: SettlementResult[];
}

/**
 * Verifies and completes a mission for all active participants, then splits and settles
 * every reward component.
 *
 * Objective verification is re-checked here, so a reward is only ever paid once every
 * objective reports its target quantity. When `force` is false and objectives are
 * incomplete, the call is rejected — unless the caller asks for a `partial` hand-in, open
 * to every mission, which pays the share done less the partial penalty (see
 * {@link completeMissionPartial}). A `partial` request on a mission that is in fact done
 * is a full completion, paid in full.
 *
 * Multiplayer: every active assignee is completed at once and receives an equal share of
 * each reward component (frozen in `rewardShares`, so a retry settles the exact same
 * amounts). The bonus components of the completed bonus objectives are merged into the
 * shares.
 * @param missionId - Mission id.
 * @param playerId - Player triggering the completion (must have an active assignment).
 * @param opts.force - Skip the objective check (game-server override).
 * @param opts.settle - Settle the rewards now (default true).
 * @param opts.partial - Hand in now, paid for the share of the required objectives done.
 * @returns The caller's assignment, all completed assignments, the mission and settlements.
 */
export async function completeMission(
  missionId: string,
  playerId: string,
  opts: { force?: boolean; settle?: boolean; partial?: boolean } = {},
): Promise<CompleteMissionResult> {
  const mission = await requireMission(missionId);
  const assignment = await requireAssignment(missionId, playerId);
  if (assignment.status !== 'active') {
    throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', `Assignment is ${assignment.status}`, { status: assignment.status });
  }
  await assertAssignmentDeadline(assignment);

  let objectives = await listObjectives(missionId);
  if (!opts.force) {
    // Fresh measurement of service objectives right before paying (snapshots may have
    // regressed, deliveries from other participants are picked up here).
    objectives = await measureServiceObjectives(mission, assignment, objectives);
    if (!allRequiredObjectivesCompleted(objectives)) {
      if (opts.partial) {
        return completeMissionPartial(mission, assignment, objectives, opts);
      }
      throw new HttpError(409, 'OBJECTIVES_INCOMPLETE', 'Not every objective is completed');
    }
  }

  const participants = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, 'active')));
  const bonuses = bonusComponents(objectives);
  const sharesByAssignment = new Map(
    participants.map((participant) => [
      participant.id,
      freezeShares(mission, participant, participants, { bonusComponents: bonuses }),
    ]),
  );
  const now = new Date();

  const { assignments: completedAssignments, mission: updatedMission } = await db.transaction(
    async (tx) => {
      const done: MissionAssignment[] = [];
      for (const participant of participants) {
        const shares = sharesByAssignment.get(participant.id) ?? [];
        const creditsShare = shares.find((component) => component.type === 'credits');
        const [row] = await tx
          .update(missionAssignments)
          .set({
            status: 'completed',
            completedAt: now,
            rewardShares: shares,
            rewardAmount: creditsShare?.type === 'credits' ? creditsShare.amount : null,
            completionRatio: RATIO_FULL,
            updatedAt: now,
          })
          .where(eq(missionAssignments.id, participant.id))
          .returning();
        done.push(row);
      }
      const [updated] = await tx
        .update(missions)
        .set({ status: 'completed', updatedAt: now })
        .where(eq(missions.id, missionId))
        .returning();
      return { assignments: done, mission: updated };
    },
  );

  const caller = completedAssignments.find((a) => a.id === assignment.id) ?? assignment;
  if (opts.settle === false) {
    return { assignment: caller, assignments: completedAssignments, mission: updatedMission };
  }

  const settlements = await settleMissionRewards(updatedMission, { force: opts.force });
  const settlement = settlements.find((s) => s.assignment.id === caller.id);
  await Promise.all(
    completedAssignments.map((participant) =>
      notifyMissionEvent(participant.playerId, {
        type: 'mission.completed',
        data: { missionId, assignmentId: participant.id, ratioBp: RATIO_FULL },
      }),
    ),
  );
  return {
    assignment: settlement?.assignment ?? caller,
    assignments: completedAssignments,
    mission: updatedMission,
    settlement,
    settlements,
  };
}

/**
 * Hands a mission in before it is done, for the share of its required objectives done
 * ({@link requiredProgressBp}): open to every mission. Every active assignee's assignment
 * becomes `partially_completed` (terminal) with that share frozen in `completionRatio`, and
 * receives their equal split of the credits reward × share × (1 − penalty) ({@link payoutBp});
 * no item, standing or bonus is ever paid against a partial. The mission is closed on the
 * spot for all of them, as a full completion closes it: the progress is shared, so the
 * mission cannot go on for one assignee once another has been paid for it. The unpaid
 * remainder of the escrow goes back to the payer, as a cancelled mission's does.
 * @param mission - The mission.
 * @param assignment - The caller's active assignment.
 * @param objectives - Objectives as measured for this completion attempt.
 * @param opts.settle - Settle the rewards now (default true).
 * @returns The result, with the partially completed assignments and their settlements.
 * @throws 409 `PARTIAL_NO_PROGRESS` when nothing is done yet (an all-or-nothing objective
 *   counts only once completed, so a mission of those has nothing to hand in partially).
 */
async function completeMissionPartial(
  mission: Mission,
  assignment: MissionAssignment,
  objectives: MissionObjective[],
  opts: { settle?: boolean } = {},
): Promise<CompleteMissionResult> {
  const ratioBp = requiredProgressBp(objectives);
  if (ratioBp <= 0) {
    throw new HttpError(409, 'PARTIAL_NO_PROGRESS', 'None of the required objectives has any progress yet');
  }
  const participants = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, mission.id), eq(missionAssignments.status, 'active')));
  const sharesByAssignment = new Map(
    participants.map((participant) => [participant.id, freezeShares(mission, participant, participants, { ratioBp })]),
  );
  const now = new Date();

  const { assignments: partials, mission: updatedMission } = await db.transaction(async (tx) => {
    const done: MissionAssignment[] = [];
    for (const participant of participants) {
      const shares = sharesByAssignment.get(participant.id) ?? [];
      const creditsShare = shares.find((component) => component.type === 'credits');
      const [row] = await tx
        .update(missionAssignments)
        .set({
          status: 'partially_completed',
          completedAt: now,
          completionRatio: ratioBp,
          rewardShares: shares,
          rewardAmount: creditsShare?.type === 'credits' ? creditsShare.amount : null,
          updatedAt: now,
        })
        .where(and(eq(missionAssignments.id, participant.id), eq(missionAssignments.status, 'active')))
        .returning();
      if (!row) {
        // Another completion won the race: the transaction rolls back, nothing was paid.
        throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', 'Assignment is no longer active');
      }
      done.push(row);
    }
    const [missionRow] = await tx
      .update(missions)
      .set({ status: 'completed', updatedAt: now })
      .where(eq(missions.id, mission.id))
      .returning();
    return { assignments: done, mission: missionRow };
  });

  const caller = partials.find((a) => a.id === assignment.id) ?? assignment;
  if (opts.settle === false) {
    return { assignment: caller, assignments: partials, mission: updatedMission };
  }
  const settlements = await settleMissionRewards(updatedMission);
  // Nothing else can be completed on this mission: give the payer what is left — once every
  // share is paid, or the refund would count an unpaid share as left over (the internal
  // settle route refunds it after paying the rest).
  if (settlements.every((s) => s.settled)) await refundEscrow(updatedMission);
  const settlement = settlements.find((s) => s.assignment.id === caller.id);
  const paid = payoutBp(ratioBp);
  await Promise.all(
    partials.map((participant) =>
      notifyMissionEvent(participant.playerId, {
        type: 'mission.partially_completed',
        data: {
          missionId: mission.id,
          assignmentId: participant.id,
          title: mission.title,
          ratioBp,
          payoutBp: paid,
          amount: participant.rewardAmount ?? 0,
        },
      }),
    ),
  );
  return {
    assignment: settlement?.assignment ?? caller,
    assignments: partials,
    mission: updatedMission,
    settlement,
    settlements,
  };
}

/** What turning a mission in now would pay its caller: the preview the turn-in screens show. */
export interface TurnInPreview {
  /** Share of the required objectives done, in basis points (10000 = every one). */
  shareBp: number;
  /** Every required objective is done: the turn-in is a full completion, paid in full. */
  full: boolean;
  /**
   * A partial turn-in is possible now: something is done, not everything. False on a mission
   * of all-or-nothing objectives none of which is done (nothing to hand in).
   */
  partial: boolean;
  /** What a partial turn-in takes off its pay, in basis points (2500 = a quarter). */
  penaltyBp: number;
  /** What the turn-in pays of the reward, in basis points (penalty included). */
  payoutBp: number;
  /** The credits it would pay the caller (their split of a shared mission), 0 when none. */
  amount: number;
  currency: string | null;
}

/**
 * What turning [mission] in now would pay [assignment]'s holder, from the objectives as last
 * measured — the same rules as {@link completeMission} (the split between the active
 * assignees, the bonuses of a full completion, the partial penalty), so the screen shows the
 * service's number. The turn-in measures again before paying: a delivery made meanwhile can
 * only raise it.
 * @param mission - The mission.
 * @param assignment - The caller's active assignment.
 * @param objectives - The mission's objectives.
 * @param participants - Every active assignee of the mission.
 * @returns The preview.
 */
export function turnInPreview(
  mission: Mission,
  assignment: MissionAssignment,
  objectives: MissionObjective[],
  participants: MissionAssignment[],
): TurnInPreview {
  const full = allRequiredObjectivesCompleted(objectives);
  const shareBp = full ? RATIO_FULL : requiredProgressBp(objectives);
  const sharers = participants.some((p) => p.id === assignment.id) ? participants : [...participants, assignment];
  const shares = freezeShares(mission, assignment, sharers, {
    ratioBp: shareBp,
    bonusComponents: full ? bonusComponents(objectives) : [],
  });
  const credits = shares.find((component) => component.type === 'credits');
  const reward = (mission.rewards ?? []).find((component) => component.type === 'credits');
  return {
    shareBp,
    full,
    partial: !full && shareBp > 0,
    penaltyBp: Math.round(env.mission.partialPenalty * RATIO_FULL),
    payoutBp: payoutBp(shareBp),
    amount: credits?.type === 'credits' ? credits.amount : 0,
    currency: reward?.type === 'credits' ? reward.currency : null,
  };
}

/**
 * Abandons the player's active assignment. The mission is reopened when no active
 * assignment remains.
 * @param missionId - Mission id.
 * @param playerId - Player id.
 * @returns Updated assignment.
 */
export async function abandonMission(
  missionId: string,
  playerId: string,
): Promise<MissionAssignment> {
  const assignment = await requireAssignment(missionId, playerId);
  if (assignment.status !== 'active') {
    throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', `Assignment is ${assignment.status}`, { status: assignment.status });
  }

  return db.transaction(async (tx) => {
    const [updated] = await tx
      .update(missionAssignments)
      .set({ status: 'abandoned', updatedAt: new Date() })
      .where(eq(missionAssignments.id, assignment.id))
      .returning();

    const remaining = await tx
      .select({ id: missionAssignments.id })
      .from(missionAssignments)
      .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, 'active')));
    if (remaining.length === 0) {
      const [mission] = await tx.select().from(missions).where(eq(missions.id, missionId)).limit(1);
      if (mission && mission.status === 'active') {
        await tx
          .update(missions)
          .set({ status: 'available', updatedAt: new Date() })
          .where(eq(missions.id, missionId));
      }
    }
    return updated;
  });
}

/**
 * Rejects an assignment whose per-assignment deadline has passed, expiring it on the way
 * out so it stops consuming a slot even if the built-in sweep has not run yet.
 * @param assignment - The assignment being acted upon.
 * @throws 409 `DEADLINE_PASSED`.
 */
async function assertAssignmentDeadline(assignment: MissionAssignment): Promise<void> {
  if (!assignment.deadlineAt || assignment.deadlineAt.getTime() > Date.now()) return;
  await db
    .update(missionAssignments)
    .set({ status: 'expired', updatedAt: new Date() })
    .where(and(eq(missionAssignments.id, assignment.id), eq(missionAssignments.status, 'active')));
  await notifyMissionEvent(assignment.playerId, {
    type: 'mission.deadline',
    data: { missionId: assignment.missionId, assignmentId: assignment.id, expired: true },
  });
  throw new HttpError(409, 'DEADLINE_PASSED', 'This assignment reached its deadline');
}

/**
 * Verifies the scenario chain of a mission: every mission listed in `requiresMissionIds`
 * must already have been completed by this player.
 * @param mission - The mission being accepted.
 * @param playerId - Player accepting it.
 * @throws 409 `CHAIN_INCOMPLETE`.
 */
async function assertMissionChainCompleted(mission: Mission, playerId: string): Promise<void> {
  const rows = await db
    .select({ missionId: missionAssignments.missionId })
    .from(missionAssignments)
    .where(
      and(
        eq(missionAssignments.playerId, playerId),
        eq(missionAssignments.status, 'completed'),
        inArray(missionAssignments.missionId, mission.requiresMissionIds),
      ),
    );
  const done = new Set(rows.map((row) => row.missionId));
  const missing = mission.requiresMissionIds.filter((id) => !done.has(id));
  if (missing.length > 0) {
    throw new HttpError(
      409,
      'CHAIN_INCOMPLETE',
      t('mission.chain_incomplete', { count: missing.length }),
      { missing: missing.join(', ') },
    );
  }
}

/**
 * Expires every assignment that reached its per-assignment deadline, notifying its holder
 * once. The sweep is idempotent: `deadlineNotifiedAt` is claimed in the same statement, so
 * a retried run never notifies twice.
 * @param now - Reference time.
 * @returns The number of assignments expired.
 */
export async function expireDueAssignments(now = new Date()): Promise<number> {
  const due = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.status, 'active'), lte(missionAssignments.deadlineAt, now)));

  for (const assignment of due) {
    const expired = await db
      .update(missionAssignments)
      .set({ status: 'expired', deadlineNotifiedAt: now, updatedAt: now })
      .where(
        and(
          eq(missionAssignments.id, assignment.id),
          eq(missionAssignments.status, 'active'),
          isNull(missionAssignments.deadlineNotifiedAt),
        ),
      )
      .returning({ id: missionAssignments.id });
    if (expired.length === 0) continue;
    await notifyMissionEvent(assignment.playerId, {
      type: 'mission.deadline',
      data: { missionId: assignment.missionId, assignmentId: assignment.id, expired: true },
    });
    // The mission may be left with no assignee at all: put it back on the board.
    await db.transaction(async (tx) => {
      const remaining = await tx
        .select({ id: missionAssignments.id })
        .from(missionAssignments)
        .where(and(eq(missionAssignments.missionId, assignment.missionId), eq(missionAssignments.status, 'active')));
      if (remaining.length === 0) {
        await tx
          .update(missions)
          .set({ status: 'available', updatedAt: now })
          .where(and(eq(missions.id, assignment.missionId), eq(missions.status, 'active')));
      }
    });
  }
  return due.length;
}

/**
 * Every assignment of a mission (any status), earliest first: the internal mission
 * detail's "who took it" list, including abandoned/expired participants.
 * @param missionId - Mission id.
 * @returns Assignments.
 */
export function listAssignmentsForMission(missionId: string): Promise<MissionAssignment[]> {
  return db
    .select()
    .from(missionAssignments)
    .where(eq(missionAssignments.missionId, missionId))
    .orderBy(asc(missionAssignments.acceptedAt), asc(missionAssignments.id));
}

/**
 * A page of one player's assignments (internal listing).
 * @param playerId - Player id.
 * @param limit - Max rows in the page.
 * @param offset - Rows to skip.
 * @returns Page of assignments.
 */
export async function listAssignmentsForPlayer(playerId: string, limit: number, offset: number): Promise<Page<MissionAssignment>> {
  const condition = eq(missionAssignments.playerId, playerId);
  const [items, totalRows] = await Promise.all([
    db
      .select()
      .from(missionAssignments)
      .where(condition)
      .orderBy(desc(missionAssignments.acceptedAt), desc(missionAssignments.id))
      .limit(limit)
      .offset(offset),
    db.select({ total: count() }).from(missionAssignments).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/** All missions referenced by a set of assignments. */
export function listMissionsByIds(ids: string[]): Promise<Mission[]> {
  if (ids.length === 0) return Promise.resolve([]);
  return db.select().from(missions).where(inArray(missions.id, ids));
}
