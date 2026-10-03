/**
 * Market catalog: the game-defined list of tradable good types. The game server pushes
 * entries through the internal API; the market only references `goodType` strings.
 */
import { asc, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { marketCatalog, type GoodKind, type MarketCatalogEntry } from '../db/schema/index.js';
import { HttpError, notFound } from '../lib/httpError.js';

/**
 * Lists catalog entries, optionally only enabled ones.
 * @param enabledOnly - Filter to enabled entries.
 * @returns Catalog entries.
 */
export function listCatalog(enabledOnly = true): Promise<MarketCatalogEntry[]> {
  const query = db.select().from(marketCatalog);
  const filtered = enabledOnly ? query.where(eq(marketCatalog.enabled, true)) : query;
  return filtered.orderBy(asc(marketCatalog.goodType));
}

/** Fetches a catalog entry or throws 404. */
export async function requireCatalogEntry(goodType: string): Promise<MarketCatalogEntry> {
  const [row] = await db.select().from(marketCatalog).where(eq(marketCatalog.goodType, goodType)).limit(1);
  if (!row) throw notFound(`Unknown good type "${goodType}"`);
  return row;
}

/**
 * Upserts a catalog entry (game server push).
 * @param goodType - Good type key.
 * @param patch - Kind, unit, display name and enabled flag.
 * @returns The stored entry.
 */
export async function upsertCatalogEntry(
  goodType: string,
  patch: { kind: GoodKind; unit?: string | null; displayName?: string | null; enabled?: boolean },
): Promise<MarketCatalogEntry> {
  const [row] = await db
    .insert(marketCatalog)
    .values({
      goodType,
      kind: patch.kind,
      unit: patch.unit ?? null,
      displayName: patch.displayName ?? null,
      enabled: patch.enabled ?? true,
    })
    .onConflictDoUpdate({
      target: marketCatalog.goodType,
      set: {
        kind: patch.kind,
        unit: patch.unit ?? null,
        displayName: patch.displayName ?? null,
        enabled: patch.enabled ?? true,
        updatedAt: new Date(),
      },
    })
    .returning();
  return row;
}

/**
 * Enables/disables a catalog entry (admin toggle).
 * @param goodType - Good type key.
 * @param enabled - New flag.
 * @returns Updated entry.
 */
export async function setCatalogEnabled(goodType: string, enabled: boolean): Promise<MarketCatalogEntry> {
  const [row] = await db
    .update(marketCatalog)
    .set({ enabled, updatedAt: new Date() })
    .where(eq(marketCatalog.goodType, goodType))
    .returning();
  if (!row) throw notFound(`Unknown good type "${goodType}"`);
  return row;
}

/** Ensures a good type exists in the catalog before an order/demand references it. */
export async function ensureTradable(goodType: string, kind: GoodKind): Promise<void> {
  const entry = await requireCatalogEntry(goodType);
  if (!entry.enabled) throw new HttpError(403, 'GOOD_DISABLED', `Good type "${goodType}" is not tradable`);
  if (entry.kind !== kind) {
    throw new HttpError(400, 'GOOD_KIND_MISMATCH', `Good type "${goodType}" is a ${entry.kind}, not ${kind}`);
  }
}
