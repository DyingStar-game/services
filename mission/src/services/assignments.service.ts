/**
 * Player assignments: accepting a mission, reporting objective progress, completing
 * (which settles the reward) and abandoning.
 */
import { and, desc, eq, inArray } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  missionAssignments,
  missionObjectives,
  missions,
  type AssignmentStatus,
  type Mission,
  type MissionAssignment,
  type MissionObjective,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import {
  allObjectivesCompleted,
  assertObjectiveUnlocked,
  countActiveAssignments,
  markMissionActive,
  requireMission,
  requireObjective,
  requireOpenMission,
} from './missions.service.js';
import { settleMissionRewards, splitReward, type SettlementResult } from './rewards.service.js';
import { isCorporationMember } from './social.client.js';

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
  if (!assignment) throw notFound(`No assignment for mission ${missionId}`);
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
): Promise<PlayerMission[]> {
  const conditions = [eq(missionAssignments.playerId, playerId)];
  if (status) conditions.push(eq(missionAssignments.status, status));

  const rows = await db
    .select({ assignment: missionAssignments, mission: missions })
    .from(missionAssignments)
    .innerJoin(missions, eq(missions.id, missionAssignments.missionId))
    .where(and(...conditions))
    .orderBy(desc(missionAssignments.acceptedAt))
    .limit(limit);
  return rows;
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
): Promise<{ assignment: MissionAssignment; mission: Mission; objectives: MissionObjective[] }> {
  const mission = await requireMission(missionId);
  requireOpenMission(mission);

  if (mission.visibility === 'corporation') {
    if (!mission.issuerId || !(await isCorporationMember(playerId, mission.issuerId))) {
      throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'This mission is reserved to the corporation members');
    }
  }

  const existing = await getAssignment(missionId, playerId);
  if (existing && existing.status === 'active') {
    throw new HttpError(409, 'ALREADY_ASSIGNED', 'You already accepted this mission');
  }
  if (existing && existing.status === 'completed') {
    throw new HttpError(409, 'ALREADY_COMPLETED', 'You already completed this mission');
  }

  const active = await countActiveAssignments(missionId);
  if (active >= mission.maxAssignees) {
    throw new HttpError(409, 'MISSION_FULL', 'Mission has no free slot');
  }

  const rewardExternalId =
    existing?.rewardExternalId ?? `mission:${missionId}:${playerId}`;

  let assignment: MissionAssignment;
  if (existing) {
    const [updated] = await db
      .update(missionAssignments)
      .set({ status: 'active', acceptedAt: new Date(), completedAt: null, updatedAt: new Date() })
      .where(eq(missionAssignments.id, existing.id))
      .returning();
    assignment = updated;
  } else {
    try {
      const [created] = await db
        .insert(missionAssignments)
        .values({ missionId, playerId, rewardExternalId })
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
 * Reports progress on one objective of the player's mission and returns the updated state.
 * @param missionId - Mission id.
 * @param playerId - Player id.
 * @param objectiveId - Objective id.
 * @param quantity - Quantity to add (positive).
 * @returns Updated objective, all objectives and whether they are all complete.
 * @throws 409 when the player has no active assignment or the objective is locked.
 */
export async function reportProgress(
  missionId: string,
  playerId: string,
  objectiveId: string,
  quantity: number,
): Promise<{ objective: MissionObjective; objectives: MissionObjective[]; allComplete: boolean }> {
  const mission = await requireMission(missionId);
  const assignment = await requireAssignment(missionId, playerId);
  if (assignment.status !== 'active') {
    throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', `Assignment is ${assignment.status}`);
  }
  requireOpenMission(mission);

  const objective = await requireObjective(missionId, objectiveId);
  const objectives = await db
    .select()
    .from(missionObjectives)
    .where(eq(missionObjectives.missionId, missionId));
  assertObjectiveUnlocked(mission, objective, objectives);

  const nextProgress = Math.min(objective.currentProgress + quantity, objective.targetQuantity);
  const status = nextProgress >= objective.targetQuantity ? 'completed' : 'in_progress';
  const [updated] = await db
    .update(missionObjectives)
    .set({ currentProgress: nextProgress, status, updatedAt: new Date() })
    .where(eq(missionObjectives.id, objective.id))
    .returning();

  const merged = objectives.map((o) => (o.id === updated.id ? updated : o));
  return { objective: updated, objectives: merged, allComplete: allObjectivesCompleted(merged) };
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
 * Verifies and completes a mission for all active participants, then splits and settles the
 * economic reward.
 *
 * Objective verification is re-checked here, so a reward is only ever paid once every
 * objective reports its target quantity. When `force` is false and objectives are
 * incomplete, the call is rejected.
 *
 * Multiplayer: every active assignee is completed at once and receives an equal share
 * (frozen on the assignment, so a retry credits the same amount).
 * @param missionId - Mission id.
 * @param playerId - Player triggering the completion (must have an active assignment).
 * @param opts.force - Skip the objective check (game-server override).
 * @param opts.settle - Settle the rewards now (default true).
 * @returns The caller's assignment, all completed assignments, the mission and settlements.
 */
export async function completeMission(
  missionId: string,
  playerId: string,
  opts: { force?: boolean; settle?: boolean } = {},
): Promise<CompleteMissionResult> {
  const mission = await requireMission(missionId);
  const assignment = await requireAssignment(missionId, playerId);
  if (assignment.status !== 'active') {
    throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', `Assignment is ${assignment.status}`);
  }

  const objectives = await db
    .select()
    .from(missionObjectives)
    .where(eq(missionObjectives.missionId, missionId));
  if (!opts.force && !allObjectivesCompleted(objectives)) {
    throw new HttpError(409, 'OBJECTIVES_INCOMPLETE', 'Not every objective is completed');
  }

  const participants = await db
    .select()
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, 'active')));
  const shares = splitReward(mission.reward?.economic?.amount ?? 0, participants);
  const now = new Date();

  const { assignments: completedAssignments, mission: updatedMission } = await db.transaction(
    async (tx) => {
      const done: MissionAssignment[] = [];
      for (const participant of participants) {
        const [row] = await tx
          .update(missionAssignments)
          .set({
            status: 'completed',
            completedAt: now,
            rewardAmount: shares.get(participant.id) ?? 0,
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
  return {
    assignment: settlement?.assignment ?? caller,
    assignments: completedAssignments,
    mission: updatedMission,
    settlement,
    settlements,
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
    throw new HttpError(409, 'ASSIGNMENT_NOT_ACTIVE', `Assignment is ${assignment.status}`);
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

/** Assignments for many missions of one player, used by the internal settle endpoint. */
export function listAssignmentsForPlayer(playerId: string): Promise<MissionAssignment[]> {
  return db
    .select()
    .from(missionAssignments)
    .where(eq(missionAssignments.playerId, playerId))
    .orderBy(desc(missionAssignments.acceptedAt));
}

/** All missions referenced by a set of assignments. */
export function listMissionsByIds(ids: string[]): Promise<Mission[]> {
  if (ids.length === 0) return Promise.resolve([]);
  return db.select().from(missions).where(inArray(missions.id, ids));
}
