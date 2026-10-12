/**
 * `deliver_items` towards an Inventory `warehouse` holder (id = persistence POI uuid):
 * params validation through the spec gate and the transfer performed at measure time.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const inventory = vi.hoisted(() => ({
  getStack: vi.fn(),
  transferStack: vi.fn(),
}));

vi.mock('../src/services/inventory.client.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/services/inventory.client.js')>()),
  getStack: inventory.getStack,
  transferStack: inventory.transferStack,
}));

const { deliverItemsKind } = await import('../src/kinds/objectives/deliverItems.js');
const { validateMissionSpec } = await import('../src/services/spec.service.js');

const WAREHOUSE = '2e9d4a61-5b3c-4f7a-8e12-6c4b3a2d1e0f';
const PLAYER = '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c';

function spec(to: Record<string, unknown>) {
  return {
    category: 'delivery' as const,
    maxAssignees: 1,
    objectives: [{ type: 'deliver_items', title: 'Livrer', targetQuantity: 10, params: { itemId: 'iron_ore', to } }],
  };
}

beforeEach(() => {
  inventory.getStack.mockReset();
  inventory.transferStack.mockReset().mockResolvedValue({});
});

describe('deliver_items', () => {
  it('accepts a warehouse delivery target', () => {
    const validated = validateMissionSpec(spec({ holderType: 'warehouse', holderId: WAREHOUSE }), { escrowed: false });
    expect(validated.objectives[0].params).toEqual({
      itemId: 'iron_ore',
      to: { holderType: 'warehouse', holderId: WAREHOUSE },
    });
  });

  it('still rejects an unknown holder type', () => {
    expect(() => validateMissionSpec(spec({ holderType: 'planet', holderId: WAREHOUSE }), { escrowed: false })).toThrow(
      expect.objectContaining({ status: 400, code: 'INVALID_OBJECTIVE_PARAMS' }),
    );
  });

  it('advertises warehouse in the params JSON schema', () => {
    const to = (deliverItemsKind.paramsJsonSchema as any).properties.to;
    expect(to.properties.holderType.enum).toContain('warehouse');
  });

  it('transfers the available goods to the warehouse at measure time', async () => {
    inventory.getStack.mockResolvedValue({ goodType: 'iron_ore', quantity: 7, held: 0, available: 7 });

    const progress = await deliverItemsKind.measure!({
      playerId: PLAYER,
      holderType: 'player',
      objective: {
        params: { itemId: 'iron_ore', to: { holderType: 'warehouse', holderId: WAREHOUSE } },
        targetQuantity: 10,
        currentProgress: 0,
      },
    } as never);

    expect(progress).toBe(7);
    expect(inventory.transferStack).toHaveBeenCalledWith(
      { holderType: 'player', holderId: PLAYER },
      { holderType: 'warehouse', holderId: WAREHOUSE },
      'iron_ore',
      7,
    );
  });
});
