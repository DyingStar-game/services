/**
 * Player assignments: accepting a mission, reporting objective progress, completing
 * (which settles the reward) and abandoning.
 */
import { and, asc, count, desc, eq, inArray, isNull, lte } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  missionAssignments,
  missionObjectives,
  missions,
  type AssigneeHolderType,
  type AssignmentStatus,
  type Mission,
  type MissionAssignment,
  type MissionObjective,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { getObjectiveKind } from '../kinds/index.js';
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
  settleAssignment,
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
  status: AssignmentStatus | undefined,
  limit: number,
  offset: number,
): Promise<Page<PlayerMission>> {
  const conditions = [eq(missionAssignments.playerId, playerId)];
  if (status) conditions.push(eq(missionAssignments.status, status));
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
 * Measures every `service` objective of the mission against external state (Inventory,
 * player-states, ...) and persists the result. Scenario-locked objectives are skipped.
 *
 * Snapshot kinds (`owns_items`) may regress: spending the goods lowers the stored progress,
 * which is why completion re-runs this measurement before paying. High-water kinds
 * (`visit`, `deliver_material`, `transport`, `transport_passenger`) never regress: stored
 * progress is `max(stored, measured)`.
 * @param mission - Mission being measured.
 * @param assignment - Assignee whose holder state is measured.
 * @param objectives - Current objectives of the mission.
 * @returns The objective list with measurements merged in.
 */
async function measureServiceObjectives(
  mission: Mission,
  assignment: MissionAssignment,
  objectives: MissionObjective[],
): Promise<MissionObjective[]> {
  const now = new Date();
  const result: MissionObjective[] = [];
  for (const objective of objectives) {
    const kindDef = getObjectiveKind(objective.type);
    if (!kindDef || kindDef.evaluation !== 'service' || !kindDef.measure) {
      result.push(objective);
      continue;
    }
    if (isObjectiveLocked(mission, objective, objectives)) {
      result.push(objective);
      continue;
    }

    const measured = await kindDef.measure({
      playerId: assignment.playerId,
      holderType: assignment.holderType,
      missionId: mission.id,
      objective,
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
 * incomplete, the call is rejected — unless the caller asks for a `partial` hand-in, which
 * is allowed on missions with `allowPartial` and pays a proportional credits share (see
 * {@link completeMissionPartial}).
 *
 * Multiplayer: every active assignee is completed at once and receives an equal share of
 * each reward component (frozen in `rewardShares`, so a retry settles the exact same
 * amounts). The bonus components of the completed bonus objectives are merged into the
 * shares.
 * @param missionId - Mission id.
 * @param playerId - Player triggering the completion (must have an active assignment).
 * @param opts.force - Skip the objective check (game-server override).
 * @param opts.settle - Settle the rewards now (default true).
 * @param opts.partial - Hand in proportionally to the weighted progress of the required objectives.
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
 * Hands a mission in proportionally to the weighted progress of its required objectives:
 * the caller's assignment becomes `partially_completed` (terminal), receives the
 * corresponding credits share and never receives any item. The mission is closed on the
 * spot (the payout is final) and the unpaid remainder of the escrow goes back to the
 * payer.
 *
 * Only one active assignee is allowed: their share is frozen against the whole mission
 * reward, so a second active assignee would be paid out of a pool already reduced by the
 * partial payout.
 * @param mission - The mission.
 * @param assignment - The caller's active assignment.
 * @param objectives - Objectives as measured for this completion attempt.
 * @param opts.settle - Settle the rewards now (default true).
 * @returns The result, with the partially completed assignment and its settlement.
 * @throws 409 `PARTIAL_NOT_ALLOWED` / `PARTIAL_NO_PROGRESS` / `PARTIAL_MULTIPLAYER`.
 */
async function completeMissionPartial(
  mission: Mission,
  assignment: MissionAssignment,
  objectives: MissionObjective[],
  opts: { settle?: boolean } = {},
): Promise<CompleteMissionResult> {
  if (!mission.allowPartial) {
    throw new HttpError(409, 'PARTIAL_NOT_ALLOWED', 'This mission does not accept a partial hand-in');
  }
  const ratioBp = requiredProgressBp(objectives);
  if (ratioBp <= 0) {
    throw new HttpError(409, 'PARTIAL_NO_PROGRESS', 'None of the required objectives has any progress yet');
  }
  const participants = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, mission.id), eq(missionAssignments.status, 'active')));
  if (participants.length > 1) {
    throw new HttpError(
      409,
      'PARTIAL_MULTIPLAYER',
      'A partial hand-in requires you to be the only assignee still working on this mission',
    );
  }

  const shares = freezeShares(mission, assignment, participants, { ratioBp });
  const creditsShare = shares.find((component) => component.type === 'credits');
  const now = new Date();

  const { assignment: partial, mission: updatedMission } = await db.transaction(async (tx) => {
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
      .where(and(eq(missionAssignments.id, assignment.id), eq(missionAssignments.status, 'active')))
      .returning();
    if (!row) {
      // Another completion won the race: nothing was paid.
      throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', 'Assignment is no longer active');
    }
    const [missionRow] = await tx
      .update(missions)
      .set({ status: 'completed', updatedAt: now })
      .where(eq(missions.id, mission.id))
      .returning();
    return { assignment: row, mission: missionRow };
  });

  if (opts.settle === false) {
    return { assignment: partial, assignments: [partial], mission: updatedMission };
  }
  const settlement = await settleAssignment(updatedMission, partial);
  // Nothing else can be completed on this mission: give the payer what is left.
  await refundEscrow(updatedMission);
  await notifyMissionEvent(partial.playerId, {
    type: 'mission.partially_completed',
    data: { missionId: mission.id, assignmentId: partial.id, ratioBp },
  });
  return {
    assignment: settlement.assignment,
    assignments: [partial],
    mission: updatedMission,
    settlement,
    settlements: [settlement],
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
