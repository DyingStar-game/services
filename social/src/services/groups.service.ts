/**
 * Temporary group lifecycle and membership: creation, members, invitations, ownership
 * succession and disbanding. A player belongs to at most one group at a time.
 */
import { and, asc, count, eq } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { db } from '../db/connection.js';
import {
  groupInvitations,
  groupMembers,
  groups,
  playerProfiles,
  type Group,
  type GroupInvitation,
  type GroupMember,
  type PlayerLocation,
  type PlayerProfile,
  type PresenceStatus,
} from '../db/schema/index.js';
import { HttpError, conflict, forbidden, notFound } from '../lib/httpError.js';
import { page, type Page } from '../lib/pagination.js';
import { recordActivity } from './activity.service.js';
import { isBlockedEitherWay } from './blocks.service.js';
import { getPresenceMap } from './presence.service.js';
import { requireProfile } from './profiles.service.js';

/** Fields editable by the group owner. */
export interface GroupPatch {
  name?: string;
  description?: string | null;
  maxMembers?: number;
}

/** Group as returned to callers, with its current member count. */
export interface GroupSummary extends Group {
  memberCount: number;
}

/** A member with profile and presence. */
export interface GroupMemberView extends PlayerProfile {
  joinedAt: Date;
  isOwner: boolean;
  status: PresenceStatus;
  location: PlayerLocation | null;
}

/** The membership context of a player in a group. */
export interface GroupMembership {
  group: Group;
  member: GroupMember;
}

/** A pending invitation as seen by the invitee. */
export interface PlayerGroupInvitationView extends GroupInvitation {
  group: Pick<Group, 'id' | 'name' | 'description' | 'maxMembers'>;
}

const PG_UNIQUE_VIOLATION = '23505';
const GROUP_MEMBERS_PLAYER_UNIQUE = 'group_members_player_one_group';
const GROUPS_NAME_UNIQUE = 'groups_name_unique';

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === PG_UNIQUE_VIOLATION;
}

/** Name of the PostgreSQL constraint that triggered a unique violation, when available. */
function uniqueConstraintOf(err: unknown): string | undefined {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { constraint?: string } | undefined)?.constraint;
}

/**
 * Fetches a group or throws 404.
 * @param groupId - Group id.
 * @returns Group.
 */
export async function requireGroup(groupId: string): Promise<Group> {
  const rows = await db.select().from(groups).where(eq(groups.id, groupId)).limit(1);
  if (!rows[0]) throw notFound(t('not_found.group', { id: groupId }));
  return rows[0];
}

/**
 * Group summary (with member count), or null when it does not exist.
 * @param groupId - Group id.
 * @returns Summary, or null.
 */
export async function getGroupSummary(groupId: string): Promise<GroupSummary | null> {
  const rows = await db
    .select({ group: groups, memberCount: count(groupMembers.playerId) })
    .from(groups)
    .leftJoin(groupMembers, eq(groupMembers.groupId, groups.id))
    .where(eq(groups.id, groupId))
    .groupBy(groups.id)
    .limit(1);
  const row = rows[0];
  if (!row) return null;
  return { ...row.group, memberCount: Number(row.memberCount) };
}

/**
 * Membership context of a player in a group, or null.
 * @param groupId - Group id.
 * @param playerId - Player id.
 * @returns Membership, or null.
 */
export async function getGroupMembership(groupId: string, playerId: string): Promise<GroupMembership | null> {
  const rows = await db
    .select({ group: groups, member: groupMembers })
    .from(groupMembers)
    .innerJoin(groups, eq(groups.id, groupMembers.groupId))
    .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.playerId, playerId)))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * The (single) group of a player, or null.
 * @param playerId - Player id.
 * @returns Membership, or null.
 */
export async function getPlayerGroup(playerId: string): Promise<GroupMembership | null> {
  const rows = await db
    .select({ group: groups, member: groupMembers })
    .from(groupMembers)
    .innerJoin(groups, eq(groups.id, groupMembers.groupId))
    .where(eq(groupMembers.playerId, playerId))
    .limit(1);
  return rows[0] ?? null;
}

/**
 * Requires the player to be a member of the group.
 * @param groupId - Group id.
 * @param playerId - Player id.
 * @returns Membership context.
 */
export async function requireGroupMembership(groupId: string, playerId: string): Promise<GroupMembership> {
  await requireGroup(groupId);
  const membership = await getGroupMembership(groupId, playerId);
  if (!membership) throw forbidden(t('forbidden.not_group_member'));
  return membership;
}

/**
 * Requires the player to own the group.
 * @param groupId - Group id.
 * @param playerId - Player id.
 * @returns Membership context (owner).
 */
export async function requireGroupOwner(groupId: string, playerId: string): Promise<GroupMembership> {
  const membership = await requireGroupMembership(groupId, playerId);
  if (membership.group.ownerId !== playerId) throw forbidden(t('forbidden.owner_only'));
  return membership;
}

/** Number of members of a group (owner included). */
export async function countGroupMembers(groupId: string): Promise<number> {
  const [row] = await db
    .select({ value: count(groupMembers.playerId) })
    .from(groupMembers)
    .where(eq(groupMembers.groupId, groupId));
  return Number(row?.value ?? 0);
}

/**
 * Creates a group with its owner as first member.
 * @param playerId - Creating player (becomes the owner).
 * @param data - Name, optional description and member cap.
 * @returns The created group.
 * @throws 409 when the player already belongs to a group or the name is taken.
 */
export async function createGroup(
  playerId: string,
  data: { name: string; description?: string | null; maxMembers?: number },
): Promise<Group> {
  await requireProfile(playerId);
  if (await getPlayerGroup(playerId)) {
    throw new HttpError(409, 'ALREADY_IN_GROUP', t('conflict.already_in_group'));
  }

  try {
    return await db.transaction(async (tx) => {
      const [group] = await tx
        .insert(groups)
        .values({
          name: data.name,
          description: data.description ?? null,
          ownerId: playerId,
          maxMembers: data.maxMembers ?? 10,
        })
        .returning();
      await tx.insert(groupMembers).values({ groupId: group.id, playerId });
      return group;
    });
  } catch (err) {
    if (isUniqueViolation(err)) {
      if (uniqueConstraintOf(err) === GROUP_MEMBERS_PLAYER_UNIQUE) {
        throw new HttpError(409, 'ALREADY_IN_GROUP', t('conflict.already_in_group'));
      }
      if (uniqueConstraintOf(err) === GROUPS_NAME_UNIQUE || !uniqueConstraintOf(err)) {
        throw conflict(t('conflict.group_name_taken'));
      }
    }
    throw err;
  }
}

/**
 * Updates the editable fields of a group (owner only).
 * @param groupId - Group id.
 * @param actorId - Acting owner.
 * @param patch - Fields to change.
 * @returns Updated group.
 */
export async function updateGroup(groupId: string, actorId: string, patch: GroupPatch): Promise<Group> {
  await requireGroupOwner(groupId, actorId);
  try {
    const [updated] = await db
      .update(groups)
      .set({ ...patch, updatedAt: new Date() })
      .where(eq(groups.id, groupId))
      .returning();
    if (!updated) throw notFound(t('not_found.group', { id: groupId }));
    return updated;
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(t('conflict.group_name_taken'));
    throw err;
  }
}

/**
 * Disbands a group (owner only): members and pending invitations are cascade-deleted.
 * @param groupId - Group id.
 * @param actorId - Acting owner.
 */
export async function disbandGroup(groupId: string, actorId: string): Promise<void> {
  const { group } = await requireGroupOwner(groupId, actorId);
  await db.delete(groups).where(eq(groups.id, groupId));
  await recordActivity(actorId, 'group_disbanded', { groupId, name: group.name });
}

/**
 * Members of a group with profile and presence (owner first, then oldest join).
 * @param groupId - Group id.
 * @param actorId - Acting member.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of members.
 */
export async function listGroupMembers(
  groupId: string,
  actorId: string,
  limit: number,
  offset: number,
): Promise<Page<GroupMemberView>> {
  const { group } = await requireGroupMembership(groupId, actorId);
  const condition = eq(groupMembers.groupId, groupId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(groupMembers).where(condition),
    db
      .select({ profile: playerProfiles, member: groupMembers })
      .from(groupMembers)
      .innerJoin(playerProfiles, eq(playerProfiles.playerId, groupMembers.playerId))
      .where(condition)
      .orderBy(asc(groupMembers.joinedAt), asc(groupMembers.playerId))
      .limit(limit)
      .offset(offset),
  ]);
  const presence = await getPresenceMap(rows.map((r) => r.profile.playerId));
  return page(
    rows.map((r) => {
      const p = presence.get(r.profile.playerId);
      return {
        ...r.profile,
        joinedAt: r.member.joinedAt,
        isOwner: r.profile.playerId === group.ownerId,
        status: p?.status ?? ('offline' as PresenceStatus),
        location: p?.location ?? null,
      };
    }),
    totalRow[0].total,
    limit,
    offset,
  );
}

/**
 * Adds a member to a group (shared by creation and invitation acceptance).
 * @param group - Target group.
 * @param playerId - Joining player.
 * @throws 409 when the player already belongs to a group or the group is full.
 */
export async function addGroupMember(group: Group, playerId: string): Promise<void> {
  try {
    await db.insert(groupMembers).values({ groupId: group.id, playerId });
  } catch (err) {
    if (isUniqueViolation(err)) {
      if (uniqueConstraintOf(err) === GROUP_MEMBERS_PLAYER_UNIQUE) {
        throw new HttpError(409, 'ALREADY_IN_GROUP', t('conflict.already_in_group'));
      }
      throw conflict(t('conflict.group_member'));
    }
    throw err;
  }
}

/**
 * Removes a member from their group (self-leave). When the owner leaves, ownership passes
 * to the oldest remaining member; an empty group is deleted.
 * @param groupId - Group id.
 * @param playerId - Leaving player.
 */
export async function leaveGroup(groupId: string, playerId: string): Promise<void> {
  const membership = await requireGroupMembership(groupId, playerId);
  await db.transaction(async (tx) => {
    await tx
      .delete(groupMembers)
      .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.playerId, playerId)));
    if (membership.group.ownerId === playerId) {
      const [next] = await tx
        .select()
        .from(groupMembers)
        .where(eq(groupMembers.groupId, groupId))
        .orderBy(asc(groupMembers.joinedAt))
        .limit(1);
      if (!next) {
        await tx.delete(groups).where(eq(groups.id, groupId));
        return;
      }
      await tx
        .update(groups)
        .set({ ownerId: next.playerId, updatedAt: new Date() })
        .where(eq(groups.id, groupId));
    }
  });
  await recordActivity(playerId, 'group_left', { groupId });
}

/**
 * Removes another member from a group (owner only; the owner is removed via their own leave).
 * @param groupId - Group id.
 * @param actorId - Acting owner.
 * @param targetId - Member to remove.
 */
export async function removeGroupMember(groupId: string, actorId: string, targetId: string): Promise<void> {
  const { group } = await requireGroupOwner(groupId, actorId);
  if (targetId === group.ownerId) throw forbidden(t('forbidden.owner_cannot_kicked'));
  if (targetId === actorId) throw forbidden(t('forbidden.use_leave'));
  const membership = await getGroupMembership(groupId, targetId);
  if (!membership) throw notFound(t('not_found.group_member', { id: targetId }));
  await db
    .delete(groupMembers)
    .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.playerId, targetId)));
}

/**
 * Invites a player to a group (owner only).
 * @param groupId - Group id.
 * @param actorId - Acting owner.
 * @param playerId - Invitee.
 * @returns The created invitation.
 * @throws 409 when the invitee already belongs to a group, is already invited or the group is full.
 */
export async function inviteGroupPlayer(groupId: string, actorId: string, playerId: string): Promise<GroupInvitation> {
  const { group } = await requireGroupOwner(groupId, actorId);
  await requireProfile(playerId);
  if (playerId === actorId) throw conflict(t('conflict.invite_self'));
  if (await getPlayerGroup(playerId)) {
    throw new HttpError(409, 'ALREADY_IN_GROUP', t('conflict.player_in_group'));
  }
  if (await isBlockedEitherWay(actorId, playerId)) throw conflict(t('conflict.block_exists'));
  if ((await countGroupMembers(groupId)) >= group.maxMembers) {
    throw new HttpError(409, 'GROUP_FULL', 'This group has no free slot');
  }

  const [existing] = await db
    .select()
    .from(groupInvitations)
    .where(and(eq(groupInvitations.groupId, groupId), eq(groupInvitations.playerId, playerId)))
    .limit(1);
  if (existing) throw conflict(t('conflict.invitation_pending'));

  try {
    const [invitation] = await db
      .insert(groupInvitations)
      .values({ groupId, playerId, invitedBy: actorId })
      .returning();
    await recordActivity(playerId, 'group_invitation_received', { groupId, by: actorId });
    return invitation;
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(t('conflict.invitation_pending'));
    throw err;
  }
}

/**
 * Pending group invitations of a player.
 * @param playerId - Player id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of invitations with group refs.
 */
export async function listPlayerGroupInvitations(
  playerId: string,
  limit: number,
  offset: number,
): Promise<Page<PlayerGroupInvitationView>> {
  const condition = eq(groupInvitations.playerId, playerId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(groupInvitations).where(condition),
    db
      .select({
        invitation: groupInvitations,
        group: {
          id: groups.id,
          name: groups.name,
          description: groups.description,
          maxMembers: groups.maxMembers,
        },
      })
      .from(groupInvitations)
      .innerJoin(groups, eq(groups.id, groupInvitations.groupId))
      .where(condition)
      .orderBy(asc(groupInvitations.createdAt), asc(groupInvitations.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(rows.map((r) => ({ ...r.invitation, group: r.group })), totalRow[0].total, limit, offset);
}

/**
 * Player-side decision on a group invitation: accept (joins) or decline.
 * @param playerId - Invitee.
 * @param invitationId - Invitation id.
 * @param accept - Accept or decline.
 * @throws 409 when the player already belongs to a group or the group is full.
 */
export async function resolveGroupInvitation(
  playerId: string,
  invitationId: number,
  accept: boolean,
): Promise<void> {
  const [invitation] = await db
    .select()
    .from(groupInvitations)
    .where(eq(groupInvitations.id, invitationId))
    .limit(1);
  if (!invitation || invitation.playerId !== playerId) throw notFound(t('not_found.invitation', { id: invitationId }));

  if (!accept) {
    await db.delete(groupInvitations).where(eq(groupInvitations.id, invitationId));
    return;
  }

  // The group may have been disbanded (the invitation is cascade-deleted with it).
  const [group] = await db.select().from(groups).where(eq(groups.id, invitation.groupId)).limit(1);
  if (!group) {
    await db.delete(groupInvitations).where(eq(groupInvitations.id, invitationId));
    throw notFound(t('not_found.group', { id: invitation.groupId }));
  }
  if (await getPlayerGroup(playerId)) {
    throw new HttpError(409, 'ALREADY_IN_GROUP', t('conflict.already_in_group'));
  }
  if ((await countGroupMembers(group.id)) >= group.maxMembers) {
    throw new HttpError(409, 'GROUP_FULL', 'This group has no free slot');
  }

  await addGroupMember(group, playerId);
  await db.delete(groupInvitations).where(eq(groupInvitations.id, invitationId));
  await recordActivity(playerId, 'group_joined', { groupId: group.id });
}
