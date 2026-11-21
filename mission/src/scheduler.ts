/**
 * Built-in periodic sweep: closes the missions and assignments that reached their
 * deadline, notifying their holders.
 *
 * The service is single-instance by design (one Postgres, one process), so a plain
 * `setInterval` is enough; the sweep itself is idempotent, which also makes an overlapping
 * run harmless. Runs start after the first interval, never at boot.
 */
import { env } from './config/env.js';
import { expireDueMissions } from './services/missions.service.js';
import { expireDueAssignments } from './services/assignments.service.js';

/** Interval timer kept so tests (and shutdown) can stop it. */
let timer: NodeJS.Timeout | undefined;

/**
 * Runs one sweep: mission TTLs first, then the per-assignment deadlines.
 * @returns The number of expired missions and assignments.
 */
export async function runExpirySweep(): Promise<{ missions: number; assignments: number }> {
  const missions = await expireDueMissions();
  const assignments = await expireDueAssignments();
  return { missions, assignments };
}

/**
 * Starts the built-in sweep when `MISSION_EXPIRE_INTERVAL_SECONDS` is positive.
 * @returns The timer, or undefined when the sweep is disabled.
 */
export function startExpirySweep(): NodeJS.Timeout | undefined {
  if (env.mission.expireIntervalSeconds <= 0 || timer) return timer;
  timer = setInterval(() => {
    runExpirySweep().catch((err: unknown) => {
      // Never let a failed sweep crash the process: the next tick retries.
      console.error('[scheduler] expiry sweep failed', err);
    });
  }, env.mission.expireIntervalSeconds * 1000);
  timer.unref();
  return timer;
}

/** Stops the built-in sweep (shutdown / tests). */
export function stopExpirySweep(): void {
  if (timer) clearInterval(timer);
  timer = undefined;
}
