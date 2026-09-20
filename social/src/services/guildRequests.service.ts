/**
 * Joining a guild: direct join, applications (player → guild) and invitations (guild → player).
 */
import { and, desc, eq } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  guildJoinRequests,
  guilds,
  playerProfiles,
  type Guild,
  type GuildJoinRequest,
  type PlayerProfile,
} from '../db/schema/index.js';
import { conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { isBlockedEitherWay } from './blocks.service.js';
import { recordGuildActivity } from './guildActivity.service.js';
import { addMember, getMembership, hasPermission, requireGuild, requireMember, requirePermission } from './guilds.service.js';
import { requireProfile } from './profiles.service.js';

/** A request as seen from the guild side. */
export interface GuildRequestView extends GuildJoinRequest {
  player: PlayerProfile;
}

/** A request as seen from the player side. */
export interface PlayerRequestView extends GuildJoinRequest {
  guild: Pick<Guild, 'id' | 'name' | 'tag' | 'logoUrl'>;
}

async function findRequest(guildId: string, playerId: string): Promise<GuildJoinRequest | null> {
  const rows = await db
    .select()
    .from(guildJoinRequests)
    .where(and(eq(guildJoinRequests.guildId, guildId), eq(guildJoinRequests.playerId, playerId)))
    .limit(1);
  return rows[0] ?? null;
}

async function requireRequest(id: number): Promise<GuildJoinRequest> {
  const rows = await db.select().from(guildJoinRequests).where(eq(guildJoinRequests.id, id)).limit(1);
  if (!rows[0]) throw notFound(`Request ${id} not found`);
  return rows[0];
}

/**
 * Player asks to join: joins directly when recruitment is `open`, files an application when `apply`.
 * A pending invitation from that guild is accepted instead.
 * @param guildId - Guild id.
 * @param playerId - Player.
 * @param message - Optional application message.
 * @returns `{ joined: true }` or the created application.
 */
export async function requestJoin(
  guildId: string,
  playerId: string,
  message?: string,
): Promise<{ joined: true } | { joined: false; request: GuildJoinRequest }> {
  const guild = await requireGuild(guildId);
  await requireProfile(playerId);
  if (await getMembership(playerId)) throw conflict('Already a member of a guild');

  const existing = await findRequest(guildId, playerId);
  if (existing?.kind === 'invitation') {
    await addMember(guildId, playerId, playerId);
    return { joined: true };
  }
  if (existing) throw conflict('Application already pending');

  if (guild.recruitment === 'closed') throw forbidden('This guild is not recruiting');
  if (guild.recruitment === 'open') {
    await addMember(guildId, playerId, playerId);
    return { joined: true };
  }
  const [request] = await db
    .insert(guildJoinRequests)
    .values({ guildId, playerId, kind: 'application', message: message ?? null })
    .returning();
  await recordActivity(playerId, 'guild_application_sent', { guildId });
  return { joined: false, request };
}

/**
 * Invites a player (requires `invite`). A pending application from that player is accepted instead.
 * @param guildId - Guild id.
 * @param actorId - Inviting member.
 * @param playerId - Invitee.
 * @returns `{ joined: true }` or the created invitation.
 */
export async function invitePlayer(
  guildId: string,
  actorId: string,
  playerId: string,
): Promise<{ joined: true } | { joined: false; request: GuildJoinRequest }> {
  await requirePermission(guildId, actorId, 'invite');
  await requireProfile(playerId);
  if (await getMembership(playerId)) throw conflict('Player is already a member of a guild');
  if (await isBlockedEitherWay(actorId, playerId)) throw conflict('A block exists between these players');

  const existing = await findRequest(guildId, playerId);
  if (existing?.kind === 'application') {
    await addMember(guildId, playerId, actorId);
    return { joined: true };
  }
  if (existing) throw conflict('Invitation already pending');

  const [request] = await db
    .insert(guildJoinRequests)
    .values({ guildId, playerId, kind: 'invitation', createdBy: actorId })
    .returning();
  await recordGuildActivity(guildId, actorId, 'player_invited', { playerId });
  await recordActivity(playerId, 'guild_invitation_received', { guildId, by: actorId });
  return { joined: false, request };
}

/**
 * Pending applications and invitations of a guild (requires `recruit` or `invite`).
 * @param guildId - Guild id.
 * @param actorId - Acting member.
 * @returns Requests with player profiles.
 */
export async function listGuildRequests(guildId: string, actorId: string): Promise<GuildRequestView[]> {
  const { rank } = await requireMember(guildId, actorId);
  if (!hasPermission(rank, 'recruit') && !hasPermission(rank, 'invite')) {
    throw forbidden('Missing guild permission: recruit or invite');
  }
  const rows = await db
    .select({ request: guildJoinRequests, player: playerProfiles })
    .from(guildJoinRequests)
    .innerJoin(playerProfiles, eq(playerProfiles.playerId, guildJoinRequests.playerId))
    .where(eq(guildJoinRequests.guildId, guildId))
    .orderBy(desc(guildJoinRequests.createdAt));
  return rows.map((r) => ({ ...r.request, player: r.player }));
}

/**
 * Guild-side decision on an application (requires `recruit`), or withdrawal of an invitation (requires `invite`).
 * @param guildId - Guild id.
 * @param actorId - Acting member.
 * @param requestId - Request id.
 * @param accept - Accept (applications only) or decline/withdraw.
 */
export async function resolveGuildRequest(
  guildId: string,
  actorId: string,
  requestId: number,
  accept: boolean,
): Promise<void> {
  const request = await requireRequest(requestId);
  if (request.guildId !== guildId) throw notFound(`Request ${requestId} not found`);
  if (request.kind === 'invitation') {
    if (accept) throw forbidden('Only the invited player can accept an invitation');
    await requirePermission(guildId, actorId, 'invite');
    await db.delete(guildJoinRequests).where(eq(guildJoinRequests.id, requestId));
    await recordGuildActivity(guildId, actorId, 'invitation_withdrawn', { playerId: request.playerId });
    return;
  }
  await requirePermission(guildId, actorId, 'recruit');
  if (accept) {
    await addMember(guildId, request.playerId, actorId);
    return;
  }
  await db.delete(guildJoinRequests).where(eq(guildJoinRequests.id, requestId));
  await recordGuildActivity(guildId, actorId, 'application_declined', { playerId: request.playerId });
  await recordActivity(request.playerId, 'guild_application_declined', { guildId });
}

/**
 * Pending invitations and applications of a player.
 * @param playerId - Player id.
 * @returns Requests with guild refs.
 */
export async function listPlayerRequests(playerId: string): Promise<PlayerRequestView[]> {
  const rows = await db
    .select({
      request: guildJoinRequests,
      guild: { id: guilds.id, name: guilds.name, tag: guilds.tag, logoUrl: guilds.logoUrl },
    })
    .from(guildJoinRequests)
    .innerJoin(guilds, eq(guilds.id, guildJoinRequests.guildId))
    .where(eq(guildJoinRequests.playerId, playerId))
    .orderBy(desc(guildJoinRequests.createdAt));
  return rows.map((r) => ({ ...r.request, guild: r.guild }));
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
    if (request.kind !== 'invitation') throw forbidden('Only the guild can accept an application');
    await addMember(request.guildId, playerId, request.createdBy ?? playerId);
    return;
  }
  await db.delete(guildJoinRequests).where(eq(guildJoinRequests.id, requestId));
  if (request.kind === 'invitation') {
    await recordGuildActivity(request.guildId, playerId, 'invitation_declined', { playerId });
  }
  await recordActivity(playerId, request.kind === 'invitation' ? 'guild_invitation_declined' : 'guild_application_withdrawn', {
    guildId: request.guildId,
  });
}
