/**
 * Points of interest: geometry-bearing, shareable markers owned by a holder.
 *
 * Ownership model (wider than goods: political entities own POIs too):
 * - the owner has full control (edit, delete, re-share, transfer ownership);
 * - grants are **read-only** and go to a player/NPC/corporation/political entity;
 * - `visibility: 'public'` makes a POI readable by every player without a grant;
 * - organization-owned POIs are managed by members holding `inventory:poi:manage`
 *   (decided by Social: CEO/head, or `manage_corporation` / `manage_entity`).
 *
 * `system` POIs are only manageable through the internal API (game server).
 */
import { and, desc, eq, inArray } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  inventoryPois,
  inventoryPoiShares,
  type InventoryPoi,
  type InventoryPoiShare,
  type PoiGranteeType,
  type PoiOwnerType,
  type PoiVisibility,
} from '../db/schema/index.js';
import { t } from '../i18n/index.js';
import { HttpError, notFound } from '../lib/httpError.js';
import { authorizeAction, isSocialConfigured } from './social.client.js';

/** POI plus its shares (returned by detail endpoints). */
export interface PoiView extends InventoryPoi {
  shares: InventoryPoiShare[];
}

/** Geometry and metadata accepted when creating a POI. */
export interface PoiInput {
  name: string;
  description?: string | null;
  system?: string | null;
  scene?: string | null;
  parentId?: string | null;
  x: number;
  y: number;
  z: number;
  radiusM?: number | null;
  visibility?: PoiVisibility;
  metadata?: Record<string, unknown> | null;
}

/** Mutable POI fields (ownership changes go through `transferPoi`). */
export interface PoiPatch {
  name?: string;
  description?: string | null;
  system?: string | null;
  scene?: string | null;
  parentId?: string | null;
  x?: number;
  y?: number;
  z?: number;
  radiusM?: number | null;
  visibility?: PoiVisibility;
  metadata?: Record<string, unknown> | null;
}

/** A POI of the player's own scope: owned, granted to them, or public. */
export async function listPlayerPois(playerId: string): Promise<InventoryPoi[]> {
  const [owned, granted, publicPois] = await Promise.all([
    db
      .select()
      .from(inventoryPois)
      .where(and(eq(inventoryPois.ownerType, 'player'), eq(inventoryPois.ownerId, playerId))),
    db
      .select({ poi: inventoryPois })
      .from(inventoryPois)
      .innerJoin(inventoryPoiShares, eq(inventoryPoiShares.poiId, inventoryPois.id))
      .where(
        and(
          eq(inventoryPoiShares.granteeType, 'player'),
          eq(inventoryPoiShares.granteeId, playerId),
        ),
      ),
    db.select().from(inventoryPois).where(eq(inventoryPois.visibility, 'public')),
  ]);

  const merged = new Map<string, InventoryPoi>();
  for (const poi of owned) merged.set(poi.id, poi);
  for (const row of granted) merged.set(row.poi.id, row.poi);
  for (const poi of publicPois) merged.set(poi.id, poi);
  return [...merged.values()].sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime());
}

/**
 * POIs owned by an organization, plus the ones granted to it (read-only).
 * @param ownerType - `corporation` or `political`.
 * @param ownerId - Organization id (opaque, owned by Social).
 * @returns POIs.
 */
export async function listOrgPois(
  ownerType: Extract<PoiOwnerType, 'corporation' | 'political'>,
  ownerId: string,
): Promise<InventoryPoi[]> {
  const [owned, granted] = await Promise.all([
    db
      .select()
      .from(inventoryPois)
      .where(and(eq(inventoryPois.ownerType, ownerType), eq(inventoryPois.ownerId, ownerId))),
    db
      .select({ poi: inventoryPois })
      .from(inventoryPois)
      .innerJoin(inventoryPoiShares, eq(inventoryPoiShares.poiId, inventoryPois.id))
      .where(
        and(eq(inventoryPoiShares.granteeType, ownerType), eq(inventoryPoiShares.granteeId, ownerId)),
      ),
  ]);

  const merged = new Map<string, InventoryPoi>();
  for (const poi of owned) merged.set(poi.id, poi);
  for (const row of granted) merged.set(row.poi.id, row.poi);
  return [...merged.values()].sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime());
}

/** One POI by id, or null. */
export async function getPoi(poiId: string): Promise<InventoryPoi | null> {
  const rows = await db.select().from(inventoryPois).where(eq(inventoryPois.id, poiId)).limit(1);
  return rows[0] ?? null;
}

/** One POI with its shares, or null. */
export async function getPoiView(poiId: string): Promise<PoiView | null> {
  const poi = await getPoi(poiId);
  if (!poi) return null;
  const shares = await db
    .select()
    .from(inventoryPoiShares)
    .where(eq(inventoryPoiShares.poiId, poiId))
    .orderBy(inventoryPoiShares.createdAt);
  return { ...poi, shares };
}

/**
 * Fetches several POIs by id (batch geometry resolution for the mission service).
 * Unknown ids are simply omitted from the result.
 * @param ids - POI ids (at most 100, enforced by the route schema).
 * @returns The found POIs.
 */
export async function resolvePois(ids: string[]): Promise<InventoryPoi[]> {
  if (ids.length === 0) return [];
  return db.select().from(inventoryPois).where(inArray(inventoryPois.id, ids));
}

/** POIs held by any holder (internal/game-server view). */
export async function listHolderPois(
  ownerType: PoiOwnerType,
  ownerId: string,
): Promise<InventoryPoi[]> {
  return db
    .select()
    .from(inventoryPois)
    .where(and(eq(inventoryPois.ownerType, ownerType), eq(inventoryPois.ownerId, ownerId)))
    .orderBy(desc(inventoryPois.createdAt));
}

/** Creates a POI for an owner. */
export async function createPoi(
  owner: { ownerType: PoiOwnerType; ownerId: string },
  input: PoiInput,
): Promise<InventoryPoi> {
  const [poi] = await db
    .insert(inventoryPois)
    .values({
      ownerType: owner.ownerType,
      ownerId: owner.ownerId,
      name: input.name,
      description: input.description ?? null,
      system: input.system ?? null,
      scene: input.scene ?? null,
      parentId: input.parentId ?? null,
      x: input.x,
      y: input.y,
      z: input.z,
      radiusM: input.radiusM ?? null,
      visibility: input.visibility ?? 'private',
      metadata: input.metadata ?? null,
    })
    .returning();
  return poi;
}

/** Applies a partial update to a POI (ownership untouched). */
export async function updatePoi(poiId: string, patch: PoiPatch): Promise<InventoryPoi> {
  const set: Partial<typeof inventoryPois.$inferInsert> = { updatedAt: new Date() };
  if (patch.name !== undefined) set.name = patch.name;
  if (patch.description !== undefined) set.description = patch.description;
  if (patch.system !== undefined) set.system = patch.system;
  if (patch.scene !== undefined) set.scene = patch.scene;
  if (patch.parentId !== undefined) set.parentId = patch.parentId;
  if (patch.x !== undefined) set.x = patch.x;
  if (patch.y !== undefined) set.y = patch.y;
  if (patch.z !== undefined) set.z = patch.z;
  if (patch.radiusM !== undefined) set.radiusM = patch.radiusM;
  if (patch.visibility !== undefined) set.visibility = patch.visibility;
  if (patch.metadata !== undefined) set.metadata = patch.metadata;

  const rows = await db.update(inventoryPois).set(set).where(eq(inventoryPois.id, poiId)).returning();
  if (!rows[0]) throw notFound(t('not_found.poi', { id: poiId }));
  return rows[0];
}

/** Deletes a POI (its shares cascade). */
export async function deletePoi(poiId: string): Promise<void> {
  const rows = await db.delete(inventoryPois).where(eq(inventoryPois.id, poiId)).returning({ id: inventoryPois.id });
  if (!rows[0]) throw notFound(t('not_found.poi', { id: poiId }));
}

/**
 * Grants read-only access to another player/NPC/corporation/political entity.
 * @param poi - POI being shared.
 * @param grantee - Who receives the grant.
 * @param sharedBy - Player creating the grant (audit).
 * @throws 409 `POI_ALREADY_SHARED` when the grant already exists (or the grantee is the owner).
 */
export async function sharePoi(
  poi: InventoryPoi,
  grantee: { granteeType: PoiGranteeType; granteeId: string },
  sharedBy: string,
): Promise<InventoryPoiShare> {
  if (grantee.granteeType === poi.ownerType && grantee.granteeId === poi.ownerId) {
    throw new HttpError(409, 'POI_SELF_SHARE', t('poi.self_share'));
  }
  const [row] = await db
    .insert(inventoryPoiShares)
    .values({ poiId: poi.id, ...grantee, sharedBy })
    .onConflictDoNothing()
    .returning();
  if (!row) throw new HttpError(409, 'POI_ALREADY_SHARED', t('conflict.poi_shared', { id: poi.id }));
  return row;
}

/**
 * Revokes a read-only grant.
 * @throws 404 when there is no such grant.
 */
export async function revokeShare(
  poiId: string,
  grantee: { granteeType: PoiGranteeType; granteeId: string },
): Promise<void> {
  const rows = await db
    .delete(inventoryPoiShares)
    .where(
      and(
        eq(inventoryPoiShares.poiId, poiId),
        eq(inventoryPoiShares.granteeType, grantee.granteeType),
        eq(inventoryPoiShares.granteeId, grantee.granteeId),
      ),
    )
    .returning({ poiId: inventoryPoiShares.poiId });
  if (!rows[0]) throw new HttpError(404, 'POI_NOT_SHARED', t('not_found.poi_share', { id: poiId }));
}

/**
 * Transfers ownership to another holder (player, NPC, corporation or political entity).
 * A grant held by the new owner is dropped (the owner sees their POI anyway); other
 * grants are kept — the new owner can revoke them.
 * @param poi - POI being transferred.
 * @param to - New owner.
 * @returns The updated POI.
 * @throws 409 `POI_TRANSFER_SELF` when the target already owns the POI.
 */
export async function transferPoi(
  poi: InventoryPoi,
  to: { toType: PoiGranteeType; toId: string },
): Promise<InventoryPoi> {
  if (to.toType === poi.ownerType && to.toId === poi.ownerId) {
    throw new HttpError(409, 'POI_TRANSFER_SELF', t('poi.transfer_to_owner'));
  }
  return db.transaction(async (tx) => {
    const [updated] = await tx
      .update(inventoryPois)
      .set({ ownerType: to.toType, ownerId: to.toId, updatedAt: new Date() })
      .where(eq(inventoryPois.id, poi.id))
      .returning();
    await tx
      .delete(inventoryPoiShares)
      .where(
        and(
          eq(inventoryPoiShares.poiId, poi.id),
          eq(inventoryPoiShares.granteeType, to.toType),
          eq(inventoryPoiShares.granteeId, to.toId),
        ),
      );
    return updated;
  });
}

// ── Access control ───────────────────────────────────────────────────────────

/** Requires Social to answer organization checks (local dev may bypass it). */
function requireSocialForOrg(): void {
  if (isSocialConfigured()) return;
  if (env.authDevBypass) return; // Local dev without Social: trust the authenticated player.
  throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
}

/**
 * Requires membership + the `inventory:poi:manage` action on the corporation, decided by
 * Social (CEO passes implicitly, `manage_corporation` satisfies it).
 * @param playerId - Acting player.
 * @param corporationId - Corporation id.
 * @throws 403 `NOT_CORPORATION_MEMBER` / `ORG_PERMISSION_REQUIRED`.
 */
export async function assertCorporationManage(playerId: string, corporationId: string): Promise<void> {
  requireSocialForOrg();
  if (!isSocialConfigured()) return; // dev bypass
  const decision = await authorizeAction({
    holderType: 'corporation',
    holderId: corporationId,
    playerId,
    action: 'inventory:poi:manage',
  });
  if (decision.allowed) return;
  if (decision.reason === 'not_member') {
    throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'You are not a member of this corporation');
  }
  throw new HttpError(403, 'ORG_PERMISSION_REQUIRED', 'Your rank cannot manage this organization');
}

/**
 * Requires membership + the `inventory:poi:manage` action on the political entity, decided
 * by Social (head office passes implicitly, `manage_entity` satisfies it).
 * @param playerId - Acting player.
 * @param entityId - Political entity id.
 * @throws 403 `NOT_POLITICAL_MEMBER` / `ORG_PERMISSION_REQUIRED`.
 */
export async function assertPoliticalManage(playerId: string, entityId: string): Promise<void> {
  requireSocialForOrg();
  if (!isSocialConfigured()) return; // dev bypass
  const decision = await authorizeAction({
    holderType: 'political',
    holderId: entityId,
    playerId,
    action: 'inventory:poi:manage',
  });
  if (decision.allowed) return;
  if (decision.reason === 'not_member') {
    throw new HttpError(403, 'NOT_POLITICAL_MEMBER', 'You are not a member of this political entity');
  }
  throw new HttpError(403, 'ORG_PERMISSION_REQUIRED', 'Your office cannot manage this organization');
}

/**
 * Whether a player may mutate a POI (owner, or member with the org permission).
 * `system`/`npc` POIs are never manageable by a player through this API.
 * @param poi - POI.
 * @param playerId - Acting player.
 * @throws 403 `NOT_POI_OWNER` / `POI_SYSTEM_OWNER_FORBIDDEN` / org errors, 503 without Social.
 */
export async function assertPoiWriteAccess(poi: InventoryPoi, playerId: string): Promise<void> {
  switch (poi.ownerType) {
    case 'player':
      if (poi.ownerId === playerId) return;
      throw new HttpError(403, 'NOT_POI_OWNER', t('poi.not_owner'));
    case 'npc':
      throw new HttpError(403, 'NOT_POI_OWNER', t('poi.not_owner'));
    case 'system':
      throw new HttpError(403, 'POI_SYSTEM_OWNER_FORBIDDEN', t('poi.system_owner'));
    case 'corporation':
      await assertCorporationManage(playerId, poi.ownerId);
      return;
    case 'political':
      await assertPoliticalManage(playerId, poi.ownerId);
      return;
    default:
      throw new HttpError(403, 'NOT_POI_OWNER', t('poi.not_owner'));
  }
}

/** Whether a grantee (player, NPC, corporation or political entity) has a direct read grant. */
async function hasShare(
  poiId: string,
  granteeType: PoiGranteeType,
  granteeId: string,
): Promise<boolean> {
  const rows = await db
    .select({ poiId: inventoryPoiShares.poiId })
    .from(inventoryPoiShares)
    .where(
      and(
        eq(inventoryPoiShares.poiId, poiId),
        eq(inventoryPoiShares.granteeType, granteeType),
        eq(inventoryPoiShares.granteeId, granteeId),
      ),
    )
    .limit(1);
  return rows.length > 0;
}

/** Whether a player has a direct read-only grant on a POI. */
function hasPlayerShare(poiId: string, playerId: string): Promise<boolean> {
  return hasShare(poiId, 'player', playerId);
}

/**
 * Whether a player may read a POI through the `/api/me/pois` scope: they own it, they
 * hold a grant, it is public, or they manage the organization owning it.
 * @param poi - POI.
 * @param playerId - Acting player.
 * @returns True when readable.
 */
export async function canReadAsPlayer(poi: InventoryPoi, playerId: string): Promise<boolean> {
  if (poi.visibility === 'public') return true;
  if (poi.ownerType === 'player' && poi.ownerId === playerId) return true;
  if (await hasPlayerShare(poi.id, playerId)) return true;
  if (poi.ownerType === 'corporation' || poi.ownerType === 'political') {
    try {
      await assertPoiWriteAccess(poi, playerId);
      return true;
    } catch (err) {
      // Only "no access" errors fall through to false; config errors stay loud.
      if (err instanceof HttpError && err.status === 403) return false;
      throw err;
    }
  }
  return false;
}

/** Requires read access on a POI for a player; 404 when it is not in their scope. */
export async function requirePlayerRead(poi: InventoryPoi, playerId: string): Promise<InventoryPoi> {
  if (await canReadAsPlayer(poi, playerId)) return poi;
  throw notFound(t('not_found.poi', { id: poi.id }));
}

/**
 * Whether an organization reads a POI: it owns it, holds a grant, or it is public.
 * @param poi - POI.
 * @param ownerType - `corporation` or `political`.
 * @param ownerId - Organization id.
 * @returns True when readable.
 */
export async function canReadAsOrg(
  poi: InventoryPoi,
  ownerType: Extract<PoiOwnerType, 'corporation' | 'political'>,
  ownerId: string,
): Promise<boolean> {
  if (poi.visibility === 'public') return true;
  if (poi.ownerType === ownerType && poi.ownerId === ownerId) return true;
  return hasShare(poi.id, ownerType, ownerId);
}

/** Requires organization read access; 404 when the POI is outside the organization's scope. */
export async function requireOrgRead(
  poi: InventoryPoi,
  ownerType: Extract<PoiOwnerType, 'corporation' | 'political'>,
  ownerId: string,
): Promise<InventoryPoi> {
  if (await canReadAsOrg(poi, ownerType, ownerId)) return poi;
  throw notFound(t('not_found.poi', { id: poi.id }));
}

/** Fetches a POI or throws 404. */
export async function requirePoi(poiId: string): Promise<InventoryPoi> {
  const poi = await getPoi(poiId);
  if (!poi) throw notFound(t('not_found.poi', { id: poiId }));
  return poi;
}
