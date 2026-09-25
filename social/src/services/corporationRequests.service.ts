/**
 * Joining a corporation: direct join, applications (player → corporation) and invitations (corporation → player).
 */
import { and, desc, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  corporationJoinRequests,
  corporations,
  playerProfiles,
  type Corporation,
  type CorporationJoinRequest,
  type PlayerProfile,
} from '../db/schema/index.js';
import { conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { isBlockedEitherWay } from './blocks.service.js';
import { recordCorporationActivity } from './corporationActivity.service.js';
import {
  addCorporationMember,
  getCorporationMembership,
  hasCorporationPermission,
  requireCorporation,
  requireCorporationMember,
  requireCorporationPermission,
} from './corporations.service.js';
import { requireProfile } from './profiles.service.js';

/** A request as seen from the corporation side. */
export interface CorporationRequestView extends CorporationJoinRequest {
  player: PlayerProfile;
}

/** A request as seen from the player side. */
export interface PlayerCorporationRequestView extends CorporationJoinRequest {
  corporation: Pick<Corporation, 'id' | 'name' | 'ticker' | 'logoUrl'>;
}

async function findRequest(corporationId: string, playerId: string): Promise<CorporationJoinRequest | null> {
  const rows = await db
    .select()
    .from(corporationJoinRequests)
    .where(
      and(
        eq(corporationJoinRequests.corporationId, corporationId),
        eq(corporationJoinRequests.playerId, playerId),
      ),
    )
    .limit(1);
  return rows[0] ?? null;
}

async function requireRequest(id: number): Promise<CorporationJoinRequest> {
  const rows = await db.select().from(corporationJoinRequests).where(eq(corporationJoinRequests.id, id)).limit(1);
  if (!rows[0]) throw notFound(`Request ${id} not found`);
  return rows[0];
}

/**
 * Player asks to join: joins directly when recruitment is `open`, files an application when `apply`.
 * A pending invitation from that corporation is accepted instead.
 * @param corporationId - Corporation id.
 * @param playerId - Player.
 * @param message - Optional application message.
 * @returns `{ joined: true }` or the created application.
 */
export async function requestJoinCorporation(
  corporationId: string,
  playerId: string,
  message?: string,
): Promise<{ joined: true } | { joined: false; request: CorporationJoinRequest }> {
  const corporation = await requireCorporation(corporationId);
  await requireProfile(playerId);
  if (await getCorporationMembership(playerId)) throw conflict('Already a member of a corporation');

  const existing = await findRequest(corporationId, playerId);
  if (existing?.kind === 'invitation') {
    await addCorporationMember(corporationId, playerId, playerId);
    return { joined: true };
  }
  if (existing) throw conflict('Application already pending');

  if (corporation.recruitment === 'closed') throw forbidden('This corporation is not recruiting');
  if (corporation.recruitment === 'open') {
    await addCorporationMember(corporationId, playerId, playerId);
    return { joined: true };
  }
  const [request] = await db
    .insert(corporationJoinRequests)
    .values({ corporationId, playerId, kind: 'application', message: message ?? null })
    .returning();
  await recordActivity(playerId, 'corporation_application_sent', { corporationId });
  return { joined: false, request };
}

/**
 * Invites a player (requires `invite`). A pending application from that player is accepted instead.
 * @param corporationId - Corporation id.
 * @param actorId - Inviting member.
 * @param playerId - Invitee.
 * @returns `{ joined: true }` or the created invitation.
 */
export async function inviteCorporationPlayer(
  corporationId: string,
  actorId: string,
  playerId: string,
): Promise<{ joined: true } | { joined: false; request: CorporationJoinRequest }> {
  await requireCorporationPermission(corporationId, actorId, 'invite');
  await requireProfile(playerId);
  if (await getCorporationMembership(playerId)) throw conflict('Player is already a member of a corporation');
  if (await isBlockedEitherWay(actorId, playerId)) throw conflict('A block exists between these players');

  const existing = await findRequest(corporationId, playerId);
  if (existing?.kind === 'application') {
    await addCorporationMember(corporationId, playerId, actorId);
    return { joined: true };
  }
  if (existing) throw conflict('Invitation already pending');

  const [request] = await db
    .insert(corporationJoinRequests)
    .values({ corporationId, playerId, kind: 'invitation', createdBy: actorId })
    .returning();
  await recordCorporationActivity(corporationId, actorId, 'player_invited', { playerId });
  await recordActivity(playerId, 'corporation_invitation_received', { corporationId, by: actorId });
  return { joined: false, request };
}

/**
 * Pending applications and invitations of a corporation (requires `recruit` or `invite`).
 * @param corporationId - Corporation id.
 * @param actorId - Acting member.
 * @returns Requests with player profiles.
 */
export async function listCorporationRequests(corporationId: string, actorId: string): Promise<CorporationRequestView[]> {
  const { rank } = await requireCorporationMember(corporationId, actorId);
  if (!hasCorporationPermission(rank, 'recruit') && !hasCorporationPermission(rank, 'invite')) {
    throw forbidden('Missing corporation permission: recruit or invite');
  }
  const rows = await db
    .select({ request: corporationJoinRequests, player: playerProfiles })
    .from(corporationJoinRequests)
    .innerJoin(playerProfiles, eq(playerProfiles.playerId, corporationJoinRequests.playerId))
    .where(eq(corporationJoinRequests.corporationId, corporationId))
    .orderBy(desc(corporationJoinRequests.createdAt));
  return rows.map((r) => ({ ...r.request, player: r.player }));
}

/**
 * Corporation-side decision on an application (requires `recruit`), or withdrawal of an invitation (requires `invite`).
 * @param corporationId - Corporation id.
 * @param actorId - Acting member.
 * @param requestId - Request id.
 * @param accept - Accept (applications only) or decline/withdraw.
 */
export async function resolveCorporationRequest(
  corporationId: string,
  actorId: string,
  requestId: number,
  accept: boolean,
): Promise<void> {
  const request = await requireRequest(requestId);
  if (request.corporationId !== corporationId) throw notFound(`Request ${requestId} not found`);
  if (request.kind === 'invitation') {
    if (accept) throw forbidden('Only the invited player can accept an invitation');
    await requireCorporationPermission(corporationId, actorId, 'invite');
    await db.delete(corporationJoinRequests).where(eq(corporationJoinRequests.id, requestId));
    await recordCorporationActivity(corporationId, actorId, 'invitation_withdrawn', { playerId: request.playerId });
    return;
  }
  await requireCorporationPermission(corporationId, actorId, 'recruit');
  if (accept) {
    await addCorporationMember(corporationId, request.playerId, actorId);
    return;
  }
  await db.delete(corporationJoinRequests).where(eq(corporationJoinRequests.id, requestId));
  await recordCorporationActivity(corporationId, actorId, 'application_declined', { playerId: request.playerId });
  await recordActivity(request.playerId, 'corporation_application_declined', { corporationId });
}

/**
 * Pending invitations and applications of a player.
 * @param playerId - Player id.
 * @returns Requests with corporation refs.
 */
export async function listPlayerRequests(playerId: string): Promise<PlayerCorporationRequestView[]> {
  const rows = await db
    .select({
      request: corporationJoinRequests,
      corporation: { id: corporations.id, name: corporations.name, ticker: corporations.ticker, logoUrl: corporations.logoUrl },
    })
    .from(corporationJoinRequests)
    .innerJoin(corporations, eq(corporations.id, corporationJoinRequests.corporationId))
    .where(eq(corporationJoinRequests.playerId, playerId))
    .orderBy(desc(corporationJoinRequests.createdAt));
  return rows.map((r) => ({ ...r.request, corporation: r.corporation }));
}

/**
 * Player-side decision: accept/decline an invitation, or withdraw an application.
 * @param playerId - Player id.
 * @param requestId - Request id.
 * @param accept - Accept (invitations only) or decline/withdraw.
 */
export async function resolvePlayerRequest(playerId: string, requestId: number, accept: boolean): Promise<void> {
  const request = await requireRequest(requestId);
  if (request.playerId !== playerId) throw notFound(`Request ${requestId} not found`);
  if (accept) {
    if (request.kind !== 'invitation') throw forbidden('Only the corporation can accept an application');
    await addCorporationMember(request.corporationId, playerId, request.createdBy ?? playerId);
    return;
  }
  await db.delete(corporationJoinRequests).where(eq(corporationJoinRequests.id, requestId));
  if (request.kind === 'invitation') {
    await recordCorporationActivity(request.corporationId, playerId, 'invitation_declined', { playerId });
  }
  await recordActivity(playerId, request.kind === 'invitation' ? 'corporation_invitation_declined' : 'corporation_application_withdrawn', {
    corporationId: request.corporationId,
  });
}