/**
 * MQTT publisher for ephemeral player notifications.
 *
 * Best-effort by design: QoS 0, never retained, no offline queue — a message for a
 * player whose subscription is not live simply vanishes in the broker (the presence
 * check in `notifications.service` avoids even publishing for offline players).
 *
 * Authentication mirrors the chat clients: our own Keycloak service account token,
 * sent as the MQTT password (mosquitto-go-auth forwards it to chatauth, which allows
 * the service client on `notify/#`). The token is fetched per connection attempt and
 * connections never auto-reconnect with a stale token: a failed attempt is retried
 * after a cooldown, with a fresh one. Without `MOSQUITTO_URL` the publisher is off;
 * without `MQTT_CLIENT_SECRET` it connects anonymously (dev broker only).
 */
import { connect, type MqttClient } from 'mqtt';

import { env } from '../config/env.js';

const CONNECT_TIMEOUT_MS = 3000;
const PUBLISH_TIMEOUT_MS = 2000;
const RECONNECT_COOLDOWN_MS = 5000;

let client: MqttClient | null = null;
let connecting: Promise<MqttClient | null> | null = null;
let lastFailureAt = 0;
let tokenCache: { value: string; expiresAt: number } | null = null;

/** Whether a broker URL is configured. */
export function mqttConfigured(): boolean {
  return Boolean(env.mqtt.url);
}

/** Fetches (and caches) a service-account access token for the MQTT connection. */
async function getToken(): Promise<string> {
  const now = Date.now();
  if (tokenCache && tokenCache.expiresAt > now) return tokenCache.value;

  const res = await fetch(env.mqtt.tokenUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'client_credentials',
      client_id: env.mqtt.clientId,
      client_secret: env.mqtt.clientSecret,
    }),
  });
  if (!res.ok) {
    throw new Error(`mqtt token request failed (${res.status})`);
  }
  const json = (await res.json()) as { access_token: string; expires_in?: number };
  const ttl = Math.max(30, (json.expires_in ?? 60) - 30);
  tokenCache = { value: json.access_token, expiresAt: now + ttl * 1000 };
  return tokenCache.value;
}

function destroyClient(): void {
  if (!client) return;
  const old = client;
  client = null;
  old.removeAllListeners();
  old.end(true);
}

/** Returns a connected client, (re)connecting when needed; null when unavailable. */
async function ensureClient(): Promise<MqttClient | null> {
  if (!mqttConfigured()) return null;
  if (client?.connected) return client;
  if (connecting) return connecting;
  if (Date.now() - lastFailureAt < RECONNECT_COOLDOWN_MS) return null;

  connecting = (async () => {
    try {
      const password = env.mqtt.clientSecret ? await getToken() : undefined;
      destroyClient();
      const c = connect(env.mqtt.url, {
        username: env.mqtt.clientId,
        password,
        clean: true,
        reconnectPeriod: 0,
        connectTimeout: CONNECT_TIMEOUT_MS,
      });
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('connect timeout')), CONNECT_TIMEOUT_MS);
        c.once('connect', () => {
          clearTimeout(timer);
          resolve();
        });
        c.once('error', (err) => {
          clearTimeout(timer);
          reject(err);
        });
      });
      c.on('error', (err) => console.error(`[mqtt] ${err.message}`));
      c.on('close', () => console.warn('[mqtt] connection closed (next publish reconnects)'));
      client = c;
      return c;
    } catch (err) {
      lastFailureAt = Date.now();
      console.error(`[mqtt] connect failed: ${(err as Error).message}`);
      destroyClient();
      return null;
    } finally {
      connecting = null;
    }
  })();
  return connecting;
}

/**
 * Publishes one notification on `{MQTT_TOPIC_ROOT}/{playerId}` (QoS 0, not retained).
 * @param playerId - Target player.
 * @param payload - Serialized JSON envelope.
 * @returns True when the broker acknowledged the write.
 */
export async function publishNotification(playerId: string, payload: string): Promise<boolean> {
  const c = await ensureClient();
  if (!c?.connected) return false;

  return new Promise<boolean>((resolve) => {
    let done = false;
    const timer = setTimeout(() => {
      if (done) return;
      done = true;
      console.warn('[mqtt] publish timed out');
      resolve(false);
    }, PUBLISH_TIMEOUT_MS);
    c.publish(`${env.mqtt.topicRoot}/${playerId}`, payload, { qos: 0, retain: false }, (err) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (err) {
        console.error(`[mqtt] publish failed: ${err.message}`);
        resolve(false);
      } else {
        resolve(true);
      }
    });
  });
}
