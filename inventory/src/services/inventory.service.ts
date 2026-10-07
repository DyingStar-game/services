/**
 * Ownership registry: fungible stacks, unique instances and holds. No physical state is
 * stored here — only who owns what and what is reserved. Transfers are only applied when
 * a trusted caller (game server, market, mission) reports that the exchange happened.
 */
import { and, count, desc, eq, sql } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { db } from '../db/connection.js';
import {
  goodTypes,
  inventoryHolds,
  inventoryInstances,
  inventoryStacks,
  type GoodKind,
  type HolderType,
  type InstanceStatus,
  type InventoryHold,
  type InventoryInstance,
  type InventoryStack,
} from '../db/schema/index.js';
import { HttpError, conflict, notFound } from '../lib/httpError.js';

type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

/** Holder identity. */
export interface Holder {
  holderType: HolderType;
  holderId: string;
}

/** A stack with its reserved and available quantities. */
export interface StackView extends InventoryStack {
  held: number;
  available: number;
}

/**
 * Inventory of a holder. `stacks` is always complete (bounded by the good catalog);
 * `instances` is a page (`instancesTotal` counts them all).
 */
export interface HolderInventory {
  holderType: HolderType;
  holderId: string;
  stacks: StackView[];
  instances: InventoryInstance[];
  instancesTotal: number;
  limit: number;
  offset: number;
}

/**
 * Registers a good type on first use and returns its authoritative kind.
 * @param tx - Transaction.
 * @param goodType - Good type key.
 * @param kind - Expected kind.
 * @returns The stored kind.
 * @throws 409 when the type already exists with a different kind.
 */
async function ensureGoodKind(tx: Tx, goodType: string, kind: GoodKind): Promise<GoodKind> {
  const [created] = await tx
    .insert(goodTypes)
    .values({ goodType, kind })
    .onConflictDoNothing()
    .returning();
  if (created) return created.kind;
  const [existing] = await tx.select().from(goodTypes).where(eq(goodTypes.goodType, goodType)).limit(1);
  if (!existing) throw new HttpError(500, 'INTERNAL_ERROR', t('internal.good_type', { goodType }));
  if (existing.kind !== kind) {
    throw conflict(t('conflict.good_kind', { goodType, registered: existing.kind, expected: kind }));
  }
  return existing.kind;
}

/** Reserves held quantity of a stack for a holder (active holds only). */
async function heldQuantity(tx: Tx, holder: Holder, goodType: string): Promise<number> {
  const [row] = await tx
    .select({ total: sql<string>`coalesce(sum(${inventoryHolds.quantity}), 0)` })
    .from(inventoryHolds)
    .where(
      and(
        eq(inventoryHolds.holderType, holder.holderType),
        eq(inventoryHolds.holderId, holder.holderId),
        eq(inventoryHolds.goodType, goodType),
        eq(inventoryHolds.kind, 'stack'),
        eq(inventoryHolds.status, 'active'),
      ),
    );
  return Number(row?.total ?? 0);
}

/** Whether an instance is reserved by an active hold. */
async function isInstanceHeld(tx: Tx, instanceId: string): Promise<boolean> {
  const [row] = await tx
    .select({ id: inventoryHolds.id })
    .from(inventoryHolds)
    .where(and(eq(inventoryHolds.instanceId, instanceId), eq(inventoryHolds.status, 'active')))
    .limit(1);
  return Boolean(row);
}

/**
 * Inventory of a holder: all stacks (with held/available) plus a page of instances.
 * @param holder - Holder.
 * @param limit - Page size for `instances`.
 * @param offset - Instances to skip.
 * @returns Inventory.
 */
export async function getHolderInventory(holder: Holder, limit: number, offset: number): Promise<HolderInventory> {
  const instanceCondition = and(
    eq(inventoryInstances.holderType, holder.holderType),
    eq(inventoryInstances.holderId, holder.holderId),
  );
  const [stacks, instanceTotal, instances] = await Promise.all([
    db
      .select()
      .from(inventoryStacks)
      .where(and(eq(inventoryStacks.holderType, holder.holderType), eq(inventoryStacks.holderId, holder.holderId)))
      .orderBy(inventoryStacks.goodType),
    db.select({ total: count() }).from(inventoryInstances).where(instanceCondition),
    db
      .select()
      .from(inventoryInstances)
      .where(instanceCondition)
      .orderBy(inventoryInstances.createdAt, inventoryInstances.id)
      .limit(limit)
      .offset(offset),
  ]);

  const views: StackView[] = [];
  for (const stack of stacks) {
    const held = await db
      .select({ total: sql<string>`coalesce(sum(${inventoryHolds.quantity}), 0)` })
      .from(inventoryHolds)
      .where(
        and(
          eq(inventoryHolds.holderType, holder.holderType),
          eq(inventoryHolds.holderId, holder.holderId),
          eq(inventoryHolds.goodType, stack.goodType),
          eq(inventoryHolds.kind, 'stack'),
          eq(inventoryHolds.status, 'active'),
        ),
      );
    const heldCount = Number(held[0]?.total ?? 0);
    views.push({ ...stack, held: heldCount, available: stack.quantity - heldCount });
  }

  return {
    holderType: holder.holderType,
    holderId: holder.holderId,
    stacks: views,
    instances,
    instancesTotal: instanceTotal[0].total,
    limit,
    offset,
  };
}

/**
 * Single stack view for a holder and good type (`null` when the holder owns none).
 * @param holder - Holder.
 * @param goodType - Good type.
 * @returns Stack view or null.
 */
export async function getStack(holder: Holder, goodType: string): Promise<StackView | null> {
  const [stack] = await db
    .select()
    .from(inventoryStacks)
    .where(
      and(
        eq(inventoryStacks.holderType, holder.holderType),
        eq(inventoryStacks.holderId, holder.holderId),
        eq(inventoryStacks.goodType, goodType),
      ),
    )
    .limit(1);
  if (!stack) return null;
  const held = await db
    .select({ total: sql<string>`coalesce(sum(${inventoryHolds.quantity}), 0)` })
    .from(inventoryHolds)
    .where(
      and(
        eq(inventoryHolds.holderType, holder.holderType),
        eq(inventoryHolds.holderId, holder.holderId),
        eq(inventoryHolds.goodType, goodType),
        eq(inventoryHolds.kind, 'stack'),
        eq(inventoryHolds.status, 'active'),
      ),
    );
  const heldCount = Number(held[0]?.total ?? 0);
  return { ...stack, held: heldCount, available: stack.quantity - heldCount };
}

/**
 * Grants (creates or increases) a fungible stack. Used by the game server when goods are
 * produced/mined/awarded.
 * @param holder - Receiving holder.
 * @param goodType - Good type.
 * @param quantity - Quantity to add (positive).
 * @returns Updated stack.
 */
export async function creditStack(holder: Holder, goodType: string, quantity: number): Promise<InventoryStack> {
  if (quantity <= 0) throw new HttpError(400, 'INVALID_QUANTITY', t('quantity.positive'));
  return db.transaction(async (tx) => {
    await ensureGoodKind(tx, goodType, 'stack');
    const [row] = await tx
      .insert(inventoryStacks)
      .values({ ...holder, goodType, quantity })
      .onConflictDoUpdate({
        target: [inventoryStacks.holderType, inventoryStacks.holderId, inventoryStacks.goodType],
        set: { quantity: sql`${inventoryStacks.quantity} + ${quantity}`, updatedAt: new Date() },
      })
      .returning();
    return row;
  });
}

/**
 * Debits an available (non-held) fungible stack. Used for consumption/destruction.
 * @param holder - Owning holder.
 * @param goodType - Good type.
 * @param quantity - Quantity to remove (positive).
 * @returns Updated stack.
 * @throws 409 when the available quantity is insufficient.
 */
export async function debitStack(holder: Holder, goodType: string, quantity: number): Promise<InventoryStack> {
  if (quantity <= 0) throw new HttpError(400, 'INVALID_QUANTITY', t('quantity.positive'));
  return db.transaction(async (tx) => {
    const [stack] = await tx
      .select()
      .from(inventoryStacks)
      .where(
        and(
          eq(inventoryStacks.holderType, holder.holderType),
          eq(inventoryStacks.holderId, holder.holderId),
          eq(inventoryStacks.goodType, goodType),
        ),
      )
      .for('update');
    if (!stack) throw notFound(t('not_found.stack', { id: goodType }));
    const held = await heldQuantity(tx, holder, goodType);
    if (stack.quantity - held < quantity) {
      throw new HttpError(409, 'INSUFFICIENT_GOODS', t('goods.insufficient', { available: stack.quantity - held, goodType }));
    }
    const [row] = await tx
      .update(inventoryStacks)
      .set({ quantity: sql`${inventoryStacks.quantity} - ${quantity}`, updatedAt: new Date() })
      .where(
        and(
          eq(inventoryStacks.holderType, holder.holderType),
          eq(inventoryStacks.holderId, holder.holderId),
          eq(inventoryStacks.goodType, goodType),
        ),
      )
      .returning();
    return row;
  });
}

/**
 * Registers a unique instance owned by a holder. Idempotent on the instance UUID: an
 * existing instance is returned unchanged when it already belongs to that holder.
 * @param holder - Owning holder.
 * @param goodType - Good type.
 * @param instanceId - Instance UUID.
 * @param metadata - Optional free-form metadata.
 * @returns The instance.
 * @throws 409 when the instance exists and belongs to another holder.
 */
export async function registerInstance(
  holder: Holder,
  goodType: string,
  instanceId: string,
  metadata?: Record<string, unknown>,
): Promise<InventoryInstance> {
  return db.transaction(async (tx) => {
    await ensureGoodKind(tx, goodType, 'instance');
    const [existing] = await tx
      .select()
      .from(inventoryInstances)
      .where(eq(inventoryInstances.id, instanceId))
      .limit(1);
    if (existing) {
      if (existing.holderType !== holder.holderType || existing.holderId !== holder.holderId) {
        throw conflict(t('conflict.instance_other', { id: instanceId }));
      }
      return existing;
    }
    const [row] = await tx
      .insert(inventoryInstances)
      .values({ id: instanceId, goodType, ...holder, metadata })
      .returning();
    return row;
  });
}

/**
 * Updates the physical status (`stored` / `in_world`) of an owned instance.
 * @param holder - Owning holder.
 * @param instanceId - Instance UUID.
 * @param status - New status.
 * @returns Updated instance.
 */
export async function setInstanceStatus(
  holder: Holder,
  instanceId: string,
  status: InstanceStatus,
): Promise<InventoryInstance> {
  const [row] = await db
    .update(inventoryInstances)
    .set({ status, updatedAt: new Date() })
    .where(
      and(
        eq(inventoryInstances.id, instanceId),
        eq(inventoryInstances.holderType, holder.holderType),
        eq(inventoryInstances.holderId, holder.holderId),
      ),
    )
    .returning();
  if (!row) throw notFound(t('not_found.instance_holder', { id: instanceId }));
  return row;
}

/**
 * Creates a hold reserving available goods of a holder for a pending operation.
 * @param holder - Holder whose goods are reserved.
 * @param input - Hold parameters (stack quantity or instance id).
 * @returns The created hold.
 * @throws 409 when the available quantity/instance is insufficient or already held.
 */
export async function createHold(
  holder: Holder,
  input: {
    kind: 'stack' | 'instance';
    goodType: string;
    quantity?: number;
    instanceId?: string;
    refType: 'market_order' | 'mission_escrow' | 'manual';
    refId?: string;
    expiresAt?: Date;
    details?: Record<string, unknown>;
  },
): Promise<InventoryHold> {
  if (input.refType !== 'manual' && !input.refId) {
    throw new HttpError(400, 'REF_REQUIRED', 'refId is required for non-manual holds');
  }
  return db.transaction(async (tx) => {
    await ensureGoodKind(tx, input.goodType, input.kind);

    if (input.kind === 'instance') {
      if (!input.instanceId) throw new HttpError(400, 'INSTANCE_REQUIRED', 'instanceId is required for instance holds');
      const [instance] = await tx
        .select()
        .from(inventoryInstances)
        .where(
          and(
            eq(inventoryInstances.id, input.instanceId),
            eq(inventoryInstances.holderType, holder.holderType),
            eq(inventoryInstances.holderId, holder.holderId),
          ),
        )
        .limit(1);
      if (!instance) throw notFound(t('not_found.instance_holder', { id: input.instanceId }));
      if (await isInstanceHeld(tx, input.instanceId)) {
        throw conflict(t('conflict.instance_already_held', { id: input.instanceId }));
      }
      const [row] = await tx
        .insert(inventoryHolds)
        .values({
          ...holder,
          kind: 'instance',
          goodType: input.goodType,
          quantity: 1,
          instanceId: input.instanceId,
          refType: input.refType,
          refId: input.refId,
          expiresAt: input.expiresAt,
          details: input.details,
        })
        .returning();
      return row;
    }

    const quantity = input.quantity ?? 0;
    if (quantity <= 0) throw new HttpError(400, 'INVALID_QUANTITY', t('quantity.stack_positive'));
    const [stack] = await tx
      .select()
      .from(inventoryStacks)
      .where(
        and(
          eq(inventoryStacks.holderType, holder.holderType),
          eq(inventoryStacks.holderId, holder.holderId),
          eq(inventoryStacks.goodType, input.goodType),
        ),
      )
      .for('update');
    if (!stack) throw notFound(t('not_found.stack', { id: input.goodType }));
    const held = await heldQuantity(tx, holder, input.goodType);
    if (stack.quantity - held < quantity) {
      throw new HttpError(409, 'INSUFFICIENT_GOODS', t('goods.insufficient', { available: stack.quantity - held, goodType: input.goodType }));
    }
    const [row] = await tx
      .insert(inventoryHolds)
      .values({
        ...holder,
        kind: 'stack',
        goodType: input.goodType,
        quantity,
        refType: input.refType,
        refId: input.refId,
        expiresAt: input.expiresAt,
        details: input.details,
      })
      .returning();
    return row;
  });
}

/** Fetches a hold or throws 404. */
async function requireHold(tx: Tx, holdId: string): Promise<InventoryHold> {
  const [hold] = await tx.select().from(inventoryHolds).where(eq(inventoryHolds.id, holdId)).limit(1);
  if (!hold) throw notFound(t('not_found.hold', { id: holdId }));
  if (hold.status !== 'active') throw conflict(t('conflict.hold_state', { id: holdId, status: hold.status }));
  return hold;
}

/**
 * Releases an active hold (goods become available again). No ownership changes.
 * @param holdId - Hold id.
 * @returns The released hold.
 */
export async function releaseHold(holdId: string): Promise<InventoryHold> {
  return db.transaction(async (tx) => {
    await requireHold(tx, holdId);
    const [row] = await tx
      .update(inventoryHolds)
      .set({ status: 'released', updatedAt: new Date() })
      .where(eq(inventoryHolds.id, holdId))
      .returning();
    return row;
  });
}

/**
 * Consumes an active hold by transferring its reserved goods to another holder. This is the
 * final step of an exchange: the caller (game server / market) confirms the physical
 * delivery, then ownership moves and the items stop being reserved.
 * @param holdId - Hold id.
 * @param to - Receiving holder.
 * @returns The transferred goods.
 */
export async function consumeHold(
  holdId: string,
  to: Holder,
): Promise<{ hold: InventoryHold; stack?: InventoryStack; instance?: InventoryInstance }> {
  return db.transaction(async (tx) => {
    const hold = await requireHold(tx, holdId);
    const from: Holder = { holderType: hold.holderType, holderId: hold.holderId };

    if (hold.kind === 'instance') {
      const [instance] = await tx
        .update(inventoryInstances)
        .set({ holderType: to.holderType, holderId: to.holderId, updatedAt: new Date() })
        .where(
          and(
            eq(inventoryInstances.id, hold.instanceId!),
            eq(inventoryInstances.holderType, from.holderType),
            eq(inventoryInstances.holderId, from.holderId),
          ),
        )
        .returning();
      if (!instance) throw notFound(t('not_found.instance_hold', { id: hold.instanceId ?? '' }));
      await tx
        .update(inventoryHolds)
        .set({ status: 'consumed', updatedAt: new Date() })
        .where(eq(inventoryHolds.id, holdId));
      return { hold: { ...hold, status: 'consumed' }, instance };
    }

    const [stack] = await tx
      .update(inventoryStacks)
      .set({ quantity: sql`${inventoryStacks.quantity} - ${hold.quantity}`, updatedAt: new Date() })
      .where(
        and(
          eq(inventoryStacks.holderType, from.holderType),
          eq(inventoryStacks.holderId, from.holderId),
          eq(inventoryStacks.goodType, hold.goodType),
          sql`${inventoryStacks.quantity} >= ${hold.quantity}`,
        ),
      )
      .returning();
    if (!stack) throw new HttpError(409, 'INSUFFICIENT_GOODS', t('goods.held_gone'));

    const [toStack] = await tx
      .insert(inventoryStacks)
      .values({ ...to, goodType: hold.goodType, quantity: hold.quantity })
      .onConflictDoUpdate({
        target: [inventoryStacks.holderType, inventoryStacks.holderId, inventoryStacks.goodType],
        set: { quantity: sql`${inventoryStacks.quantity} + ${hold.quantity}`, updatedAt: new Date() },
      })
      .returning();

    await tx.update(inventoryHolds).set({ status: 'consumed', updatedAt: new Date() }).where(eq(inventoryHolds.id, holdId));
    return { hold: { ...hold, status: 'consumed' }, stack: toStack };
  });
}

/**
 * Transfers available (non-held) goods directly between two holders, without a hold.
 * @param from - Source holder.
 * @param to - Destination holder.
 * @param goodType - Good type.
 * @param quantity - Quantity.
 * @returns The destination stack.
 */
export async function transferStack(
  from: Holder,
  to: Holder,
  goodType: string,
  quantity: number,
): Promise<InventoryStack> {
  if (quantity <= 0) throw new HttpError(400, 'INVALID_QUANTITY', t('quantity.positive'));
  return db.transaction(async (tx) => {
    await ensureGoodKind(tx, goodType, 'stack');
    const [stack] = await tx
      .select()
      .from(inventoryStacks)
      .where(
        and(
          eq(inventoryStacks.holderType, from.holderType),
          eq(inventoryStacks.holderId, from.holderId),
          eq(inventoryStacks.goodType, goodType),
        ),
      )
      .for('update');
    if (!stack) throw notFound(t('not_found.stack_source', { id: goodType }));
    const held = await heldQuantity(tx, from, goodType);
    if (stack.quantity - held < quantity) {
      throw new HttpError(409, 'INSUFFICIENT_GOODS', t('goods.insufficient', { available: stack.quantity - held, goodType }));
    }
    await tx
      .update(inventoryStacks)
      .set({ quantity: sql`${inventoryStacks.quantity} - ${quantity}`, updatedAt: new Date() })
      .where(
        and(
          eq(inventoryStacks.holderType, from.holderType),
          eq(inventoryStacks.holderId, from.holderId),
          eq(inventoryStacks.goodType, goodType),
        ),
      );
    const [toStack] = await tx
      .insert(inventoryStacks)
      .values({ ...to, goodType, quantity })
      .onConflictDoUpdate({
        target: [inventoryStacks.holderType, inventoryStacks.holderId, inventoryStacks.goodType],
        set: { quantity: sql`${inventoryStacks.quantity} + ${quantity}`, updatedAt: new Date() },
      })
      .returning();
    return toStack;
  });
}

/**
 * Transfers an owned instance directly to another holder (must not be held).
 * @param from - Source holder.
 * @param to - Destination holder.
 * @param instanceId - Instance UUID.
 * @returns The transferred instance.
 */
export async function transferInstance(from: Holder, to: Holder, instanceId: string): Promise<InventoryInstance> {
  return db.transaction(async (tx) => {
    if (await isInstanceHeld(tx, instanceId)) {
      throw conflict(t('conflict.instance_is_held', { id: instanceId }));
    }
    const [instance] = await tx
      .update(inventoryInstances)
      .set({ holderType: to.holderType, holderId: to.holderId, updatedAt: new Date() })
      .where(
        and(
          eq(inventoryInstances.id, instanceId),
          eq(inventoryInstances.holderType, from.holderType),
          eq(inventoryInstances.holderId, from.holderId),
        ),
      )
      .returning();
    if (!instance) throw notFound(t('not_found.instance_source', { id: instanceId }));
    return instance;
  });
}
