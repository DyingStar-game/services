/**
 * Full mission detail for internal callers (the admin panel): the mission, its objectives
 * in declaration order, **every** assignment (who took it, status, frozen reward shares)
 * and a best-effort Social enrichment (assignee display names, the group the mission was
 * shared with or claimed by).
 *
 * The internal channel is deliberately blind to visibility rules: a mission shared with a
 * group reads back here for any service holding `mission:read`, unlike the player route
 * which hides it from non-members. Social lookups are cached and fail soft — an outage
 * degrades the response to ids only, never to an error.
 */
import type { Mission, MissionAssignment, MissionObjective } from '../db/schema/index.js';
import { getMissionById, listObjectives, requiredProgressBp } from './missions.service.js';
import { listAssignmentsForMission, listMissionsByIds } from './assignments.service.js';
import { getGroup, getPlayerProfile } from './social.client.js';

/** How long a Social lookup result (name or group) is cached. */
const LOOKUP_TTL_MS = 60_000;
/** Upper bound on distinct Social lookups performed for one detail (bounded fan-out). */
const LOOKUP_LIMIT = 200;

/** Per-participant counts of a mission's assignments. */
export interface MissionSummary {
  /** Every assignment ever made, whatever its status. */
  assignments: number;
  /** Assignments still in progress. */
  active: number;
  /** Full completions. */
  completed: number;
  /** Partial hand-ins (paid proportionally). */
  partial: number;
  abandoned: number;
  expired: number;
  /** Weighted progress of the required objectives, in basis points (10000 = all done). */
  progressBp: number;
}

/** One entry of the scenario chain required by this mission. */
export interface MissionChainEntry {
  id: string;
  title: string;
  status: string;
}

/** Mission detail as served by `GET /api/internal/missions/:missionId`. */
export interface InternalMissionDetail {
  mission: Mission;
  objectives: MissionObjective[];
  assignments: MissionAssignment[];
  /** Assignee id → display name (empty map when Social is unreachable). */
  players: Record<string, string>;
  /** The group this mission is shared with / claimed by, or null. */
  group: { id: string; name: string } | null;
  /** Missions that must be completed before this one can be accepted. */
  chain: MissionChainEntry[];
  summary: MissionSummary;
}

interface CacheEntry {
  value: string | null;
  expiresAt: number;
}

const profileCache = new Map<string, CacheEntry>();
const groupCache = new Map<string, CacheEntry>();

/**
 * Reads a cached Social lookup; a failure (including Social being unconfigured) is cached
 * as `null` so a down service is not hammered on every panel refresh.
 * @param cache - Cache to read from.
 * @param key - Cache key (player or group id).
 * @param load - Loader run on a miss.
 * @returns The value, or null.
 */
async function cached(
  cache: Map<string, CacheEntry>,
  key: string,
  load: () => Promise<string | null>,
): Promise<string | null> {
  const now = Date.now();
  const hit = cache.get(key);
  if (hit && hit.expiresAt > now) return hit.value;
  let value: string | null = null;
  try {
    value = await load();
  } catch {
    value = null;
  }
  cache.set(key, { value, expiresAt: now + LOOKUP_TTL_MS });
  return value;
}

/** Resolves the display name of each assignee, best-effort and in parallel. */
async function resolvePlayerNames(playerIds: string[]): Promise<Record<string, string>> {
  const names: Record<string, string> = {};
  const ids = [...new Set(playerIds)].slice(0, LOOKUP_LIMIT);
  await Promise.all(
    ids.map(async (playerId) => {
      const name = await cached(profileCache, playerId, async () => {
        const profile = await getPlayerProfile(playerId);
        return profile?.displayName ?? null;
      });
      if (name) names[playerId] = name;
    }),
  );
  return names;
}

/** Resolves the name of the mission's group, best-effort. */
async function resolveGroup(groupId: string | null): Promise<{ id: string; name: string } | null> {
  if (!groupId) return null;
  const name = await cached(groupCache, groupId, async () => (await getGroup(groupId))?.name ?? null);
  return name === null ? null : { id: groupId, name };
}

/** Counts the assignments per status and computes the mission's weighted progress. */
function summarize(
  assignments: MissionAssignment[],
  objectives: MissionObjective[],
): MissionSummary {
  const summary: MissionSummary = {
    assignments: assignments.length,
    active: 0,
    completed: 0,
    partial: 0,
    abandoned: 0,
    expired: 0,
    progressBp: requiredProgressBp(objectives),
  };
  for (const assignment of assignments) {
    if (assignment.status === 'active') summary.active += 1;
    else if (assignment.status === 'completed') summary.completed += 1;
    else if (assignment.status === 'partially_completed') summary.partial += 1;
    else if (assignment.status === 'abandoned') summary.abandoned += 1;
    else if (assignment.status === 'expired') summary.expired += 1;
  }
  return summary;
}

/**
 * Builds the internal mission detail (mission + ordered objectives + assignments + Social
 * enrichment + scenario chain).
 * @param missionId - Mission id.
 * @returns The detail, or null when the mission does not exist.
 */
export async function getInternalMissionDetail(missionId: string): Promise<InternalMissionDetail | null> {
  const mission = await getMissionById(missionId);
  if (!mission) return null;

  const [objectives, assignments] = await Promise.all([
    listObjectives(missionId),
    listAssignmentsForMission(missionId),
  ]);
  const [players, group, chainMissions] = await Promise.all([
    resolvePlayerNames(assignments.map((assignment) => assignment.playerId)),
    resolveGroup(mission.groupId),
    mission.requiresMissionIds.length > 0 ? listMissionsByIds(mission.requiresMissionIds) : Promise.resolve([]),
  ]);

  const chainById = new Map(chainMissions.map((row) => [row.id, row]));
  return {
    mission,
    objectives,
    assignments,
    players,
    group,
    chain: mission.requiresMissionIds.map((id) => {
      const required = chainById.get(id);
      return { id, title: required?.title ?? '', status: required?.status ?? 'unknown' };
    }),
    summary: summarize(assignments, objectives),
  };
}
