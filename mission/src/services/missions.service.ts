/**
 * Mission and objective persistence: creation (with ordered objectives), lookup, listing,
 * status transitions and the scenario-ordering rule.
 */
import { randomUUID } from 'crypto';
import { t } from '../i18n/index.js';
import { and, asc, count, desc, eq, inArray, lte, sql } from 'drizzle-orm';

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
  type MissionStatus,
  type EscrowPayerType,
  type MissionVisibility,
  type MissionZone,
  type PrerequisiteSpec,
  type RewardComponent,
  type ZoneLocation,
} from '../db/schema/index.js';
import { getObjectiveKind } from '../kinds/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { creditHolder, debitHolder, type WalletHolderType } from './economy.client.js';
import { createHold, releaseHold } from './inventory.client.js';
import { settledTotal } from './rewards.service.js';
import { authorizeAction, getGroup, isCorporationMember } from './social.client.js';
import { validateMissionSpec, type SpecObjectiveInput } from './spec.service.js';
import { assertPoiZonesExist, resolveListingPoiGeometry, zoneFilterSql } from './zones.service.js';

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/** Input describing a mission to create. */
export interface CreateMissionInput {
  title: string;
  description?: string;
  kind?: MissionKind;
  category?: MissionCategory;
  issuerType?: MissionIssuerType;
  issuerId?: string | null;
  rewards?: RewardComponent[] | null;
  prerequisites?: PrerequisiteSpec[];
  /** Availability zones (empty = global). */
  zones?: MissionZone[];
  maxAssignees?: number;
  /** Limited to a single group: the first group to accept claims it. */
  groupClaimable?: boolean;
  scriptId?: string | null;
  expiresAt?: Date | null;
  objectives: SpecObjectiveInput[];
}

/** Input describing a player-created mission (reward escrowed from the creator). */
export interface CreatePlayerMissionInput {
  title: string;
  description?: string;
  category?: MissionCategory;
  /** Public by default; `corporation` restricts acceptance to the members of `issuerId`. */
  visibility?: MissionVisibility;
  /** Corporation id when `visibility` is `corporation`. */
  issuerId?: string | null;
  /** Alternative issuer: political entity (commune … federation, owned by Social). */
  politicalEntityId?: string | null;
  /**
   * Who funds the escrow: `creator` pays from their own wallet (default), `issuer` debits
   * the issuing organization's treasury (requires the org issuer + its permission).
   */
  escrowSource?: 'creator' | 'issuer';
  rewards: RewardComponent[];
  prerequisites?: PrerequisiteSpec[];
  /** Availability zones (empty = global). */
  zones?: MissionZone[];
  maxAssignees?: number;
  /** Limited to a single group: the first group to accept claims it. */
  groupClaimable?: boolean;
  expiresAt?: Date | null;
  objectives: SpecObjectiveInput[];
}

/** Filters accepted when listing missions. */
export interface MissionFilters {
  status?: MissionStatus;
  /** Several statuses at once (player browse: `available` + `active`). */
  statuses?: MissionStatus[];
  kind?: MissionKind;
  category?: MissionCategory;
  issuerType?: MissionIssuerType;
  issuerId?: string;
  visibility?: MissionVisibility;
  groupId?: string;
  groupClaimable?: boolean;
  isEvent?: boolean;
  /** Only missions with at least one free assignee slot (full missions disappear). */
  hasFreeSlots?: boolean;
  /**
   * Only missions created by this id (player id for player-made missions, Keycloak client
   * id for service-made ones). Powers the caller's "missions I created" listing.
   */
  createdBy?: string;
  /**
   * The viewer's group (Social). Missions shared with a group are only visible to its
   * members; pass `undefined` to disable the group filter, e.g. on the internal API.
   */
  viewerGroupId?: string | null;
  /**
   * The viewer's location (Social presence). Applies the zone filter: `undefined` disables
   * it (internal API), `null` (unknown location) hides every zoned mission.
   */
  viewerLocation?: ZoneLocation | null;
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
  if (!mission) throw notFound(t('not_found.mission', { id: missionId }));
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
  if (!objective) throw notFound(t('not_found.objective', { id: objectiveId, missionId }));
  return objective;
}

/**
 * Lists missions, newest first.
 *
 * `hasFreeSlots` hides missions whose active assignees already reach `maxAssignees`, and
 * `viewerGroupId` hides missions shared with a group from non-members (pass `undefined`
 * to disable the group filter, e.g. on the internal API).
 * @param filters - Optional filters.
 * @param limit - Max rows.
 * @returns Missions.
 */
export async function listMissions(filters: MissionFilters, limit: number, offset: number): Promise<Page<Mission>> {
  const conditions = [];
  if (filters.status) conditions.push(eq(missions.status, filters.status));
  if (filters.statuses?.length) conditions.push(inArray(missions.status, filters.statuses));
  if (filters.kind) conditions.push(eq(missions.kind, filters.kind));
  if (filters.category) conditions.push(eq(missions.category, filters.category));
  if (filters.issuerType) conditions.push(eq(missions.issuerType, filters.issuerType));
  if (filters.issuerId) conditions.push(eq(missions.issuerId, filters.issuerId));
  if (filters.visibility) conditions.push(eq(missions.visibility, filters.visibility));
  if (filters.groupId) conditions.push(eq(missions.groupId, filters.groupId));
  if (filters.groupClaimable !== undefined) conditions.push(eq(missions.groupClaimable, filters.groupClaimable));
  if (filters.isEvent !== undefined) conditions.push(eq(missions.isEvent, filters.isEvent));
  if (filters.createdBy) conditions.push(eq(missions.createdBy, filters.createdBy));
  if (filters.hasFreeSlots) {
    conditions.push(
      sql`(select count(*) from ${missionAssignments} where ${missionAssignments.missionId} = ${missions.id} and ${missionAssignments.status} = 'active') < ${missions.maxAssignees}`,
    );
  }
  if (filters.viewerGroupId !== undefined) {
    conditions.push(sql`(${missions.groupId} is null or ${missions.groupId} = ${filters.viewerGroupId})`);
  }
  if (filters.viewerLocation !== undefined) {
    // POI zones resolve their geometry through Inventory (cached; empty = no mission
    // references a POI, so Inventory is never touched in that case).
    conditions.push(zoneFilterSql(filters.viewerLocation, await resolveListingPoiGeometry()));
  }

  const condition = conditions.length > 0 ? and(...conditions) : undefined;
  const [items, totalRows] = await Promise.all([
    db.select().from(missions).where(condition).orderBy(desc(missions.createdAt), desc(missions.id)).limit(limit).offset(offset),
    db.select({ total: count() }).from(missions).where(condition),
  ]);
  return page(items, totalRows[0]?.total ?? 0, limit, offset);
}

/**
 * Creates a mission and its objectives atomically (spec validated by the kind registry).
 * @param input - Mission and objective definitions.
 * @param createdBy - Keycloak client id of the creator service (or player id).
 * @returns The created mission and objectives.
 * @throws 409 when a `scriptId` is already used.
 */
export async function createMission(
  input: CreateMissionInput,
  createdBy: string,
): Promise<MissionWithObjectives> {
  const spec = validateMissionSpec(
    {
      category: input.category ?? 'generic',
      objectives: input.objectives,
      prerequisites: input.prerequisites,
      rewards: input.rewards,
      maxAssignees: input.maxAssignees ?? 1,
    },
    { escrowed: false },
  );
  await assertPoiZonesExist(input.zones);

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
          rewards: spec.rewards.length > 0 ? spec.rewards : null,
          prerequisites: spec.prerequisites,
          zones: input.zones ?? [],
          maxAssignees: input.maxAssignees ?? 1,
          groupClaimable: input.groupClaimable ?? false,
          scriptId: input.scriptId ?? null,
          expiresAt,
          createdBy,
        })
        .returning();

      const objectives = await tx
        .insert(missionObjectives)
        .values(spec.objectives.map((objective) => ({ missionId: mission.id, ...objective })))
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
 * Creates a player-sponsored mission. Reward components are escrowed from the creator:
 * the credits component is debited from their wallet through Economy, every item
 * component is held in their inventory through Inventory, then held until settlement or
 * refund. All escrows use deterministic keys, so a retried creation never debits/holds
 * twice; any refusal rolls everything back.
 * @param input - Mission definition (at least one reward component required).
 * @param playerId - Creating player (escrow payer).
 * @returns The created mission and objectives.
 */
/**
 * Whether the player may commit an organization's treasury (corporation or political
 * entity) to fund a mission. The decision is delegated to Social (`mission:treasury:commit`,
 * satisfied by `manage_corporation` for a corporation and `manage_treasury` for a
 * political entity); mission only maps the refusal onto its own error.
 * @param issuerType - `corporation` or `politics`.
 * @param issuerId - Organization id (opaque, owned by Social).
 * @param playerId - Acting player.
 * @throws 403 `TREASURY_FORBIDDEN` when the player may not commit the treasury.
 */
async function assertTreasuryAccess(
  issuerType: 'corporation' | 'politics',
  issuerId: string,
  playerId: string,
): Promise<void> {
  const corporate = issuerType === 'corporation';
  const decision = await authorizeAction({
    holderType: corporate ? 'corporation' : 'political',
    holderId: issuerId,
    playerId,
    action: 'mission:treasury:commit',
  });
  if (decision.allowed) return;
  if (decision.reason === 'not_member') {
    throw new HttpError(
      403,
      'TREASURY_FORBIDDEN',
      t(corporate ? 'treasury.not_corp_member' : 'treasury.not_politics_member'),
    );
  }
  throw new HttpError(
    403,
    'TREASURY_FORBIDDEN',
    t(corporate ? 'treasury.no_corp_permission' : 'treasury.no_treasury_permission'),
  );
}

export async function createPlayerMission(
  input: CreatePlayerMissionInput,
  playerId: string,
): Promise<MissionWithObjectives> {
  const spec = validateMissionSpec(
    {
      category: input.category ?? 'generic',
      objectives: input.objectives,
      prerequisites: input.prerequisites,
      rewards: input.rewards,
      maxAssignees: input.maxAssignees ?? 1,
    },
    { escrowed: true, requireReward: true },
  );
  await assertPoiZonesExist(input.zones);

  const visibility = input.visibility ?? 'public';
  if (visibility === 'corporation') {
    if (!input.issuerId) {
      throw new HttpError(400, 'CORPORATION_REQUIRED', 'A corporation mission requires an issuerId');
    }
    if (!(await isCorporationMember(playerId, input.issuerId))) {
      throw new HttpError(403, 'NOT_CORPORATION_MEMBER', t('corp.not_member_create'));
    }
  }

  // Issuer resolution: a political entity issuer takes over the default player/corp issuer.
  const politicalEntityId = input.politicalEntityId ?? null;
  const issuerType: MissionIssuerType = politicalEntityId
    ? 'politics'
    : input.issuerId
      ? 'corporation'
      : 'player';
  const issuerId: string | null = politicalEntityId ?? input.issuerId ?? null;
  const escrowSource = input.escrowSource ?? 'creator';
  if (escrowSource === 'issuer') {
    if (!issuerId || (issuerType !== 'corporation' && issuerType !== 'politics')) {
      throw new HttpError(
        400,
        'ESCROW_SOURCE_INVALID',
        'escrowSource "issuer" requires a corporation or political entity issuer',
      );
    }
    await assertTreasuryAccess(issuerType, issuerId, playerId);
  }
  const escrowPayerType: EscrowPayerType =
    escrowSource === 'issuer' ? (issuerType === 'politics' ? 'politics' : 'corporation') : 'player';
  // `issuerId` is non-null here: the `issuer` branch above throws otherwise.
  const escrowPayerId: string = escrowSource === 'issuer' ? issuerId! : playerId;

  const economic = spec.rewards.find((component) => component.type === 'credits');
  const items = spec.rewards.filter((component): component is Extract<RewardComponent, { type: 'item' }> => component.type === 'item');
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
        issuerType,
        issuerId,
        visibility,
        rewards: spec.rewards,
        prerequisites: spec.prerequisites,
        zones: input.zones ?? [],
        maxAssignees: input.maxAssignees ?? 1,
        groupClaimable: input.groupClaimable ?? false,
        expiresAt: input.expiresAt ?? null,
        escrowStatus: economic ? 'pending' : 'none',
        escrowAmount: economic ? economic.amount : null,
        escrowCurrency: economic ? economic.currency : null,
        escrowPayerId,
        escrowPayerType,
        escrowExternalId: economic ? escrowExternalId : null,
        escrowItemStatus: items.length > 0 ? 'held' : 'none',
        createdBy: playerId,
      })
      .returning();

    const objectives = await tx
      .insert(missionObjectives)
      .values(spec.objectives.map((objective) => ({ missionId, ...objective })))
      .returning();

    return { mission, objectives };
  });

  // Escrow the item components (one hold per item, aligned with the reward list order).
  const holdIds: string[] = [];
  if (items.length > 0) {
    try {
      for (const item of items) {
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
        holdIds.push(hold.id);
      }
      await db
        .update(missions)
        .set({ escrowItemHoldIds: holdIds, updatedAt: new Date() })
        .where(eq(missions.id, missionId));
    } catch (err) {
      await Promise.all(holdIds.map((holdId) => releaseHold(holdId).catch(() => undefined)));
      await db
        .update(missions)
        .set({ status: 'cancelled', escrowItemStatus: 'none', updatedAt: new Date() })
        .where(eq(missions.id, missionId));
      throw err;
    }
  }

  // Escrow the credits component (debited from the funding account: the creator's wallet
  // or the issuing organization's treasury).
  if (economic) {
    try {
      await debitHolder(escrowPayerType, escrowPayerId, {
        amount: economic.amount,
        currency: economic.currency,
        reference: 'mission_escrow',
        externalId: escrowExternalId,
      });
    } catch (err) {
      await Promise.all(holdIds.map((holdId) => releaseHold(holdId).catch(() => undefined)));
      await db
        .update(missions)
        .set({
          status: 'cancelled',
          escrowStatus: 'none',
          escrowItemStatus: 'none',
          escrowItemHoldIds: null,
          updatedAt: new Date(),
        })
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
      await creditHolder(mission.escrowPayerType ?? 'player', mission.escrowPayerId, {
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

  if (mission.escrowItemStatus === 'held' && mission.escrowItemHoldIds?.length) {
    await Promise.all(mission.escrowItemHoldIds.map((holdId) => releaseHold(holdId).catch(() => undefined)));
    await db
      .update(missions)
      .set({ escrowItemStatus: 'released', escrowItemHoldIds: null, updatedAt: new Date() })
      .where(and(eq(missions.id, mission.id), eq(missions.escrowItemStatus, 'held')));
  }
}

/**
 * Updates the mutable presentation fields of a mission. The spec (objectives,
 * prerequisites, rewards, capacity) is immutable after creation: the escrow is taken
 * against it at creation time.
 * @param missionId - Mission id.
 * @param patch - Fields to change.
 * @returns Updated mission.
 */
export async function updateMission(
  missionId: string,
  patch: {
    title?: string;
    description?: string | null;
    expiresAt?: Date | null;
    isEvent?: boolean;
    /** Re-zone a mission (internal API only; no escrow impact). */
    zones?: MissionZone[];
  },
): Promise<Mission> {
  if (patch.zones) await assertPoiZonesExist(patch.zones);
  const [updated] = await db
    .update(missions)
    .set({ ...patch, updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();
  if (!updated) throw notFound(t('not_found.mission', { id: missionId }));
  return updated;
}

/**
 * Completes an `issuer`-confirmed objective (the mission creator, or the game server
 * through the internal API). Scenario ordering still applies.
 * @param missionId - Mission id.
 * @param objectiveId - Objective to confirm.
 * @returns The confirmed objective, all objectives and whether the mission is complete.
 * @throws 409 when the mission is not open, the objective is not confirmable, or the
 *   objective is locked by the scenario order.
 */
export async function confirmObjective(
  missionId: string,
  objectiveId: string,
): Promise<{ objective: MissionObjective; objectives: MissionObjective[]; allComplete: boolean }> {
  const mission = await requireMission(missionId);
  requireOpenMission(mission);

  const objective = await requireObjective(missionId, objectiveId);
  const kindDef = getObjectiveKind(objective.type);
  if (!kindDef || kindDef.evaluation !== 'issuer') {
    throw new HttpError(409, 'OBJECTIVE_NOT_CONFIRMABLE', 'Only issuer-confirmed objectives can be confirmed');
  }
  const objectives = await listObjectives(missionId);
  assertObjectiveUnlocked(mission, objective, objectives);

  if (objective.status !== 'completed') {
    await db
      .update(missionObjectives)
      .set({ status: 'completed', currentProgress: objective.targetQuantity, updatedAt: new Date() })
      .where(eq(missionObjectives.id, objective.id));
  }
  const refreshed = await listObjectives(missionId);
  const updated = refreshed.find((row) => row.id === objective.id) ?? objective;
  return { objective: updated, objectives: refreshed, allComplete: allObjectivesCompleted(refreshed) };
}

/**
 * Whether a player may manage a mission (share/unshare/flag as event): its creator, or a
 * member of the issuing corporation.
 * @param mission - Mission to check.
 * @param playerId - Acting player.
 * @throws 403 when the player may not manage the mission.
 */
async function assertCanManageMission(mission: Mission, playerId: string): Promise<void> {
  if (mission.createdBy === playerId) return;
  if (mission.issuerType === 'corporation' && mission.issuerId) {
    if (await isCorporationMember(playerId, mission.issuerId)) return;
    throw new HttpError(403, 'NOT_CORPORATION_MEMBER', t('corp.not_member_manage'));
  }
  throw new HttpError(403, 'NOT_MISSION_MANAGER', 'Only the creator of the mission can manage it');
}

/**
 * Shares a mission with a group: only its members will be able to see and accept it, and
 * it counts as taken by that group. A mission with active assignees cannot be shared.
 * @param missionId - Mission id.
 * @param groupId - Target group (must exist in Social).
 * @param opts.actorId - Acting player (required unless `trusted`).
 * @param opts.trusted - Internal caller (game server): skips the ownership check.
 * @returns Updated mission.
 */
export async function shareMission(
  missionId: string,
  groupId: string,
  opts: { actorId?: string; trusted?: boolean } = {},
): Promise<Mission> {
  const mission = await requireMission(missionId);
  if (!opts.trusted) await assertCanManageMission(mission, opts.actorId ?? '');
  if (!(await getGroup(groupId))) throw notFound(t('not_found.group', { id: groupId }));
  if ((await countActiveAssignments(missionId)) > 0) {
    throw new HttpError(409, 'MISSION_HAS_ASSIGNEES', t('share.blocked'));
  }
  const [updated] = await db
    .update(missions)
    .set({ groupId, updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();
  if (!updated) throw notFound(t('not_found.mission', { id: missionId }));
  return updated;
}

/**
 * Removes the group share of a mission (back to public/corporation access). Only possible
 * while no assignee is active.
 * @param missionId - Mission id.
 * @param opts.actorId - Acting player (required unless `trusted`).
 * @param opts.trusted - Internal caller (game server): skips the ownership check.
 * @returns Updated mission.
 */
export async function unshareMission(
  missionId: string,
  opts: { actorId?: string; trusted?: boolean } = {},
): Promise<Mission> {
  const mission = await requireMission(missionId);
  if (!opts.trusted) await assertCanManageMission(mission, opts.actorId ?? '');
  if ((await countActiveAssignments(missionId)) > 0) {
    throw new HttpError(409, 'MISSION_HAS_ASSIGNEES', t('share.unblocked'));
  }
  const [updated] = await db
    .update(missions)
    .set({ groupId: null, updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();
  if (!updated) throw notFound(t('not_found.mission', { id: missionId }));
  return updated;
}

/**
 * Declares (or lifts) a "big event" flag on a corporation mission. Events allow a capacity
 * up to 1000 assignees; the flag can only be set by a member authorized by Social
 * (`mission:event:manage`, satisfied by `manage_corporation` — the CEO passes implicitly).
 * @param missionId - Mission id.
 * @param playerId - Acting player.
 * @param opts.enabled - Whether the mission is an event.
 * @param opts.maxAssignees - Optional new capacity (1..1000).
 * @returns Updated mission.
 */
export async function setMissionEvent(
  missionId: string,
  playerId: string,
  opts: { enabled: boolean; maxAssignees?: number },
): Promise<Mission> {
  const mission = await requireMission(missionId);
  if (mission.issuerType !== 'corporation' || !mission.issuerId) {
    throw new HttpError(403, 'NOT_CORPORATION_MISSION', 'Only corporation missions can be flagged as events');
  }
  const decision = await authorizeAction({
    holderType: 'corporation',
    holderId: mission.issuerId,
    playerId,
    action: 'mission:event:manage',
  });
  if (decision.reason === 'not_member') {
    throw new HttpError(403, 'NOT_CORPORATION_MEMBER', t('corp.not_member_manage'));
  }
  if (!decision.allowed) {
    throw new HttpError(403, 'EVENT_FORBIDDEN', 'Requires the mission:event:manage permission');
  }

  const maxAssignees = opts.maxAssignees ?? mission.maxAssignees;
  if (opts.enabled && maxAssignees > 1000) {
    throw new HttpError(400, 'INVALID_MAX_ASSIGNEES', t('event.capacity_1000'));
  }
  if (!opts.enabled && maxAssignees > 100) {
    throw new HttpError(
      400,
      'EVENT_CAPACITY_REQUIRES_FLAG',
      'Lower maxAssignees to 100 or below before lifting the event flag',
    );
  }

  const [updated] = await db
    .update(missions)
    .set({ isEvent: opts.enabled, maxAssignees, updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();
  if (!updated) throw notFound(t('not_found.mission', { id: missionId }));
  return updated;
}

/**
 * Internal variant of the event flag (game server): no corporation membership check, any
 * mission may be flagged.
 * @param missionId - Mission id.
 * @param opts - Enabled flag and optional capacity.
 * @returns Updated mission.
 */
export async function setMissionEventInternal(
  missionId: string,
  opts: { enabled: boolean; maxAssignees?: number },
): Promise<Mission> {
  const mission = await requireMission(missionId);
  const maxAssignees = opts.maxAssignees ?? mission.maxAssignees;
  if (maxAssignees < 1 || maxAssignees > 1000) {
    throw new HttpError(400, 'INVALID_MAX_ASSIGNEES', t('event.capacity_range'));
  }
  const [updated] = await db
    .update(missions)
    .set({ isEvent: opts.enabled, maxAssignees, updatedAt: new Date() })
    .where(eq(missions.id, missionId))
    .returning();
  if (!updated) throw notFound(t('not_found.mission', { id: missionId }));
  return updated;
}

/**
 * Rejects acceptance/progress when the mission is not open.
 * @param mission - Mission to check.
 */
export function requireOpenMission(mission: Mission): void {
  if (mission.status !== 'available' && mission.status !== 'active') {
    throw new HttpError(409, 'MISSION_NOT_OPEN', `Mission is ${mission.status}`, { status: mission.status });
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

/** True when a scenario objective is still locked by an incomplete lower-`order` objective. */
export function isObjectiveLocked(
  mission: Mission,
  objective: MissionObjective,
  objectives: MissionObjective[],
): boolean {
  if (mission.kind !== 'scenario') return false;
  return objectives.some((other) => other.order < objective.order && other.status !== 'completed');
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
  if (isObjectiveLocked(mission, objective, objectives)) {
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
