/**
 * Ephemeral player notifications: presence-gated, best-effort delivery over MQTT
 * (`notify/{playerId}`, QoS 0, not retained). Nothing is stored — an offline player
 * simply does not receive the message; callers needing a history keep their own.
 */
import { randomUUID } from 'node:crypto';

import { HttpError } from '../lib/httpError.js';
import { publishNotification } from '../mqtt/publisher.js';
import { getPresence } from './presence.service.js';

/** What a caller may send; social wraps it into the published envelope. */
export interface NotificationInput {
  type: string;
  title?: string;
  body?: string;
  data?: Record<string, unknown>;
}

/** Outcome of a notification attempt. */
export interface NotifyResult {
  delivered: boolean;
  /** Why the message was not handed to the broker (absent when delivered). */
  reason?: 'offline' | 'unreachable';
}

/** Maximum size of the serialized envelope handed to the broker. */
export const NOTIFICATION_MAX_BYTES = 8 * 1024;

/**
 * Notifies a player if they are online; never blocks on delivery failures.
 * @param playerId - Target player.
 * @param input - Notification content (`type` drives client-side rendering).
 * @returns Whether the broker accepted the message.
 */
export async function notifyPlayer(playerId: string, input: NotificationInput): Promise<NotifyResult> {
  const presence = await getPresence(playerId);
  if (presence.status === 'offline') return { delivered: false, reason: 'offline' };

  const payload = JSON.stringify({
    id: randomUUID(),
    type: input.type,
    ...(input.title !== undefined ? { title: input.title } : {}),
    ...(input.body !== undefined ? { body: input.body } : {}),
    ...(input.data !== undefined ? { data: input.data } : {}),
    sentAt: new Date().toISOString(),
  });
  if (Buffer.byteLength(payload, 'utf8') > NOTIFICATION_MAX_BYTES) {
    throw new HttpError(413, 'NOTIFICATION_TOO_LARGE', `Notification exceeds ${NOTIFICATION_MAX_BYTES} bytes`);
  }

  const delivered = await publishNotification(playerId, payload);
  return delivered ? { delivered } : { delivered: false, reason: 'unreachable' };
}
