/**
 * Best-effort mission notifications relayed by Social to the player's MQTT channel.
 *
 * A notification never fails the gameplay action that triggered it: Social being
 * unconfigured or down is swallowed here, and nothing is ever persisted (messages are
 * ephemeral — an offline player simply misses them). Payloads are structured (`type` +
 * `data`) so the client owns the rendering.
 */
import { notifyPlayer, type PlayerNotification } from './social.client.js';

/** Notification types emitted by the mission service. */
export type MissionNotificationType =
  /** An assignment reached its per-assignment deadline and was expired. */
  | 'mission.deadline'
  /** The mission itself reached its `expiresAt` and was closed. */
  | 'mission.expired'
  /** Every required objective was completed and the reward was settled. */
  | 'mission.completed'
  /** The player handed the mission in proportionally (partial completion). */
  | 'mission.partially_completed'
  /** A measurement failed permanently and the assignment was failed. */
  | 'mission.failed'
  /** The creator (or an admin) cancelled the mission. */
  | 'mission.cancelled';

/** Sends a mission notification, swallowing every failure. */
export async function notifyMissionEvent(
  playerId: string,
  notification: { type: MissionNotificationType; data: Record<string, unknown> },
): Promise<void> {
  const payload: PlayerNotification = { type: notification.type, data: notification.data };
  await notifyPlayer(playerId, payload).catch(() => undefined);
}
