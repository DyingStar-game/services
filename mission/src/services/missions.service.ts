/**
 * Mission and objective persistence: creation (with ordered objectives), lookup, listing,
 * status transitions and the scenario-ordering rule.
 */
import { randomUUID } from 'crypto';
import { and, asc, count, desc, eq, inArray, lte } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  missionAssignments,
  missionObjectives,
  missions,
  type Mission,
  type MissionCategory,
  type MissionIssuerType,
  type MissionKind,
  type MissionObjective,
  type MissionReward,
  type MissionStatus,
  type MissionVisibility,
  type ObjectiveType,
} from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { debitPlayer, creditPlayer } from './economy.client.js';
import { createHold, isInventoryConfigured, releaseHold } from './inventory.client.js';
import { settledTotal } from './rewards.service.js';
import { isCorporationMember } from './social.client.js';

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/** Input describing a single objective when creating a mission. */
export interface ObjectiveInput {
  type: ObjectiveType;
  title: string;
  description?: string;
  targetQuantity?: number;
  unit?: string;
  locationFrom?: Record<string, unknown>;
  locationTo?: Record<string, unknown>;
  order?: number;
  payload?: Record<string, unknown>;
}

/** Input describing a mission to create. */
export interface CreateMissionInput {
  title: string;
  description?: string;
  kind?: MissionKind;
  category?: MissionCategory;
  issuerType?: MissionIssuerType;
  issuerId?: string | null;
  reward?: MissionReward | null;
  maxAssignees?: number;
  scriptId?: string | null;
  expiresAt?: Date | null;
  objectives: ObjectiveInput[];
}

/** Input describing a player-created mission (economic reward escrowed from the creator). */
export interface CreatePlayerMissionInput {
  title: string;
  description?: string;
  category?: MissionCategory;
  /** Public by default; `corporation` restricts acceptance to the members of `issuerId`. */
  visibility?: MissionVisibility;
  /** Corporation id when `visibility` is `corporation`. */
  issuerId?: string | null;
  reward: MissionReward;
  maxAssignees?: number;
  expiresAt?: Date | null;
  objectives: ObjectiveInput[];
}

/** Filters accepted when listing missions. */
export interface MissionFilters {
  status?: MissionStatus;
  kind?: MissionKind;
  category?: MissionCategory;
  issuerType?: MissionIssuerType;
  issuerId?: string;
  visibility?: MissionVisibility;
}

/** A mission together with its objectives, ordered by `order` then creation. */
export interface MissionWithObjectives {
  mission: Mission;
  objectives: MissionObjective[];
}

/** True when the mission's deadline is in the past. */
export function isExpired(mission: Mission, now = new Date()): boolean {
  return mission.expiresAt !== null && mission.expiresAt.getTime() <= now.getTime();
}

/**
 * Fetches a mission by id.
 * @param missionId - Mission id.
 * @returns Mission, or null.
 */
export async function getMissionById(missionId: string): Promise<Mission | null> {
  const rows = await db.select().from(missions).where(eq(missions.id, missionId)).limit(1);
  return rows[0] ?? null;
}

/**
 * Fetches a mission or throws 404.
 * @param missionId - Mission id.
 * @returns Mission.
 */
export async function requireMission(missionId: string): Promise<Mission> {
  const mission = await getMissionById(missionId);
  if (!mission) throw notFound(`Mission ${missionId} not found`);
  return mission;
}

/** Objectives of a mission, ordered. */
export function listObjectives(missionId: string): Promise<MissionObjective[]> {
  return db
    .select()
    .from(missionObjectives)
    .where(eq(missionObjectives.missionId, missionId))
    .orderBy(asc(missionObjectives.order), asc(missionObjectives.createdAt));
}

/**
 * Fetches a mission and its objectives.
 * @param missionId - Mission id.
 * @returns Mission and objectives, or null.
 */
export async function getMissionWithObjectives(missionId: string): Promise<MissionWithObjectives | null> {
  const mission = await getMissionById(missionId);
  if (!mission) return null;
  return { mission, objectives: await listObjectives(missionId) };
}

/**
 * Fetches an objective belonging to a mission, or throws 404.
 * @param missionId - Mission id.
 * @param objectiveId - Objective id.
 * @returns Objective.
 */
export async function requireObjective(missionId: string, objectiveId: string): Promise<MissionObjective> {
  const rows = await db
    .select()
    .from(missionObjectives)
    .where(and(eq(missionObjectives.missionId, missionId), eq(missionObjectives.id, objectiveId)))
    .limit(1);
  const objective = rows[0];
  if (!objective) throw notFound(`Objective ${objectiveId} not found on mission ${missionId}`);
  return objective;
}

/**
 * Lists missions, newest first.
 * @param filters - Optional filters.
 * @param limit - Max rows.
 * @returns Missions.
 */
export async function listMissions(filters: MissionFilters, limit: number): Promise<Mission[]> {
  const conditions = [];
  if (filters.status) conditions.push(eq(missions.status, filters.status));
  if (filters.kind) conditions.push(eq(missions.kind, filters.kind));
  if (filters.category) conditions.push(eq(missions.category, filters.category));
  if (filters.issuerType) conditions.push(eq(missions.issuerType, filters.issuerType));
  if (filters.issuerId) conditions.push(eq(missions.issuerId, filters.issuerId));
  if (filters.visibility) conditions.push(eq(missions.visibility, filters.visibility));

  return db
    .select()
    .from(missions)
    .where(conditions.length > 0 ? and(...conditions) : undefined)
    .orderBy(desc(missions.createdAt))
    .limit(limit);
}

/**
 * Rejects an item reward that cannot be split across participants.
 *
 * A unique-instance reward can only go to one assignee. A player-funded (escrowed) item is
 * reserved as a whole and is therefore also restricted to a single assignee. System-funded
 * fungible rewards may be split between participants.
 * @param maxAssignees - Mission capacity.
 * @param reward - Mission reward.
 * @param opts.escrowed - True when the item reward is escrowed from the creator.
 */
function assertRewardSplitable(
  maxAssignees: number,
  reward?: MissionReward | null,
  opts: { escrowed?: boolean } = {},
): void {
  if (maxAssignees <= 1 || !reward?.item) return;
  const splittable = !reward.item.instanceId && !opts.escrowed;
  if (!splittable) {
    throw new HttpError(
      400,
      'ITEM_REWARD_NOT_SPLITABLE',
      'This item reward cannot be shared by several assignees (unique instance or escrowed item)',
    );
  }
}

/**
 * Creates a mission and its objectives atomically.
 * @param input - Mission and objective definitions.
 * @param createdBy - Keycloak client id of the creator service (or player id).
 * @returns The created mission and objectives.
 * @throws 409 when a `scriptId` is already used.
 */
export async function createMission(
  input: CreateMissionInput,
  createdBy: string,
): Promise<MissionWithObjectives> {
  if (input.objectives.length === 0) {
    throw new HttpError(400, 'MISSING_OBJECTIVES', 'A mission requires at least one objective');
  }
  assertRewardSplitable(input.maxAssignees ?? 1, input.reward);

  const expiresAt =
    input.expiresAt ??
    (env.mission.defaultTtlHours > 0
      ? new Date(Date.now() + env.mission.defaultTtlHours * 3600 * 1000)
      : null);

  try {
    return await db.transaction(async (tx) => {
      const [mission] = await tx
        .insert(missions)
        .values({
          title: input.title,
          description: input.description,
          kind: input.kind ?? 'dynamic',
          category: input.category ?? 'generic',
          issuerType: input.issuerType ?? 'system',
          issuerId: input.issuerId ?? null,
          reward: input.reward ?? null,
          maxAssignees: input.maxAssignees ?? 1,
          scriptId: input.scriptId ?? null,
          expiresAt,
          createdBy,
        })
        .returning();

      const objectives = await tx
        .insert(missionObjectives)
        .values(
          input.objectives.map((objective, index) => ({
            missionId: mission.id,
            type: objective.type,
            title: objective.title,
            description: objective.description,
            targetQuantity: objective.targetQuantity ?? 1,
            unit: objective.unit,
            locationFrom: objective.locationFrom,
            locationTo: objective.locationTo,
            order: objective.order ?? index,
            payload: objective.payload,
          })),
        )
        .returning();

      return { mission, objectives };
    });
  } catch (err) {
    if (isUniqueViolation(err)) {
      throw new HttpError(409, 'DUPLICATE_SCRIPT_ID', 'A mission with this scriptId already exists');
    }
    throw err;
  }
}

/**
 * Creates a player-sponsored mission. An economic reward is escrowed (debited) from the
 * creator's wallet through Economy, and/or an item reward is escrowed (held) from the
 * creator's inventory through Inventory, then held until settlement or refund.
 *
 * Both escrows use deterministic keys, so a retried creation never debits/holds twice. If
 * an escrow is refused (insufficient funds/goods), everything is rolled back and the error
 * is propagated.
 * @param input - Mission definition (economic and/or item reward required).
 * @param playerId - Creating player (escrow payer).
 * @returns The created mission and objectives.
 */
export async function createPlayerMission(
  input: CreatePlayerMissionInput,
  playerId: string,
): Promise<MissionWithObjectives> {
  if (input.objectives.length === 0) {
    throw new HttpError(400, 'MISSING_OBJECTIVES', 'A mission requires at least one objective');
  }
  const economic = input.reward.economic;
  const item = input.reward.item;
  if (!economic && !item) {
    throw new HttpError(400, 'REWARD_REQUIRED', 'Player-created missions require an economic and/or item reward');
  }
  assertRewardSplitable(input.maxAssignees ?? 1, input.reward, { escrowed: true });

  const visibility = input.visibility ?? 'public';
  if (visibility === 'corporation') {
    if (!input.issuerId) {
      throw new HttpError(400, 'CORPORATION_REQUIRED', 'A corporation mission requires an issuerId');
    }
    if (!(await isCorporationMember(playerId, input.issuerId))) {
      throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'You are not a member of this corporation');
    }
  }

  if (item && !isInventoryConfigured()) {
    throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'Item rewards require the Inventory service to be configured');
  }

  const missionId = randomUUID();
  const escrowExternalId = `mission-escrow:${missionId}`;

  const created = await db.transaction(async (tx) => {
    const [mission] = await tx
      .insert(missions)
      .values({
        id: missionId,
        title: input.title,
        description: input.description,
        kind: 'player',
        category: input.category ?? 'generic',
        issuerType: visibility === 'corporation' ? 'corporation' : 'player',
        issuerId: input.issuerId ?? null,
        visibility,
        reward: input.reward,
        maxAssignees: input.maxAssignees ?? 1,
        expiresAt: input.expiresAt ?? null,
        escrowStatus: economic ? 'pending' : 'none',
        escrowAmount: economic?.amount ?? null,
        escrowCurrency: economic?.currency ?? null,
        escrowPayerId: playerId,
        escrowExternalId: economic ? escrowExternalId : null,
        escrowItemStatus: item ? 'held' : 'none',
        createdBy: playerId,
      })
      .returning();

    const objectives = await tx
      .insert(missionObjectives)
      .values(
        input.objectives.map((objective, index) => ({
          missionId,
          type: objective.type,
          title: objective.title,
          description: objective.description,
          targetQuantity: objective.targetQuantity ?? 1,
          unit: objective.unit,
          locationFrom: objective.locationFrom,
          locationTo: objective.locationTo,
          order: objective.order ?? index,
          payload: objective.payload,
        })),
      )
      .returning();

    return { mission, objectives };
  });

  // Escrow the item reward (hold from the creator's inventory).
  let itemHoldId: string | null = null;
  if (item) {
    try {
      const hold = await createHold(
        { holderType: 'player', holderId: playerId },
        {
          kind: item.instanceId ? 'instance' : 'stack',
          goodType: item.itemId,
          quantity: item.instanceId ? undefined : item.quantity,
          instanceId: item.instanceId,
          refType: 'mission_escrow',
          refId: missionId,
        },
      );
      itemHoldId = hold.id;
      await db.update(missions).set({ escrowItemHoldId: itemHoldId, updatedAt: new Date() }).where(eq(missions.id, missionId));
    } catch (err) {
      await db
        .update(missions)
        .set({ status: 'cancelled', escrowItemStatus: 'none', updatedAt: new Date() })
        .where(eq(missions.id, missionId));
      throw err;
    }
  }

  // Escrow the economic reward (debit from the creator's wallet).
  if (economic) {
    try {
      await debitPlayer(playerId, {
        amount: economic.amount,
        currency: economic.currency,
        reference: 'mission_escrow',
        externalId: escrowExternalId,
      });
    } catch (err) {
      if (itemHoldId) {
        await releaseHold(itemHoldId).catch(() => undefined);
      }
      await db
        .update(missions)
        .set({ status: 'cancelled', escrowStatus: 'none', escrowItemStatus: 'none', escrowItemHoldId: null, updatedAt: new Date() })
        .where(eq(missions.id, missionId));
      throw err;
    }
  }

  const [mission] = await db
    .update(missions)
    .set({ ...(economic ? { escrowStatus: 'held' } : {}), updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();

  return { mission, objectives: created.objectives };
}

/**
 * Refunds the unspent escrow of a cancelled/expired player mission to its creator, and
 * releases any held item escrow. Idempotent through the `mission-refund:<missionId>` Economy
 * key and the hold status.
 * @param mission - Mission whose escrow may be refunded.
 */
export async function refundEscrow(mission: Mission): Promise<void> {
  if (mission.escrowStatus === 'held' && mission.escrowAmount && mission.escrowPayerId) {
    const refund = mission.escrowAmount - (await settledTotal(mission.id));
    if (refund > 0) {
      await creditPlayer(mission.escrowPayerId, {
        amount: refund,
        currency: mission.escrowCurrency ?? 'credits',
        reference: 'mission_refund',
        externalId: `mission-refund:${mission.id}`,
      });
    }
    await db
      .update(missions)
      .set({ escrowStatus: 'refunded', updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowStatus, 'held')));
  }

  if (mission.escrowItemStatus === 'held' && mission.escrowItemHoldId) {
    await releaseHold(mission.escrowItemHoldId).catch(() => undefined);
    await db
      .update(missions)
      .set({ escrowItemStatus: 'released', escrowItemHoldId: null, updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowItemStatus, 'held')));
  }
}

/**
 * Updates mutable mission fields.
 * @param missionId - Mission id.
 * @param patch - Fields to change.
 * @returns Updated mission.
 */
export async function updateMission(
  missionId: string,
  patch: {
    title?: string;
    description?: string | null;
    reward?: MissionReward | null;
    maxAssignees?: number;
    expiresAt?: Date | null;
  },
): Promise<Mission> {
  const [updated] = await db
    .update(missions)
    .set({ ...patch, updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();
  if (!updated) throw notFound(`Mission ${missionId} not found`);
  return updated;
}

/**
 * Rejects acceptance/progress when the mission is not open.
 * @param mission - Mission to check.
 */
export function requireOpenMission(mission: Mission): void {
  if (mission.status !== 'available' && mission.status !== 'active') {
    throw new HttpError(409, 'MISSION_NOT_OPEN', `Mission is ${mission.status}`);
  }
  if (isExpired(mission)) {
    throw new HttpError(409, 'MISSION_EXPIRED', 'Mission has expired');
  }
}

/** Number of assignments currently active on a mission. */
export async function countActiveAssignments(missionId: string): Promise<number> {
  const [row] = await db
    .select({ value: count() })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, 'active')));
  return Number(row?.value ?? 0);
}

/** Marks a mission `active` (idempotent) when it has at least one assignee. */
export async function markMissionActive(missionId: string): Promise<void> {
  await db
    .update(missions)
    .set({ status: 'active', updatedAt: new Date() })
    .where(and(eq(missions.id, missionId), eq(missions.status, 'available')));
}

/**
 * Cancels a mission and every still-active assignment on it, refunding any held escrow to
 * the creator.
 */
export async function cancelMission(missionId: string): Promise<Mission> {
  const mission = await requireMission(missionId);
  if (mission.status === 'completed') {
    throw new HttpError(409, 'MISSION_COMPLETED', 'A completed mission cannot be cancelled');
  }
  const updated = await db.transaction(async (tx) => {
    const [row] = await tx
      .update(missions)
      .set({ status: 'cancelled', updatedAt: new Date() })
      .where(eq(missions.id, missionId))
      .returning();
    await tx
      .update(missionAssignments)
      .set({ status: 'abandoned', updatedAt: new Date() })
      .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, 'active')));
    return row;
  });
  await refundEscrow(updated);
  return updated;
}

/** True when every objective of the mission is `completed`. */
export function allObjectivesCompleted(objectives: MissionObjective[]): boolean {
  return objectives.length > 0 && objectives.every((objective) => objective.status === 'completed');
}

/**
 * For scenarized missions, blocks an objective until every lower-`order` objective is done.
 * @param mission - Parent mission.
 * @param objective - Objective being reported.
 * @param objectives - All objectives of the mission.
 * @throws 409 when the objective is still locked.
 */
export function assertObjectiveUnlocked(
  mission: Mission,
  objective: MissionObjective,
  objectives: MissionObjective[],
): void {
  if (mission.kind !== 'scenario') return;
  const locked = objectives.some(
    (other) => other.order < objective.order && other.status !== 'completed',
  );
  if (locked) {
    throw new HttpError(409, 'OBJECTIVE_LOCKED', 'Previous objectives must be completed first');
  }
}

/**
 * Expires missions past their deadline. Intended to be called periodically by the game
 * server or a scheduled job.
 * @returns Number of missions expired.
 */
export async function expireDueMissions(now = new Date()): Promise<number> {
  const due = await db
    .select()
    .from(missions)
    .where(and(inArray(missions.status, ['available', 'active']), lte(missions.expiresAt, now)));

  for (const mission of due) {
    await db.transaction(async (tx) => {
      await tx
        .update(missions)
        .set({ status: 'expired', updatedAt: now })
        .where(eq(missions.id, mission.id));
      await tx
        .update(missionAssignments)
        .set({ status: 'expired', updatedAt: now })
        .where(and(eq(missionAssignments.missionId, mission.id), eq(missionAssignments.status, 'active')));
    });
    await refundEscrow(mission);
  }
  return due.length;
}
