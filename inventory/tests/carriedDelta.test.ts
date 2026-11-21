/**
 * Carried goods: a player's first crate of a good has no stack yet, and the game server's mining credit
 * (a positive delta, no source) must open one instead of answering 404. Dropping goods never carried
 * still fails. The database is an in-process PGlite migrated from `./drizzle`.
 */
import { describe, expect, it, vi } from 'vitest';

vi.mock('../src/db/connection.js', async () => (await import('./helpers/testDb.js')).createTestDb());

const { carriedDelta, getStack } = await import('../src/services/inventory.service.js');

const player = { holderType: 'player' as const, holderId: '11111111-1111-4111-8111-111111111111' };
const warehouse = { holderType: 'warehouse' as const, holderId: '22222222-2222-4222-8222-222222222222' };

describe('carriedDelta', () => {
  it('opens the stack of a good the player never held (first mined crate)', async () => {
    await carriedDelta(player, 'ore.gold', 66);
    const stack = await getStack(player, 'ore.gold');
    expect(stack?.quantity).toBe(66);
    expect(stack?.carried).toBe(66);
  });

  it('adds to a stack that already exists', async () => {
    await carriedDelta(player, 'ingot.gold', 20);
    await carriedDelta(player, 'ingot.gold', 20);
    expect((await getStack(player, 'ingot.gold'))?.carried).toBe(40);
  });

  it('drops carried goods into a warehouse', async () => {
    await carriedDelta(player, 'ore.iron', 30);
    await carriedDelta(player, 'ore.iron', -30, { to: warehouse });
    expect((await getStack(player, 'ore.iron'))?.carried).toBe(0);
    expect((await getStack(warehouse, 'ore.iron'))?.quantity).toBe(30);
  });

  it('still refuses to drop goods that were never carried', async () => {
    await expect(carriedDelta(player, 'ore.cryptonite', -5, { to: warehouse })).rejects.toMatchObject({ status: 404 });
  });
});
