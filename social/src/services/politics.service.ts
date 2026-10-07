/**
 * Political entity lifecycle and membership: creation, public page, hierarchy, offices assignment
 * and head transfer. Modelled on `corporations.service.ts`, but a profile (player **or** NPC) may
 * hold any office — including the head office (mayor / president / head of state).
 */
import { and, asc, count, desc, eq, ilike, inArray, or } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { db } from '../db/connection.js';
import {
  POLITICAL_ENTITY_ORDER,
  POLITICAL_PERMISSIONS,
  playerProfiles,
  politicalEntities,
  politicalMembers,
  politicalOffices,
  type PlayerLocation,
  type PlayerProfile,
  type PoliticalEntity,
  type PoliticalEntityType,
  type PoliticalMember,
  type PoliticalOffice,
  type PoliticalPermission,
  type PresenceStatus,
} from '../db/schema/index.js';
import { HttpError, conflict, forbidden, notFound } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { recordActivity } from './activity.service.js';
import { recordPoliticalActivity } from './politicalActivity.service.js';
import { getPresenceMap } from './presence.service.js';
import { isNpc, requireProfile } from './profiles.service.js';

/** Political entity fields editable by members with `manage_entity`. */
export interface PoliticalEntityPatch {
  name?: string;
  description?: string | null;
  bannerUrl?: string | null;
}

/** Political entity as listed publicly. */
export interface PoliticalEntitySummary extends PoliticalEntity {
  memberCount: number;
}

/** Minimal political entity reference embedded in pages. */
export interface PoliticalEntityRef {
  id: string;
  type: PoliticalEntityType;
  name: string;
}

/** A member with profile, office and presence. */
export interface PoliticalMemberView extends PlayerProfile {
  joinedAt: Date;
  office: PoliticalOffice;
  status: PresenceStatus;
  location: PlayerLocation | null;
}

/** The caller's membership context. */
export interface PoliticalMembership {
  entity: PoliticalEntity;
  member: PoliticalMember;
  office: PoliticalOffice;
}

/** Office seed applied when a political entity is created. */
interface OfficeTemplate {
  name: string;
  priority: number;
  permissions: PoliticalPermission[];
  isHead?: boolean;
  isDefault?: boolean;
}

const PG_UNIQUE_VIOLATION = '23505';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

function rethrowUnique(err: unknown, patch: { name?: string }): never {
  if (isUniqueViolation(err)) {
    throw conflict(t('conflict.politics_name_taken', { name: patch.name ?? '' }));
  }
  throw err;
}

/**
 * Default offices seeded for a new political entity, by level. The head office carries every
 * permission; the default office is granted to newly appointed members.
 * @param type - Political level.
 * @returns Office templates.
 */
function defaultOfficesFor(type: PoliticalEntityType): OfficeTemplate[] {
  const head = (name: string): OfficeTemplate => ({
    name,
    priority: 100,
    permissions: [...POLITICAL_PERMISSIONS],
    isHead: true,
  });
  const fallback = (name: string): OfficeTemplate => ({ name, priority: 0, permissions: [], isDefault: true });
  switch (type) {
    case 'commune':
      return [
        head('Mayor'),
        { name: 'Deputy', priority: 50, permissions: ['manage_members'] },
        { name: 'Councilor', priority: 20, permissions: [] },
        fallback('Citizen'),
      ];
    case 'agglomeration':
      return [
        head('President'),
        { name: 'Vice President', priority: 50, permissions: ['manage_members'] },
        { name: 'Delegate', priority: 20, permissions: [] },
        fallback('Resident'),
      ];
    case 'department':
      return [
        head('President'),
        { name: 'Vice President', priority: 50, permissions: ['manage_members'] },
        { name: 'Departmental Councilor', priority: 20, permissions: [] },
        fallback('Resident'),
      ];
    case 'region':
      return [
        head('President'),
        { name: 'Vice President', priority: 50, permissions: ['manage_members'] },
        { name: 'Regional Councilor', priority: 20, permissions: [] },
        fallback('Resident'),
      ];
    case 'country':
      return [
        head('Head of State'),
        { name: 'Minister', priority: 50, permissions: ['manage_members'] },
        { name: 'Deputy', priority: 20, permissions: [] },
        fallback('Citizen'),
      ];
    case 'federation':
      return [
        head('President'),
        { name: 'Representative', priority: 50, permissions: ['manage_members'] },
        fallback('Citizen'),
      ];
  }
}

/**
 * Whether an office grants a permission (the head office grants all).
 * @param office - Office.
 * @param permission - Permission to check.
 * @returns True if granted.
 */
export function hasPoliticalPermission(office: PoliticalOffice, permission: PoliticalPermission): boolean {
  return office.isHead || office.permissions.includes(permission);
}

/**
 * Fetches a political entity or throws 404.
 * @param entityId - Entity id.
 * @returns Entity.
 */
export async function requirePoliticalEntity(entityId: string): Promise<PoliticalEntity> {
  const rows = await db.select().from(politicalEntities).where(eq(politicalEntities.id, entityId)).limit(1);
  if (!rows[0]) throw notFound(t('not_found.political_entity', { id: entityId }));
  return rows[0];
}

/**
 * Membership context of a profile in a given political entity, or null.
 * @param entityId - Entity id.
 * @param playerId - Profile id.
 * @returns Entity, member row and office.
 */
export async function getPoliticalMembership(
  entityId: string,
  playerId: string,
): Promise<PoliticalMembership | null> {
  const rows = await db
    .select({ entity: politicalEntities, member: politicalMembers, office: politicalOffices })
    .from(politicalMembers)
    .innerJoin(politicalEntities, eq(politicalEntities.id, politicalMembers.entityId))
    .innerJoin(politicalOffices, eq(politicalOffices.id, politicalMembers.officeId))
    .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * Every political membership of a profile.
 * @param playerId - Profile id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Memberships, ordered by entity name.
 */
export async function listPoliticalMemberships(playerId: string, limit: number, offset: number): Promise<Page<PoliticalMembership>> {
  const condition = eq(politicalMembers.playerId, playerId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(politicalMembers).where(condition),
    db
      .select({ entity: politicalEntities, member: politicalMembers, office: politicalOffices })
      .from(politicalMembers)
      .innerJoin(politicalEntities, eq(politicalEntities.id, politicalMembers.entityId))
      .innerJoin(politicalOffices, eq(politicalOffices.id, politicalMembers.officeId))
      .where(condition)
      .orderBy(asc(politicalEntities.name), asc(politicalEntities.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(rows, totalRow[0].total, limit, offset);
}

/**
 * Requires the profile to be a member of the given political entity.
 * @param entityId - Entity id.
 * @param playerId - Profile id.
 * @returns Membership context.
 */
export async function requirePoliticalMember(entityId: string, playerId: string): Promise<PoliticalMembership> {
  await requirePoliticalEntity(entityId);
  const membership = await getPoliticalMembership(entityId, playerId);
  if (!membership) throw forbidden(t('forbidden.not_politics_member'));
  return membership;
}

/**
 * Requires membership plus a permission.
 * @param entityId - Entity id.
 * @param playerId - Profile id.
 * @param permission - Required permission.
 * @returns Membership context.
 */
export async function requirePoliticalPermission(
  entityId: string,
  playerId: string,
  permission: PoliticalPermission,
): Promise<PoliticalMembership> {
  const membership = await requirePoliticalMember(entityId, playerId);
  if (!hasPoliticalPermission(membership.office, permission)) {
    throw forbidden(t('forbidden.political_permission', { permission }));
  }
  return membership;
}

/**
 * Political entity references for several profiles, keyed by profile id.
 * @param playerIds - Profile ids.
 * @returns Map of profile id to entity refs.
 */
export async function getPoliticalRefMap(playerIds: string[]): Promise<Map<string, PoliticalEntityRef[]>> {
  if (playerIds.length === 0) return new Map();
  const rows = await db
    .select({
      playerId: politicalMembers.playerId,
      id: politicalEntities.id,
      type: politicalEntities.type,
      name: politicalEntities.name,
    })
    .from(politicalMembers)
    .innerJoin(politicalEntities, eq(politicalEntities.id, politicalMembers.entityId))
    .where(inArray(politicalMembers.playerId, playerIds))
    .orderBy(asc(politicalEntities.name));
  const map = new Map<string, PoliticalEntityRef[]>();
  for (const r of rows) {
    const list = map.get(r.playerId) ?? [];
    list.push({ id: r.id, type: r.type, name: r.name });
    map.set(r.playerId, list);
  }
  return map;
}

/**
 * Lists political entities by name substring with member counts, optionally filtered by level.
 * @param search - Substring (empty = all).
 * @param type - Optional level filter.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of entity summaries.
 */
export async function listPoliticalEntities(
  search: string,
  type: PoliticalEntityType | undefined,
  limit: number,
  offset: number,
): Promise<Page<PoliticalEntitySummary>> {
  const memberCount = count(politicalMembers.playerId);
  const conditions = [];
  if (search) conditions.push(ilike(politicalEntities.name, `%${search}%`));
  if (type) conditions.push(eq(politicalEntities.type, type));
  const condition = conditions.length ? and(...conditions) : undefined;
  const base = db
    .select({ entity: politicalEntities, memberCount })
    .from(politicalEntities)
    .leftJoin(politicalMembers, eq(politicalMembers.entityId, politicalEntities.id))
    .where(condition)
    .groupBy(politicalEntities.id);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(politicalEntities).where(condition),
    base.orderBy(desc(memberCount), asc(politicalEntities.name), asc(politicalEntities.id)).limit(limit).offset(offset),
  ]);
  return page(
    rows.map((r) => ({ ...r.entity, memberCount: r.memberCount })),
    totalRow[0].total,
    limit,
    offset,
  );
}

/**
 * Creates a political entity with its default offices and the creator (player or NPC) as head.
 * @param headId - Profile holding the head office.
 * @param data - Entity fields.
 * @returns Created entity.
 */
export async function createPoliticalEntity(
  headId: string,
  data: { type: PoliticalEntityType; name: string; description?: string | null; bannerUrl?: string | null },
): Promise<PoliticalEntity> {
  await requireProfile(headId);
  try {
    const entity = await db.transaction(async (tx) => {
      const [entity] = await tx
        .insert(politicalEntities)
        .values({ ...data, headId })
        .returning();
      const offices = await tx
        .insert(politicalOffices)
        .values(defaultOfficesFor(data.type).map((o) => ({ entityId: entity.id, ...o })))
        .returning();
      const headOffice = offices.find((o) => o.isHead)!;
      await tx.insert(politicalMembers).values({ entityId: entity.id, playerId: headId, officeId: headOffice.id });
      await recordPoliticalActivity(entity.id, headId, 'entity_created', undefined, tx);
      return entity;
    });
    await recordActivity(headId, 'political_entity_created', { entityId: entity.id, name: entity.name, type: entity.type });
    return entity;
  } catch (err) {
    rethrowUnique(err, data);
  }
}

/**
 * Minimal political entity reference, or null for a null/unknown id.
 * @param entityId - Entity id or null.
 * @returns Entity ref or null.
 */
export async function getPoliticalEntityRef(entityId: string | null): Promise<PoliticalEntityRef | null> {
  if (!entityId) return null;
  const [row] = await db
    .select({ id: politicalEntities.id, type: politicalEntities.type, name: politicalEntities.name })
    .from(politicalEntities)
    .where(eq(politicalEntities.id, entityId))
    .limit(1);
  return row ?? null;
}

/** Public political entity page: entity, offices, members and its place in the hierarchy. */
export interface PoliticalEntityPage extends PoliticalEntitySummary {
  offices: PoliticalOffice[];
  /** First {@link PAGE_MEMBERS} members; `memberCount` is the full count. */
  members: PoliticalMemberView[];
  /** Higher-level entity, or null when independent. */
  parent: PoliticalEntityRef | null;
  /** First {@link PAGE_CHILDREN} direct lower-level entities. */
  children: PoliticalEntitySummary[];
  /** Total number of direct lower-level entities. */
  childCount: number;
}

/** Members embedded in the public political page. */
export const PAGE_MEMBERS = 50;
/** Children embedded in the public political page. */
export const PAGE_CHILDREN = 20;

/**
 * Public political entity page: entity, member count, offices, members, parent and children.
 * Members and children are truncated to {@link PAGE_MEMBERS} / {@link PAGE_CHILDREN} rows;
 * the full lists live on the paginated sub-routes.
 * @param entityId - Entity id.
 * @returns Entity page payload.
 */
export async function getPoliticalEntityPage(entityId: string): Promise<PoliticalEntityPage> {
  const entity = await requirePoliticalEntity(entityId);
  const [offices, members, parent, children] = await Promise.all([
    listPoliticalOffices(entityId),
    listPoliticalMembers(entityId, PAGE_MEMBERS, 0),
    getPoliticalEntityRef(entity.parentId),
    listPoliticalChildren(entityId, PAGE_CHILDREN, 0),
  ]);
  return {
    ...entity,
    memberCount: members.total,
    offices,
    members: members.items,
    parent,
    children: children.items,
    childCount: children.total,
  };
}

/**
 * Direct lower-level entities (entities whose `parentId` is this id).
 * @param entityId - Parent entity id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of child summaries with member counts.
 */
export async function listPoliticalChildren(entityId: string, limit: number, offset: number): Promise<Page<PoliticalEntitySummary>> {
  const memberCount = count(politicalMembers.playerId);
  const condition = eq(politicalEntities.parentId, entityId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(politicalEntities).where(condition),
    db
      .select({ entity: politicalEntities, memberCount })
      .from(politicalEntities)
      .leftJoin(politicalMembers, eq(politicalMembers.entityId, politicalEntities.id))
      .where(condition)
      .groupBy(politicalEntities.id)
      .orderBy(asc(politicalEntities.name), asc(politicalEntities.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(
    rows.map((r) => ({ ...r.entity, memberCount: r.memberCount })),
    totalRow[0].total,
    limit,
    offset,
  );
}

/**
 * True when setting `parentId` as the parent of `entityId` would create a cycle.
 * @param entityId - Child entity id.
 * @param parentId - Candidate parent id.
 * @returns True when the link would be cyclic.
 */
async function wouldCreateCycle(entityId: string, parentId: string): Promise<boolean> {
  const seen = new Set<string>();
  let current: string | null = parentId;
  while (current) {
    if (current === entityId || seen.has(current)) return true;
    seen.add(current);
    const [row] = await db
      .select({ parentId: politicalEntities.parentId })
      .from(politicalEntities)
      .where(eq(politicalEntities.id, current))
      .limit(1);
    current = row?.parentId ?? null;
  }
  return false;
}

/**
 * Attaches a political entity to a higher-level entity, or detaches it (`parentId = null`).
 * Requires `manage_hierarchy`; the parent must be of a strictly higher level and cycles are refused.
 * @param entityId - Child entity id.
 * @param actorId - Acting member.
 * @param parentId - New parent id, or null to detach.
 * @returns Updated entity.
 */
export async function setPoliticalEntityParent(
  entityId: string,
  actorId: string,
  parentId: string | null,
): Promise<PoliticalEntity> {
  const child = await requirePoliticalEntity(entityId);
  await requirePoliticalPermission(entityId, actorId, 'manage_hierarchy');
  if (parentId) {
    if (parentId === entityId) {
      throw new HttpError(400, 'INVALID_PARENT', t('parent.self_politics'));
    }
    const parent = await requirePoliticalEntity(parentId);
    if (POLITICAL_ENTITY_ORDER[parent.type] <= POLITICAL_ENTITY_ORDER[child.type]) {
      throw new HttpError(400, 'INVALID_PARENT_LEVEL', 'The parent must be of a higher political level');
    }
    if (await wouldCreateCycle(entityId, parentId)) {
      throw new HttpError(409, 'POLITICAL_CYCLE', 'This link would create a cycle in the hierarchy');
    }
  }
  const [updated] = await db
    .update(politicalEntities)
    .set({ parentId, updatedAt: new Date() })
    .where(eq(politicalEntities.id, entityId))
    .returning();
  await recordPoliticalActivity(entityId, actorId, parentId ? 'parent_set' : 'parent_cleared', { parentId });
  return updated;
}

/**
 * Updates political entity fields (requires `manage_entity`).
 * @param entityId - Entity id.
 * @param actorId - Acting member.
 * @param patch - Fields to change.
 * @returns Updated entity.
 */
export async function updatePoliticalEntity(
  entityId: string,
  actorId: string,
  patch: PoliticalEntityPatch,
): Promise<PoliticalEntity> {
  await requirePoliticalPermission(entityId, actorId, 'manage_entity');
  try {
    const [updated] = await db
      .update(politicalEntities)
      .set({ ...patch, updatedAt: new Date() })
      .where(eq(politicalEntities.id, entityId))
      .returning();
    await recordPoliticalActivity(entityId, actorId, 'entity_updated', { fields: Object.keys(patch) });
    return updated;
  } catch (err) {
    rethrowUnique(err, patch);
  }
}

/**
 * Deletes a political entity and everything attached (head only). Children become independent.
 * @param entityId - Entity id.
 * @param actorId - Acting profile (the head).
 */
export async function disbandPoliticalEntity(entityId: string, actorId: string): Promise<void> {
  const entity = await requirePoliticalEntity(entityId);
  if (entity.headId !== actorId) throw forbidden(t('forbidden.head_only_disband'));
  const members = await db
    .select({ playerId: politicalMembers.playerId })
    .from(politicalMembers)
    .where(eq(politicalMembers.entityId, entityId));
  await db.delete(politicalEntities).where(eq(politicalEntities.id, entityId));
  await Promise.all(
    members.map((m) => recordActivity(m.playerId, 'political_entity_disbanded', { entityId, name: entity.name })),
  );
}

/**
 * Offices of a political entity, highest priority first.
 * @param entityId - Entity id.
 * @returns Offices.
 */
export async function listPoliticalOffices(entityId: string): Promise<PoliticalOffice[]> {
  return db
    .select()
    .from(politicalOffices)
    .where(eq(politicalOffices.entityId, entityId))
    .orderBy(desc(politicalOffices.priority), politicalOffices.id);
}

/**
 * Fetches an office of an entity or throws 404.
 * @param entityId - Entity id.
 * @param officeId - Office id.
 * @returns Office.
 */
export async function requirePoliticalOffice(entityId: string, officeId: number): Promise<PoliticalOffice> {
  const [office] = await db
    .select()
    .from(politicalOffices)
    .where(and(eq(politicalOffices.id, officeId), eq(politicalOffices.entityId, entityId)))
    .limit(1);
  if (!office) throw notFound(t('not_found.office', { id: officeId }));
  return office;
}

/**
 * The default office of an entity (granted to newly appointed members).
 * @param entityId - Entity id.
 * @returns Default office.
 */
export async function getDefaultPoliticalOffice(entityId: string): Promise<PoliticalOffice> {
  const [office] = await db
    .select()
    .from(politicalOffices)
    .where(and(eq(politicalOffices.entityId, entityId), eq(politicalOffices.isDefault, true)))
    .limit(1);
  if (!office) throw new HttpError(500, 'INTERNAL_ERROR', 'Political entity has no default office');
  return office;
}

/**
 * Members of a political entity with profile, office and presence (highest office first).
 * @param entityId - Entity id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of members.
 */
export async function listPoliticalMembers(entityId: string, limit: number, offset: number): Promise<Page<PoliticalMemberView>> {
  const condition = eq(politicalMembers.entityId, entityId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(politicalMembers).where(condition),
    db
      .select({ profile: playerProfiles, member: politicalMembers, office: politicalOffices })
      .from(politicalMembers)
      .innerJoin(playerProfiles, eq(playerProfiles.playerId, politicalMembers.playerId))
      .innerJoin(politicalOffices, eq(politicalOffices.id, politicalMembers.officeId))
      .where(condition)
      .orderBy(desc(politicalOffices.priority), politicalMembers.joinedAt, asc(politicalMembers.playerId))
      .limit(limit)
      .offset(offset),
  ]);
  const presence = await getPresenceMap(rows.map((r) => r.profile.playerId));
  return page(
    rows.map((r) => {
      const p = presence.get(r.profile.playerId);
      return { ...r.profile, joinedAt: r.member.joinedAt, office: r.office, status: p?.status ?? 'offline', location: p?.location ?? null };
    }),
    totalRow[0].total,
    limit,
    offset,
  );
}

async function requireMemberRow(
  entityId: string,
  playerId: string,
): Promise<{ member: PoliticalMember; office: PoliticalOffice }> {
  const rows = await db
    .select({ member: politicalMembers, office: politicalOffices })
    .from(politicalMembers)
    .innerJoin(politicalOffices, eq(politicalOffices.id, politicalMembers.officeId))
    .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)))
    .limit(1);
  if (!rows[0]) throw notFound(t('not_found.politics_member', { id: playerId }));
  return rows[0];
}

/**
 * Resolves the office to assign, enforcing that it is below the actor's own office and is not the
 * head office (which only changes through a head transfer). Falls back to the default office.
 * @param entityId - Entity id.
 * @param actorOffice - Acting member's office.
 * @param officeId - Requested office, or undefined for the default.
 * @returns Chosen office.
 */
async function resolveAssignableOffice(
  entityId: string,
  actorOffice: PoliticalOffice,
  officeId?: number,
): Promise<PoliticalOffice> {
  if (officeId === undefined) return getDefaultPoliticalOffice(entityId);
  const office = await requirePoliticalOffice(entityId, officeId);
  if (office.isHead) throw forbidden(t('forbidden.use_head_transfer'));
  if (!actorOffice.isHead && office.priority >= actorOffice.priority) {
    throw forbidden(t('forbidden.office_outrank'));
  }
  return office;
}

/**
 * Appoints a profile to a political entity (requires `manage_members`). The office defaults to
 * the entity's default office and must be below the actor's own office.
 * @param entityId - Entity id.
 * @param actorId - Acting member.
 * @param playerId - Appointee.
 * @param officeId - Optional office.
 * @returns Created membership row.
 */
export async function addPoliticalMember(
  entityId: string,
  actorId: string,
  playerId: string,
  officeId?: number,
): Promise<PoliticalMember> {
  const actor = await requirePoliticalPermission(entityId, actorId, 'manage_members');
  await requireProfile(playerId);
  if (await getPoliticalMembership(entityId, playerId)) {
    throw conflict(t('conflict.profile_politics_member'));
  }
  const office = await resolveAssignableOffice(entityId, actor.office, officeId);
  const member = await db.transaction(async (tx) => {
    const [m] = await tx
      .insert(politicalMembers)
      .values({ entityId, playerId, officeId: office.id })
      .returning();
    await recordPoliticalActivity(entityId, actorId, 'member_appointed', { playerId, office: office.name }, tx);
    return m;
  });
  await recordActivity(playerId, 'political_member_appointed', { entityId, office: office.name });
  return member;
}

/**
 * Adds an NPC to a political entity (game server via the internal API). The NPC can hold any
 * office except the head office (use a head transfer).
 * @param entityId - Entity id.
 * @param playerId - NPC profile id.
 * @param officeId - Optional office (default office when omitted).
 * @returns Created membership row.
 */
export async function addNpcPoliticalMember(
  entityId: string,
  playerId: string,
  officeId?: number,
): Promise<PoliticalMember> {
  await requirePoliticalEntity(entityId);
  await requireProfile(playerId);
  if (!(await isNpc(playerId))) throw new HttpError(400, 'NOT_AN_NPC', t('npc.add_only'));
  if (await getPoliticalMembership(entityId, playerId)) {
    throw conflict(t('conflict.npc_politics_member'));
  }
  let office: PoliticalOffice;
  if (officeId !== undefined) {
    office = await requirePoliticalOffice(entityId, officeId);
    if (office.isHead) throw forbidden(t('forbidden.use_head_transfer'));
  } else {
    office = await getDefaultPoliticalOffice(entityId);
  }
  const member = await db.transaction(async (tx) => {
    const [m] = await tx
      .insert(politicalMembers)
      .values({ entityId, playerId, officeId: office.id })
      .returning();
    await recordPoliticalActivity(entityId, null, 'npc_added', { playerId, office: office.name }, tx);
    return m;
  });
  await recordActivity(playerId, 'political_member_appointed', { entityId, office: office.name });
  return member;
}

/**
 * Removes an NPC from a political entity (game server via the internal API).
 * @param entityId - Entity id.
 * @param playerId - NPC profile id.
 */
export async function removeNpcPoliticalMember(entityId: string, playerId: string): Promise<void> {
  const entity = await requirePoliticalEntity(entityId);
  await requireProfile(playerId);
  if (!(await isNpc(playerId))) throw new HttpError(400, 'NOT_AN_NPC', t('npc.remove_only'));
  if (playerId === entity.headId) throw forbidden(t('forbidden.transfer_head_office_first'));
  await requireMemberRow(entityId, playerId);
  const deleted = await db
    .delete(politicalMembers)
    .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)))
    .returning({ playerId: politicalMembers.playerId });
  if (deleted.length === 0) return;
  await recordPoliticalActivity(entityId, null, 'npc_removed', { playerId });
  await recordActivity(playerId, 'political_member_removed', { entityId });
}

/**
 * Changes a member's office. Actor needs `manage_members`, must outrank both the target and the new office.
 * @param entityId - Entity id.
 * @param actorId - Acting member.
 * @param playerId - Target member.
 * @param officeId - New office id.
 * @returns Updated member row.
 */
export async function setPoliticalMemberOffice(
  entityId: string,
  actorId: string,
  playerId: string,
  officeId: number,
): Promise<PoliticalMember> {
  const actor = await requirePoliticalPermission(entityId, actorId, 'manage_members');
  const target = await requireMemberRow(entityId, playerId);
  if (playerId === actorId) throw forbidden(t('forbidden.own_office'));
  if (!actor.office.isHead && target.office.priority >= actor.office.priority) {
    throw forbidden(t('forbidden.manage_office_outrank'));
  }
  const office = await requirePoliticalOffice(entityId, officeId);
  if (office.isHead) throw forbidden(t('forbidden.use_head_transfer'));
  if (!actor.office.isHead && office.priority >= actor.office.priority) {
    throw forbidden(t('forbidden.office_outrank'));
  }
  const [updated] = await db
    .update(politicalMembers)
    .set({ officeId })
    .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)))
    .returning();
  await recordPoliticalActivity(entityId, actorId, 'office_changed', { playerId, from: target.office.name, to: office.name });
  await recordActivity(playerId, 'political_office_changed', { entityId, office: office.name });
  return updated;
}

/**
 * Removes a member: self-leave, or removal with `manage_members` on a lower-office member.
 * The head cannot leave without transferring leadership.
 * @param entityId - Entity id.
 * @param actorId - Acting profile.
 * @param playerId - Member to remove.
 */
export async function removePoliticalMember(entityId: string, actorId: string, playerId: string): Promise<void> {
  const entity = await requirePoliticalEntity(entityId);
  const target = await requireMemberRow(entityId, playerId);
  if (playerId === entity.headId) throw forbidden(t('forbidden.head_must_transfer'));
  if (playerId !== actorId) {
    const actor = await requirePoliticalPermission(entityId, actorId, 'manage_members');
    if (!actor.office.isHead && target.office.priority >= actor.office.priority) {
      throw forbidden(t('forbidden.remove_office_outrank'));
    }
  }
  await db
    .delete(politicalMembers)
    .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)));
  const removed = playerId !== actorId;
  await recordPoliticalActivity(entityId, actorId, removed ? 'member_removed' : 'member_left', { playerId });
  await recordActivity(playerId, removed ? 'political_member_removed' : 'political_member_left', { entityId });
}

/**
 * Transfers the head office: the new head gets the head office, the former head gets the highest
 * non-head office. The new head must already be a member.
 * @param entityId - Entity id.
 * @param actorId - Current head.
 * @param playerId - New head (must be a member).
 * @returns Updated entity.
 */
export async function transferPoliticalHead(
  entityId: string,
  actorId: string,
  playerId: string,
): Promise<PoliticalEntity> {
  const entity = await requirePoliticalEntity(entityId);
  if (entity.headId !== actorId) throw forbidden(t('forbidden.head_only_transfer'));
  if (playerId === actorId) throw new HttpError(400, 'INVALID_TARGET', t('target.already_head'));
  await requireMemberRow(entityId, playerId);
  const offices = await listPoliticalOffices(entityId);
  const headOffice = offices.find((o) => o.isHead)!;
  const fallback = offices.find((o) => !o.isHead) ?? offices.find((o) => o.isDefault)!;
  const updated = await db.transaction(async (tx) => {
    await tx
      .update(politicalMembers)
      .set({ officeId: headOffice.id })
      .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, playerId)));
    await tx
      .update(politicalMembers)
      .set({ officeId: fallback.id })
      .where(and(eq(politicalMembers.entityId, entityId), eq(politicalMembers.playerId, actorId)));
    const [e] = await tx
      .update(politicalEntities)
      .set({ headId: playerId, updatedAt: new Date() })
      .where(eq(politicalEntities.id, entityId))
      .returning();
    await recordPoliticalActivity(entityId, actorId, 'head_transferred', { to: playerId }, tx);
    return e;
  });
  await recordActivity(playerId, 'political_head_received', { entityId });
  return updated;
}
