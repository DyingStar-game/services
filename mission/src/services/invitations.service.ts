/**
 * Sharing a mission already taken (team decision, 2026-11-06): a participant invites their group
 * (every member may join) or one player (who accepts or declines). Joining opens an assignment of
 * the SAME mission, past its slot count and its availability zone (the invitation is the gate):
 * progress is shared, the reward split equally between the participants (splitReward), and any
 * participant may turn it in. Each participant deposits the others' crates (the game checks they
 * share a mission).
 */
import { and, desc, eq, inArray, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { missionInvitations, type MissionInvitation } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { getAssignment, openAssignment } from './assignments.service.js';
import { getMissionById, requireMission, requireOpenMission } from './missions.service.js';
import { notifyMissionEvent } from './notifications.service.js';
import { getGroup, getPlayerGroup, isGroupMember } from './social.client.js';

/** Who to invite: one player, or the inviter's own group. */
export type InviteTarget = { playerId: string } | { groupId: string };

/** [fromPlayerId], a participant of [missionId], invites [target] to join it. */
export async function inviteToMission(
  missionId: string,
  fromPlayerId: string,
  target: InviteTarget,
): Promise<MissionInvitation> {
  const mission = await requireMission(missionId);
  requireOpenMission(mission);
  const own = await getAssignment(missionId, fromPlayerId);
  if (!own || own.status !== 'active') {
    throw new HttpError(403, 'NOT_A_PARTICIPANT', 'Only a participant of the mission can share it');
  }
  const toPlayerId = 'playerId' in target ? target.playerId : null;
  const groupId = 'groupId' in target ? target.groupId : null;
  if (toPlayerId === fromPlayerId) throw new HttpError(400, 'SELF_INVITE', 'You already take part in it');
  if (groupId && !(await isGroupMember(fromPlayerId, groupId))) {
    throw new HttpError(403, 'NOT_GROUP_MEMBER', 'You can only share with your own group');
  }
  const [invitation] = await db
    .insert(missionInvitations)
    .values({ missionId, fromPlayerId, toPlayerId, groupId })
    .returning();
  const notification = { type: 'mission.invited' as const, data: { missionId, invitationId: invitation.id, title: mission.title, fromPlayerId } };
  if (toPlayerId) {
    await notifyMissionEvent(toPlayerId, notification);
  } else if (groupId) {
    const group = await getGroup(groupId).catch(() => null);
    const members = Array.isArray(group?.members) ? (group.members as { playerId?: string }[]) : [];
    await Promise.all(
      members
        .map((m) => m.playerId ?? '')
        .filter((id) => id !== '' && id !== fromPlayerId)
        .map((id) => notifyMissionEvent(id, notification)),
    );
  }
  return invitation;
}

/** The pending invitations [playerId] may answer: theirs, and their group's, newest first. */
export async function listInvitationsFor(playerId: string): Promise<MissionInvitation[]> {
  const membership = await getPlayerGroup(playerId).catch(() => null);
  const addressed = membership
    ? or(eq(missionInvitations.toPlayerId, playerId), eq(missionInvitations.groupId, membership.group.id))
    : eq(missionInvitations.toPlayerId, playerId);
  const rows = await db
    .select()
    .from(missionInvitations)
    .where(and(eq(missionInvitations.status, 'pending'), addressed))
    .orderBy(desc(missionInvitations.createdAt))
    .limit(50);
  // A mission one already takes part in, or that closed meanwhile, is no invitation any more.
  const out: MissionInvitation[] = [];
  for (const row of rows) {
    const mine = await getAssignment(row.missionId, playerId);
    const mission = await getMissionById(row.missionId);
    if ((!mine || mine.status !== 'active') && mission && (mission.status === 'available' || mission.status === 'active')) {
      out.push(row);
    }
  }
  return out;
}

/** [playerId] joins the mission of [invitationId]. */
export async function joinMission(invitationId: string, playerId: string) {
  const invitation = await requireInvitationFor(invitationId, playerId);
  const mission = await requireMission(invitation.missionId);
  requireOpenMission(mission);
  const existing = await getAssignment(mission.id, playerId);
  if (existing?.status === 'active') throw new HttpError(409, 'ALREADY_ASSIGNED', 'You already take part in it');
  if (existing?.status === 'completed') throw new HttpError(409, 'ALREADY_COMPLETED', 'You already completed it');
  const opened = await openAssignment(mission, playerId, 'player', existing);
  if (invitation.toPlayerId) await setStatus(invitation.id, 'accepted');
  return opened;
}

/** [playerId] declines [invitationId] (a group's: only for themselves, it stays open to the others). */
export async function declineInvitation(invitationId: string, playerId: string): Promise<void> {
  const invitation = await requireInvitationFor(invitationId, playerId);
  if (invitation.toPlayerId) await setStatus(invitation.id, 'declined');
}

async function requireInvitationFor(invitationId: string, playerId: string): Promise<MissionInvitation> {
  const [invitation] = await db.select().from(missionInvitations).where(eq(missionInvitations.id, invitationId));
  if (!invitation || invitation.status !== 'pending') throw new HttpError(404, 'INVITATION_NOT_FOUND', 'No such invitation');
  const addressed =
    invitation.toPlayerId === playerId || (invitation.groupId !== null && (await isGroupMember(playerId, invitation.groupId)));
  if (!addressed) throw new HttpError(403, 'NOT_INVITED', 'This invitation is not addressed to you');
  return invitation;
}

async function setStatus(id: string, status: MissionInvitation['status']): Promise<void> {
  await db.update(missionInvitations).set({ status, updatedAt: new Date() }).where(eq(missionInvitations.id, id));
}

/** Pending invitations of [missionIds] (for a mission's detail). */
export function listMissionInvitations(missionIds: string[]): Promise<MissionInvitation[]> {
  if (missionIds.length === 0) return Promise.resolve([]);
  return db
    .select()
    .from(missionInvitations)
    .where(and(inArray(missionInvitations.missionId, missionIds), eq(missionInvitations.status, 'pending')));
}
