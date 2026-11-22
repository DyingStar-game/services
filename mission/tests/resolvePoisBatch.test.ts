/**
 * `resolvePois` asks Inventory in calls of at most RESOLVE_BATCH ids. Inventory refuses a call with
 * more (its `resolvePoisBody`), and the listing then hid every zoned mission from everybody: seen in
 * preprod on 2026-11-20, the day the open missions' POIs went past 100.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../src/config/env.js', async (importOriginal) => {
  const original = await importOriginal<typeof import('../src/config/env.js')>();
  return {
    ...original,
    env: {
      ...original.env,
      inventory: { ...original.env.inventory, apiUrl: 'http://inventory.test', internalApiKey: 'test-key', serviceClientSecret: '' },
    },
  };
});

const { resolvePois, RESOLVE_BATCH } = await import('../src/services/inventory.client.js');

/** A distinct uuid per index (hex, 8-4-4-4-12). */
const uuid = (i: number): string => `00000000-0000-4000-8000-${i.toString(16).padStart(12, '0')}`;

describe('resolvePois', () => {
  const asked: string[][] = [];

  beforeEach(() => {
    asked.length = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: { body: string }) => {
        const ids = (JSON.parse(init.body) as { ids: string[] }).ids;
        asked.push(ids);
        // As Inventory does: more than its limit is a refused call, not a partial answer.
        if (ids.length > RESOLVE_BATCH) {
          return new Response(JSON.stringify({ error: 'VALIDATION_ERROR' }), { status: 400 });
        }
        const pois = ids.map((id) => ({ id, system: null, scene: 'tarsis_3', center: { x: 0, y: 0, z: 0 }, radiusM: 1000 }));
        return new Response(JSON.stringify({ pois }), { status: 200 });
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('asks in calls of at most the batch size and resolves them all', async () => {
    const ids = Array.from({ length: RESOLVE_BATCH * 2 + 5 }, (_, i) => uuid(i));
    const resolved = await resolvePois(ids);
    expect(resolved.size).toBe(ids.length);
    expect(asked.map((batch) => batch.length)).toEqual([RESOLVE_BATCH, RESOLVE_BATCH, 5]);
  });

  it('asks nothing again for what it just resolved', async () => {
    const ids = Array.from({ length: 3 }, (_, i) => uuid(10_000 + i));
    await resolvePois(ids);
    await resolvePois(ids);
    expect(asked).toHaveLength(1);
  });
});
