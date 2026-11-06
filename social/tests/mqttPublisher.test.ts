/**
 * The MQTT publisher authenticates the way mosquitto-go-auth's jwt backend (remote mode) reads it:
 * the service token as the USERNAME, a non-empty password. Sent as the password, the broker
 * forwarded the client id instead and refused every connection — no notification ever reached a
 * player.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

const mqtt = vi.hoisted(() => ({ options: [] as Record<string, unknown>[] }));

vi.mock('mqtt', () => ({
  connect: (_url: string, options: Record<string, unknown>) => {
    mqtt.options.push(options);
    const handlers: Record<string, (arg?: unknown) => void> = {};
    const client = {
      connected: true,
      once: (event: string, fn: (arg?: unknown) => void) => {
        handlers[event] = fn;
        if (event === 'connect') setTimeout(() => fn(), 0);
        return client;
      },
      on: () => client,
      removeAllListeners: () => client,
      end: () => client,
      publish: (_topic: string, _payload: string, _opts: unknown, cb: (err?: Error) => void) => cb(),
    };
    return client;
  },
}));

vi.mock('../src/config/env.js', async (importOriginal) => {
  const original = await importOriginal<typeof import('../src/config/env.js')>();
  return {
    env: {
      ...original.env,
      mqtt: { ...original.env.mqtt, url: 'ws://broker:9001', clientId: 'svc-social', clientSecret: 'secret', tokenUrl: 'http://kc/token', topicRoot: 'notify' },
    },
  };
});

afterEach(() => vi.unstubAllGlobals());

describe('MQTT publisher', () => {
  it('sends the service token as the username, as the jwt backend reads it', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ access_token: 'the.jwt.token', expires_in: 300 }))));
    const { publishNotification } = await import('../src/mqtt/publisher.js');
    expect(await publishNotification('player-1', '{}')).toBe(true);
    expect(mqtt.options[0]).toMatchObject({ username: 'the.jwt.token', password: 'jwt' });
  });
});
