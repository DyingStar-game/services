/** A cargo depot's warehouse POI is found by the depot's uuid (its `metadata.persistence`). */
import { describe, expect, it, vi } from 'vitest';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

const { db } = await import('../src/db/connection.js');
const { inventoryPois } = await import('../src/db/schema/index.js');
const { getPoiByPersistence } = await import('../src/services/pois.service.js');

const DEPOT = '584cc3df-3052-5c9b-993d-f4d2c88b790b';
const ARES = 'bcd6a0ad-d270-4744-96b3-813789916cee';

describe('getPoiByPersistence', () => {
  it('finds the warehouse POI of a depot, and nothing for another uuid', async () => {
    await db.insert(inventoryPois).values({
      ownerType: 'political',
      ownerId: ARES,
      name: 'F0001 — CargoDepot_2',
      scene: 'tarsis_3',
      x: 1,
      y: 2,
      z: 3,
      radiusM: 40,
      metadata: { persistence: DEPOT },
    } as never);
    const poi = await getPoiByPersistence(DEPOT);
    expect(poi?.ownerType).toBe('political');
    expect(poi?.ownerId).toBe(ARES);
    expect(await getPoiByPersistence('00000000-0000-4000-8000-000000000000')).toBeNull();
  });
});
