/**
 * Friend requests, friendships, online friends and suggestions.
 */
import { and, desc, eq, inArray, or } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  friendships,
  playerProfiles,
  type Friendship,
  type PlayerLocation,
  type PlayerProfile,
  type PresenceStatus,
} from '../db/schema/index.js';
import { HttpError, conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { blockedIdsFor, isBlockedEitherWay } from './blocks.service.js';
import { listRecentEncounters } from './encounters.service.js';
import { getPresenceMap } from './presence.service.js';
import { requireProfile } from './profiles.service.js';

/** A friend as returned to the client: profile plus presence. */
export interface FriendView extends PlayerProfile {
  friendsSince: Date;
  status: PresenceStatus;
  location: PlayerLocation | null;
}

/** A pending request with the other party's profile. */
export interface FriendRequestView {
  id: number;
  createdAt: Date;
  player: PlayerProfile;
}

/** Suggested player from recent encounters. */
export interface SuggestionView extends PlayerProfile {
  lastMetAt: Date;
  encounters: number;
}

function pairCondition(a: string, b: string) {
  return or(
    and(eq(friendships.requesterId, a), eq(friendships.addresseeId, b)),
    and(eq(friendships.requesterId, b), eq(friendships.addresseeId, a)),
  );
}

/**
 * Finds the friendship row between two players, whichever direction.
 * @param a - Player id.
 * @param b - Player id.
 * @returns Friendship, or null.
 */
export async function findRelation(a: string, b: string): Promise<Friendship | null> {
  const rows = await db.select().from(friendships).where(pairCondition(a, b)).limit(1);
  return rows[0] ?? null;
}

async function profilesById(ids: string[]): Promise<Map<string, PlayerProfile>> {
  if (ids.length === 0) return new Map();
  const rows = await db.select().from(playerProfiles).where(inArray(playerProfiles.playerId, ids));
  return new Map(rows.map((p) => [p.playerId, p]));
}

/**
 * Lists accepted friends of a player with their presence.
 * @param playerId - Player id.
 * @returns Friends.
 */
export async function listFriends(playerId: string): Promise<FriendView[]> {
  const rows = await db
    .select()
    .from(friendships)
    .where(
      and(
        eq(friendships.status, 'accepted'),
        or(eq(friendships.requesterId, playerId), eq(friendships.addresseeId, playerId)),
      ),
    )
    .orderBy(desc(friendships.updatedAt));
  const otherIds = rows.map((r) => (r.requesterId === playerId ? r.addresseeId : r.requesterId));
  const [profiles, presence] = await Promise.all([profilesById(otherIds), getPresenceMap(otherIds)]);
  return rows.flatMap((r) => {
    const otherId = r.requesterId === playerId ? r.addresseeId : r.requesterId;
    const profile = profiles.get(otherId);
    if (!profile) return [];
    const p = presence.get(otherId);
    return [{ ...profile, friendsSince: r.updatedAt, status: p?.status ?? 'offline', location: p?.location ?? null }];
  });
}

/**
 * Friends currently not offline.
 * @param playerId - Player id.
 * @returns Online / in-mission friends.
 */
export async function listOnlineFriends(playerId: string): Promise<FriendView[]> {
  const friends = await listFriends(playerId);
  return friends.filter((f) => f.status !== 'offline');
}

/**
 * Pending requests received and sent by a player.
 * @param playerId - Player id.
 * @returns Incoming and outgoing requests.
 */
export async function listRequests(
  playerId: string,
): Promise<{ incoming: FriendRequestView[]; outgoing: FriendRequestView[] }> {
  const rows = await db
    .select()
    .from(friendships)
    .where(
      and(
        eq(friendships.status, 'pending'),
        or(eq(friendships.requesterId, playerId), eq(friendships.addresseeId, playerId)),
      ),
    )
    .orderBy(desc(friendships.createdAt));
  const profiles = await profilesById(
    rows.map((r) => (r.requesterId === playerId ? r.addresseeId : r.requesterId)),
  );
  const toView = (r: Friendship, otherId: string): FriendRequestView[] => {
    const player = profiles.get(otherId);
    return player ? [{ id: r.id, createdAt: r.createdAt, player }] : [];
  };
  return {
    incoming: rows.filter((r) => r.addresseeId === playerId).flatMap((r) => toView(r, r.requesterId)),
    outgoing: rows.filter((r) => r.requesterId === playerId).flatMap((r) => toView(r, r.addresseeId)),
  };
}

/**
 * Sends a friend request. If the target already sent one, it is accepted instead.
 * @param requesterId - Sender.
 * @param addresseeId - Target player.
 * @returns Resulting friendship row.
 */
export async function sendRequest(requesterId: string, addresseeId: string): Promise<Friendship> {
  if (requesterId === addresseeId) {
    throw new HttpError(400, 'INVALID_TARGET', 'Cannot send a friend request to yourself');
  }
  await requireProfile(addresseeId);
  if (await isBlockedEitherWay(requesterId, addresseeId)) {
    throw new HttpError(409, 'BLOCKED', 'A block exists between these players');
  }
  const existing = await findRelation(requesterId, addresseeId);
  if (existing) {
    if (existing.status === 'accepted') throw conflict('Already friends');
    if (existing.requesterId === requesterId) throw conflict('Request already pending');
    return acceptRequest(existing.id, requesterId);
  }
  const [created] = await db.insert(friendships).values({ requesterId, addresseeId }).returning();
  await recordActivity(requesterId, 'friend_request_sent', { playerId: addresseeId });
  await recordActivity(addresseeId, 'friend_request_received', { playerId: requesterId });
  return created;
}

async function requirePendingRequest(id: number): Promise<Friendship> {
  const rows = await db.select().from(friendships).where(eq(friendships.id, id)).limit(1);
  const request = rows[0];
  if (!request || request.status !== 'pending') throw notFound(`Friend request ${id} not found`);
  return request;
}

/**
 * Accepts a pending request (addressee only).
 * @param id - Request id.
 * @param playerId - Acting player.
 * @returns Accepted friendship.
 */
export async function acceptRequest(id: number, playerId: string): Promise<Friendship> {
  const request = await requirePendingRequest(id);
  if (request.addresseeId !== playerId) throw forbidden('Only the addressee can accept a request');
  const [accepted] = await db
    .update(friendships)
    .set({ status: 'accepted', updatedAt: new Date() })
    .where(eq(friendships.id, id))
    .returning();
  await recordActivity(playerId, 'friend_added', { playerId: request.requesterId });
  await recordActivity(request.requesterId, 'friend_added', { playerId });
  return accepted;
}

/**
 * Declines (addressee) or cancels (requester) a pending request.
 * @param id - Request id.
 * @param playerId - Acting player.
 */
export async function declineRequest(id: number, playerId: string): Promise<void> {
  const request = await requirePendingRequest(id);
  if (request.addresseeId !== playerId && request.requesterId !== playerId) {
    throw forbidden('Not a party to this request');
  }
  await db.delete(friendships).where(eq(friendships.id, id));
  const type = request.addresseeId === playerId ? 'friend_request_declined' : 'friend_request_cancelled';
  const otherId = request.addresseeId === playerId ? request.requesterId : request.addresseeId;
  await recordActivity(playerId, type, { playerId: otherId });
}

/**
 * Removes an accepted friendship.
 * @param playerId - Acting player.
 * @param otherId - Friend to remove.
 * @returns True if a friendship was removed.
 */
export async function removeFriend(playerId: string, otherId: string): Promise<boolean> {
  const deleted = await db
    .delete(friendships)
    .where(and(eq(friendships.status, 'accepted'), pairCondition(playerId, otherId)))
    .returning({ id: friendships.id });
  if (deleted.length > 0) {
    await recordActivity(playerId, 'friend_removed', { playerId: otherId });
  }
  return deleted.length > 0;
}

/**
 * Recently met players that are neither friends, pending, nor blocked.
 * @param playerId - Player id.
 * @param limit - Max suggestions.
 * @returns Suggested profiles.
 */
export async function listSuggestions(playerId: string, limit: number): Promise<SuggestionView[]> {
  const [encounters, relations, blocked] = await Promise.all([
    listRecentEncounters(playerId, limit * 3),
    db
      .select({ requesterId: friendships.requesterId, addresseeId: friendships.addresseeId })
      .from(friendships)
      .where(or(eq(friendships.requesterId, playerId), eq(friendships.addresseeId, playerId))),
    blockedIdsFor(playerId),
  ]);
  const related = new Set(relations.map((r) => (r.requesterId === playerId ? r.addresseeId : r.requesterId)));
  const candidates = encounters.filter((e) => !related.has(e.otherId) && !blocked.has(e.otherId)).slice(0, limit);
  const profiles = await profilesById(candidates.map((e) => e.otherId));
  return candidates.flatMap((e) => {
    const profile = profiles.get(e.otherId);
    return profile ? [{ ...profile, lastMetAt: e.lastMetAt, encounters: e.count }] : [];
  });
}
