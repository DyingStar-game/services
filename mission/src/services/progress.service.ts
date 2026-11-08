/**
 * The progress of the missions of a list, read in one query (see [summarizeProgress]).
 */
import { asc, inArray } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { missionObjectives, type MissionObjective } from '../db/schema/index.js';
import { summarizeProgress, type MissionProgress } from '../lib/progress.js';

/**
 * The progress of each mission of a list, in one query.
 * @param missionIds - Mission ids.
 * @returns Mission id → its progress (a mission without objectives gets an empty summary).
 */
export async function progressOfMissions(missionIds: string[]): Promise<Map<string, MissionProgress>> {
  const ids = [...new Set(missionIds)];
  const byMission = new Map<string, MissionObjective[]>(ids.map((id) => [id, []]));
  if (ids.length > 0) {
    const rows = await db
      .select()
      .from(missionObjectives)
      .where(inArray(missionObjectives.missionId, ids))
      .orderBy(asc(missionObjectives.order), asc(missionObjectives.createdAt));
    for (const row of rows) byMission.get(row.missionId)?.push(row);
  }
  return new Map([...byMission].map(([id, objectives]) => [id, summarizeProgress(objectives)]));
}
