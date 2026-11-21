/**
 * Goods moved between a player and a warehouse, reported by the game server when a crate is
 * shelved in a cargo depot (drop) or taken off one (load). They are what a `via: 'warehouse'`
 * objective counts: a delivery is fulfilled when the goods are IN the warehouse, not when the
 * player merely walks into its zone carrying them.
 *
 * The warehouse is named by its inventory holder id (the cargo depot's uuid); an objective names
 * its warehouse by a POI whose `metadata.persistence` is that id (universesimulator creates them).
 * The running totals live in the objective's `state` (shared by every assignee, as objective
 * progress already is): `delivered` for drops, `loadedAt` latched by a load at a transport's
 * source. Each move is counted once per objective (`moveKeys`), and what it filled is kept
 * (`moveUsed`): a replayed move answers what it counted the first time.
 */
import { and, eq, inArray } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { ACTIVE_ASSIGNMENT_STATUS, missionAssignments } from '../db/schema/assignments.js';
import { missionObjectives, type MissionObjective } from '../db/schema/index.js';
import { resolvePois } from './inventory.client.js';
import { verifyPlayerObjectives } from './assignments.service.js';
import { getMissionById } from './missions.service.js';
import { notifyMissionEvent } from './notifications.service.js';

/** A move reported by the game server. */
export interface GoodsMove {
  playerId: string;
  goodType: string;
  quantity: number;
  /** `drop`: player → warehouse; `load`: warehouse → player. */
  direction: 'drop' | 'load';
  /** Inventory holder id of the warehouse (the cargo depot's uuid). */
  warehouseId: string;
  /** Unique per move (the game's crate uuid + direction + time): a retry counts nothing twice. */
  key: string;
}

/** How many move keys an objective remembers (enough for any delivery; bounded for jsonb). */
const MAX_KEYS = 200;

type ObjectiveState = {
  delivered?: number;
  loadedAt?: string;
  moveKeys?: string[];
  /** What each remembered move filled of this objective (0 for a load). */
  moveUsed?: Record<string, number>;
} & Record<string, unknown>;

/** Params a warehouse-measured objective carries (deliver_material or transport). */
interface WarehouseParams {
  via?: string;
  goodType?: string;
  deliverGoodType?: string;
  zone?: { poiId?: string };
  source?: { poiId?: string };
}

/**
 * Applies [move] to the warehouse-measured objectives of the player's active missions, then
 * re-measures the missions it touched (which completes the ones that auto-complete).
 *
 * A dropped crate counts ONCE: it fills the objectives of the oldest assignment first, the rest
 * carried over to the next — never the same goods for two missions. A load latches every transport
 * it starts (it delivers nothing). The objectives are read and written in one transaction, locked,
 * so two crates shelved at the same moment cannot overwrite each other's count.
 * @returns The objectives that took the move into account, and how much of it they counted (`counted`:
 *   what is left of a drop is the player's own sale, paid by the universe simulator). A replayed move
 *   (same key) answers the same `counted` and applies nothing again; `counted` is null when the
 *   warehouses could not be resolved (inventory down), so that nobody sells on a guess.
 */
export async function recordGoodsMove(move: GoodsMove): Promise<{ applied: string[]; counted: number | null }> {
  const assignments = await db
    .select({ missionId: missionAssignments.missionId, acceptedAt: missionAssignments.acceptedAt })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.playerId, move.playerId), eq(missionAssignments.status, ACTIVE_ASSIGNMENT_STATUS)));
  if (assignments.length === 0) return { applied: [], counted: 0 };
  const acceptedAt = new Map(assignments.map((a) => [a.missionId, a.acceptedAt.getTime()]));

  const candidates = (
    await db.select().from(missionObjectives).where(inArray(missionObjectives.missionId, [...acceptedAt.keys()]))
  ).filter((o) => (o.params as WarehouseParams | null)?.via === 'warehouse');
  if (candidates.length === 0) return { applied: [], counted: 0 };
  const warehouseOf = await warehousesOfPois(candidates);
  if (warehouseOf === null) return { applied: [], counted: null };

  const applied: string[] = [];
  let counted = 0;
  const touched = new Set<string>();
  /** Missions this move may have finished: one of their objectives was not completed before it. */
  const mayFinish = new Set<string>();
  await db.transaction(async (tx) => {
    const locked = await tx
      .select()
      .from(missionObjectives)
      .where(inArray(missionObjectives.id, candidates.map((o) => o.id)))
      .for('update');
    locked.sort((a, b) => (acceptedAt.get(a.missionId) ?? 0) - (acceptedAt.get(b.missionId) ?? 0) || a.order - b.order);
    let left = move.quantity;
    for (const objective of locked) {
      // A replay: what this objective counted of the move the first time, nothing applied again.
      const before = usedBefore(objective, move);
      if (before !== null) {
        counted += before;
        left -= before;
        continue;
      }
      if (move.direction === 'drop' && left <= 0) break;
      const result = applyMove(objective, { ...move, quantity: left }, warehouseOf);
      if (result === null) continue;
      await tx.update(missionObjectives).set({ state: result.state, updatedAt: new Date() }).where(eq(missionObjectives.id, objective.id));
      applied.push(objective.id);
      touched.add(objective.missionId);
      if (objective.status !== 'completed') mayFinish.add(objective.missionId);
      left -= result.used;
      counted += result.used;
      if (move.direction === 'drop' && left <= 0) break;
    }
  });
  for (const missionId of touched) {
    try {
      const result = await verifyPlayerObjectives(missionId, move.playerId);
      // Done but waiting for a town hall: say so once, when this move is the one that finished it.
      // Otherwise the assignees hear of the progress: their DataPad reads the mission again.
      if (result.allComplete && !result.completed && mayFinish.has(missionId)) {
        await notifyAssignees(missionId, 'mission.ready');
      } else if (!result.allComplete) {
        await notifyAssignees(missionId, 'mission.progress');
      }
    } catch {
      // A mission closed or expired meanwhile: the move is recorded, nothing to measure.
    }
  }
  return { applied, counted: Math.min(counted, move.quantity) };
}

/**
 * Tells every active assignee of [missionId]: `mission.ready` (to be turned in at a town hall) or
 * `mission.progress` (a goods move counted for it).
 */
async function notifyAssignees(missionId: string, type: 'mission.ready' | 'mission.progress'): Promise<void> {
  const mission = await getMissionById(missionId);
  const assignees = await db
    .select({ playerId: missionAssignments.playerId })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, ACTIVE_ASSIGNMENT_STATUS)));
  await Promise.all(
    assignees.map((a) =>
      notifyMissionEvent(a.playerId, { type, data: { missionId, title: mission?.title ?? '' } }),
    ),
  );
}

/**
 * What [objective] counted of [move] when it first saw it, or null when it never did. A move it
 * remembers without its count (recorded before the count was kept) answers the whole quantity:
 * better to sell nothing than to pay twice. Pure: exported for the tests.
 */
export function usedBefore(objective: Pick<MissionObjective, 'state'>, move: Pick<GoodsMove, 'key' | 'quantity'>): number | null {
  const state = (objective.state ?? {}) as ObjectiveState;
  if (!(state.moveKeys ?? []).includes(move.key)) return null;
  return state.moveUsed?.[move.key] ?? move.quantity;
}

/**
 * The objective's new state once [move] is counted, or null when the move does not concern it.
 * Pure: exported for the tests.
 */
export function applyMove(
  objective: Pick<MissionObjective, 'type' | 'params' | 'state' | 'targetQuantity'>,
  move: GoodsMove,
  warehouseOf: ReadonlyMap<string, string>,
): { state: ObjectiveState; used: number } | null {
  const params = (objective.params ?? {}) as WarehouseParams;
  const state = { ...((objective.state ?? {}) as ObjectiveState) };
  const keys = state.moveKeys ?? [];
  if (keys.includes(move.key)) return null;
  const zoneWarehouse = warehouseOf.get(params.zone?.poiId ?? '');
  const deliverGood = params.deliverGoodType ?? params.goodType;
  // A drop fills what the objective still needs, no more: the rest goes to the next one.
  const fill = (): number => Math.min(move.quantity, Math.max(0, objective.targetQuantity - (state.delivered ?? 0)));
  let used = 0;

  if (objective.type === 'deliver_material') {
    if (move.direction !== 'drop' || move.goodType !== params.goodType || zoneWarehouse !== move.warehouseId) return null;
    used = fill();
    if (used <= 0) return null;
    state.delivered = (state.delivered ?? 0) + used;
  } else if (objective.type === 'transport') {
    const sourceWarehouse = warehouseOf.get(params.source?.poiId ?? '');
    if (move.direction === 'load' && move.goodType === params.goodType && sourceWarehouse === move.warehouseId) {
      if (state.loadedAt) return null;
      state.loadedAt = new Date().toISOString();
    } else if (
      move.direction === 'drop' &&
      state.loadedAt &&
      move.goodType === deliverGood &&
      zoneWarehouse === move.warehouseId
    ) {
      used = fill();
      if (used <= 0) return null;
      state.delivered = (state.delivered ?? 0) + used;
    } else {
      return null;
    }
  } else {
    return null;
  }
  state.moveKeys = [...keys, move.key].slice(-MAX_KEYS);
  const usedOf = { ...(state.moveUsed ?? {}), [move.key]: used };
  state.moveUsed = Object.fromEntries(state.moveKeys.filter((k) => k in usedOf).map((k) => [k, usedOf[k] as number]));
  return { state, used };
}

/**
 * POI id → warehouse holder id (`metadata.persistence`), for every zone/source POI of [objectives];
 * null when inventory could not be asked: the move then counts nowhere and its key is not recorded.
 */
async function warehousesOfPois(objectives: MissionObjective[]): Promise<Map<string, string> | null> {
  const ids = new Set<string>();
  for (const o of objectives) {
    const p = (o.params ?? {}) as WarehouseParams;
    if (p.zone?.poiId) ids.add(p.zone.poiId);
    if (p.source?.poiId) ids.add(p.source.poiId);
  }
  const out = new Map<string, string>();
  try {
    const pois = await resolvePois([...ids]);
    for (const poi of pois.values()) {
      const persistence = (poi as { metadata?: { persistence?: unknown } | null }).metadata?.persistence;
      if (typeof persistence === 'string' && persistence !== '') out.set(poi.id, persistence);
    }
  } catch {
    // Inventory down: nothing can be decided (the game logs its own call, and sells nothing).
    return ids.size > 0 ? null : out;
  }
  return out;
}
