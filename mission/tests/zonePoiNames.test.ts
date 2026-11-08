/**
 * Mission zones name their POI for the players: `poiName` beside `poiId`, best-effort.
 */
import { describe, expect, it, vi } from 'vitest';

const inventory = vi.hoisted(() => {
  process.env.INVENTORY_API_URL = 'http://inventory.test';
  process.env.INVENTORY_INTERNAL_API_KEY = 'test-key';
  return { resolvePois: vi.fn() };
});

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());
vi.mock('../src/services/inventory.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/inventory.client.js')>()),
  resolvePois: inventory.resolvePois,
}));

const { withPoiNames } = await import('../src/services/zones.service.js');

const VILLAGE = '11111111-1111-4111-8111-111111111111';
const GONE = '22222222-2222-4222-9222-222222222222';

describe('withPoiNames', () => {
  it('names the POI of each zone it knows, and leaves the others', async () => {
    inventory.resolvePois.mockResolvedValue(new Map([[VILLAGE, { id: VILLAGE, name: 'M0001' }]]));
    const [mission] = await withPoiNames([
      { zones: [{ kind: 'poi' as const, poiId: VILLAGE }, { kind: 'poi' as const, poiId: GONE }, { kind: 'system' as const, system: 'tarsis' }] },
    ]);
    expect(mission.zones).toEqual([
      { kind: 'poi', poiId: VILLAGE, poiName: 'M0001' },
      { kind: 'poi', poiId: GONE },
      { kind: 'system', system: 'tarsis' },
    ]);
  });

  it('an Inventory outage leaves the zones as they are', async () => {
    inventory.resolvePois.mockImplementation(async () => {
      throw new Error('down');
    });
    const items = [{ zones: [{ kind: 'poi' as const, poiId: VILLAGE }] }];
    expect(await withPoiNames(items)).toBe(items);
  });

  it('a mission without zones asks nothing', async () => {
    inventory.resolvePois.mockClear();
    const items = [{ zones: [] }];
    expect(await withPoiNames(items)).toBe(items);
    expect(inventory.resolvePois).not.toHaveBeenCalled();
  });
});
