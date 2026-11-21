/**
 * How far a mission has got, in a few fields: what a list row needs to say "step 2 of 4, 30 / 90 kg"
 * without the client reading every mission in full (the DataPad's mission cards and its home).
 */
import type { MissionObjective } from '../db/schema/index.js';

/** The objective under way, as a list row shows it. */
export interface ProgressStep {
  id: string;
  type: string;
  title: string;
  /** Same text per language; the localize middleware turns it into `title`. */
  titleI18n: Record<string, string> | null;
  status: string;
  currentProgress: number;
  targetQuantity: number;
  unit: string | null;
}

/** A mission's progress summary (`progress` of a `GET /api/me/missions` row). */
export interface MissionProgress {
  /** 1-based number of the step under way; equals `steps` once every step is done. */
  step: number;
  /** Required objectives (bonus ones left out). */
  steps: number;
  /** Required objectives completed. */
  done: number;
  /** The step under way, or null when every required step is done (or there is none). */
  current: ProgressStep | null;
  /** Where the mission goes: the first required objective naming both ends (opaque locations). */
  from: Record<string, unknown> | null;
  to: Record<string, unknown> | null;
}

/**
 * Summarizes a mission's objectives (in their chain order) into its progress. Bonus objectives
 * neither count as steps nor block one.
 * @param objectives - The mission's objectives, ordered.
 * @returns Its progress summary.
 */
export function summarizeProgress(objectives: MissionObjective[]): MissionProgress {
  const required = objectives.filter((o) => !o.isBonus);
  const index = required.findIndex((o) => o.status !== 'completed');
  const done = required.filter((o) => o.status === 'completed').length;
  const under = index >= 0 ? required[index] : null;
  const ends = required.find((o) => o.locationFrom && o.locationTo) ?? null;
  return {
    step: index >= 0 ? index + 1 : required.length,
    steps: required.length,
    done,
    current: under
      ? {
          id: under.id,
          type: under.type,
          title: under.title,
          titleI18n: under.titleI18n ?? null,
          status: under.status,
          currentProgress: under.currentProgress,
          targetQuantity: under.targetQuantity,
          unit: under.unit ?? null,
        }
      : null,
    from: ends?.locationFrom ?? null,
    to: ends?.locationTo ?? null,
  };
}
