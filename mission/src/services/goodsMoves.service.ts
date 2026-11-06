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
 * source. Each move is counted once per objective (`moveKeys`).
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
 * Applies [move] to every warehouse-measured objective of the player's active missions, then
 * re-measures those missions (which completes them when they auto-complete).
 * @returns The objectives that took the move into account.
 */
export async function recordGoodsMove(move: GoodsMove): Promise<{ applied: string[] }> {
  const assignments = await db
    .select({ missionId: missionAssignments.missionId })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.playerId, move.playerId), eq(missionAssignments.status, ACTIVE_ASSIGNMENT_STATUS)));
  const missionIds = assignments.map((a) => a.missionId);
  if (missionIds.length === 0) return { applied: [] };

  const objectives = (
    await db.select().from(missionObjectives).where(inArray(missionObjectives.missionId, missionIds))
  ).filter((o) => (o.params as WarehouseParams | null)?.via === 'warehouse');
  if (objectives.length === 0) return { applied: [] };

  const warehouseOf = await warehousesOfPois(objectives);
  const applied: string[] = [];
  const touched = new Set<string>();
  /** Missions this move may have finished: one of their objectives was not completed before it. */
  const mayFinish = new Set<string>();
  for (const objective of objectives) {
    const next = applyMove(objective, move, warehouseOf);
    if (next === null) continue;
    await db.update(missionObjectives).set({ state: next, updatedAt: new Date() }).where(eq(missionObjectives.id, objective.id));
    applied.push(objective.id);
    touched.add(objective.missionId);
    if (objective.status !== 'completed') mayFinish.add(objective.missionId);
  }
  for (const missionId of touched) {
    try {
      const result = await verifyPlayerObjectives(missionId, move.playerId);
      // Done but waiting for a town hall: say so once, when this move is the one that finished it.
      if (result.allComplete && !result.completed && mayFinish.has(missionId)) {
        await notifyReady(missionId);
      }
    } catch {
      // A mission closed or expired meanwhile: the move is recorded, nothing to measure.
    }
  }
  return { applied };
}

/** Tells every active assignee of [missionId] that it is ready to be turned in at a town hall. */
async function notifyReady(missionId: string): Promise<void> {
  const mission = await getMissionById(missionId);
  const assignees = await db
    .select({ playerId: missionAssignments.playerId })
    .from(missionAssignments)
    .where(and(eq(missionAssignments.missionId, missionId), eq(missionAssignments.status, ACTIVE_ASSIGNMENT_STATUS)));
  await Promise.all(
    assignees.map((a) =>
      notifyMissionEvent(a.playerId, { type: 'mission.ready', data: { missionId, title: mission?.title ?? '' } }),
    ),
  );
}

/**
 * The objective's new state once [move] is counted, or null when the move does not concern it.
 * Pure: exported for the tests.
 */
export function applyMove(
  objective: Pick<MissionObjective, 'type' | 'params' | 'state'>,
  move: GoodsMove,
  warehouseOf: ReadonlyMap<string, string>,
): ObjectiveState | null {
  const params = (objective.params ?? {}) as WarehouseParams;
  const state = { ...((objective.state ?? {}) as ObjectiveState) };
  const keys = state.moveKeys ?? [];
  if (keys.includes(move.key)) return null;
  const zoneWarehouse = warehouseOf.get(params.zone?.poiId ?? '');
  const deliverGood = params.deliverGoodType ?? params.goodType;

  if (objective.type === 'deliver_material') {
    if (move.direction !== 'drop' || move.goodType !== params.goodType || zoneWarehouse !== move.warehouseId) return null;
    state.delivered = (state.delivered ?? 0) + move.quantity;
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
      state.delivered = (state.delivered ?? 0) + move.quantity;
    } else {
      return null;
    }
  } else {
    return null;
  }
  state.moveKeys = [...keys, move.key].slice(-MAX_KEYS);
  return state;
}

/** POI id → warehouse holder id (`metadata.persistence`), for every zone/source POI of [objectives]. */
async function warehousesOfPois(objectives: MissionObjective[]): Promise<Map<string, string>> {
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
    // Inventory down: no warehouse resolves, the move counts nowhere (the game logs its own call).
  }
  return out;
}
