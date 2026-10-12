/**
 * Friend requests, friendships, online friends and suggestions.
 */
import { and, count, desc, eq, inArray, or } from 'drizzle-orm';
import { t } from '../i18n/index.js';

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
import { page, type Page } from '../lib/pagination.js';
import { recordActivity } from './activity.service.js';
import { blockedIdsFor, isBlockedEitherWay } from './blocks.service.js';
import { listRecentEncounters } from './encounters.service.js';
import { notifyPlayer } from './notifications.service.js';
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

/** Accepted-friendship condition of a player. */
function acceptedFriendship(playerId: string) {
  return and(
    eq(friendships.status, 'accepted'),
    or(eq(friendships.requesterId, playerId), eq(friendships.addresseeId, playerId)),
  );
}

async function toFriendViews(rows: Friendship[], playerId: string): Promise<FriendView[]> {
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
 * Lists accepted friends of a player with their presence.
 * @param playerId - Player id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of friends.
 */
export async function listFriends(playerId: string, limit: number, offset: number): Promise<Page<FriendView>> {
  const condition = acceptedFriendship(playerId);
  const [totalRow, rows] = await Promise.all([
    db.select({ total: count() }).from(friendships).where(condition),
    db
      .select()
      .from(friendships)
      .where(condition)
      .orderBy(desc(friendships.updatedAt), desc(friendships.id))
      .limit(limit)
      .offset(offset),
  ]);
  return page(await toFriendViews(rows, playerId), totalRow[0].total, limit, offset);
}

/**
 * Friends currently not offline. Presence is checked for the whole friend list, so the
 * page is cut from the online subset.
 * @param playerId - Player id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of online / in-mission friends.
 */
export async function listOnlineFriends(playerId: string, limit: number, offset: number): Promise<Page<FriendView>> {
  const rows = await db
    .select()
    .from(friendships)
    .where(acceptedFriendship(playerId))
    .orderBy(desc(friendships.updatedAt), desc(friendships.id));
  const otherIds = rows.map((r) => (r.requesterId === playerId ? r.addresseeId : r.requesterId));
  const presence = await getPresenceMap(otherIds);
  const online = rows.filter((r) => {
    const otherId = r.requesterId === playerId ? r.addresseeId : r.requesterId;
    return (presence.get(otherId)?.status ?? 'offline') !== 'offline';
  });
  const slice = online.slice(offset, offset + limit);
  const views = await toFriendViews(slice, playerId);
  return page(
    views.map((v) => ({ ...v, status: presence.get(v.playerId)?.status ?? v.status })),
    online.length,
    limit,
    offset,
  );
}

/** Pending requests received and sent by a player, paginated per direction. */
export interface FriendRequestsPage {
  incoming: FriendRequestView[];
  outgoing: FriendRequestView[];
  incomingTotal: number;
  outgoingTotal: number;
  limit: number;
  offset: number;
}

/**
 * Pending requests received and sent by a player. Both directions share the same
 * `limit`/`offset` and carry their own total.
 * @param playerId - Player id.
 * @param limit - Page size applied to each direction.
 * @param offset - Rows to skip in each direction.
 * @returns Incoming and outgoing requests.
 */
export async function listRequests(playerId: string, limit: number, offset: number): Promise<FriendRequestsPage> {
  const pending = (direction: 'incoming' | 'outgoing') =>
    and(
      eq(friendships.status, 'pending'),
      direction === 'incoming' ? eq(friendships.addresseeId, playerId) : eq(friendships.requesterId, playerId),
    );
  const [incomingTotal, outgoingTotal, incomingRows, outgoingRows] = await Promise.all([
    db.select({ total: count() }).from(friendships).where(pending('incoming')),
    db.select({ total: count() }).from(friendships).where(pending('outgoing')),
    db
      .select()
      .from(friendships)
      .where(pending('incoming'))
      .orderBy(desc(friendships.createdAt), desc(friendships.id))
      .limit(limit)
      .offset(offset),
    db
      .select()
      .from(friendships)
      .where(pending('outgoing'))
      .orderBy(desc(friendships.createdAt), desc(friendships.id))
      .limit(limit)
      .offset(offset),
  ]);
  const profiles = await profilesById([
    ...incomingRows.map((r) => r.requesterId),
    ...outgoingRows.map((r) => r.addresseeId),
  ]);
  const toView = (r: Friendship, otherId: string): FriendRequestView[] => {
    const player = profiles.get(otherId);
    return player ? [{ id: r.id, createdAt: r.createdAt, player }] : [];
  };
  return {
    incoming: incomingRows.flatMap((r) => toView(r, r.requesterId)),
    outgoing: outgoingRows.flatMap((r) => toView(r, r.addresseeId)),
    incomingTotal: incomingTotal[0].total,
    outgoingTotal: outgoingTotal[0].total,
    limit,
    offset,
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
    throw new HttpError(400, 'INVALID_TARGET', t('target.friend_self'));
  }
  await requireProfile(addresseeId);
  if (await isBlockedEitherWay(requesterId, addresseeId)) {
    throw new HttpError(409, 'BLOCKED', 'A block exists between these players');
  }
  const existing = await findRelation(requesterId, addresseeId);
  if (existing) {
    if (existing.status === 'accepted') throw conflict(t('conflict.already_friends'));
    if (existing.requesterId === requesterId) throw conflict(t('conflict.request_pending'));
    return acceptRequest(existing.id, requesterId);
  }
  const [created] = await db.insert(friendships).values({ requesterId, addresseeId }).returning();
  await recordActivity(requesterId, 'friend_request_sent', { playerId: addresseeId });
  await recordActivity(addresseeId, 'friend_request_received', { playerId: requesterId });
  void notifyPlayer(addresseeId, {
    type: 'friend_request_received',
    data: { fromPlayerId: requesterId },
  }).catch((err: Error) => console.warn(`[friends] notify failed: ${err.message}`));
  return created;
}

async function requirePendingRequest(id: number): Promise<Friendship> {
  const rows = await db.select().from(friendships).where(eq(friendships.id, id)).limit(1);
  const request = rows[0];
  if (!request || request.status !== 'pending') throw notFound(t('not_found.friend_request', { id: id }));
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
  if (request.addresseeId !== playerId) throw forbidden(t('forbidden.addressee_only'));
  const [accepted] = await db
    .update(friendships)
    .set({ status: 'accepted', updatedAt: new Date() })
    .where(eq(friendships.id, id))
    .returning();
  await recordActivity(playerId, 'friend_added', { playerId: request.requesterId });
  await recordActivity(request.requesterId, 'friend_added', { playerId });
  void notifyPlayer(request.requesterId, {
    type: 'friend_request_accepted',
    data: { playerId },
  }).catch((err: Error) => console.warn(`[friends] notify failed: ${err.message}`));
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
    throw forbidden(t('forbidden.not_party'));
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
 * Recently met players that are neither friends, pending, nor blocked. The candidate pool
 * is scanned over `(offset + limit) * 3` recent encounters, so `total` counts the
 * suggestions found in that window (encounters are bounded by design, not by the page).
 * @param playerId - Player id.
 * @param limit - Page size.
 * @param offset - Rows to skip.
 * @returns Page of suggested profiles.
 */
export async function listSuggestions(playerId: string, limit: number, offset: number): Promise<Page<SuggestionView>> {
  const [encounters, relations, blocked] = await Promise.all([
    listRecentEncounters(playerId, (offset + limit) * 3),
    db
      .select({ requesterId: friendships.requesterId, addresseeId: friendships.addresseeId })
      .from(friendships)
      .where(or(eq(friendships.requesterId, playerId), eq(friendships.addresseeId, playerId))),
    blockedIdsFor(playerId),
  ]);
  const related = new Set(relations.map((r) => (r.requesterId === playerId ? r.addresseeId : r.requesterId)));
  const candidates = encounters.filter((e) => !related.has(e.otherId) && !blocked.has(e.otherId));
  const slice = candidates.slice(offset, offset + limit);
  const profiles = await profilesById(slice.map((e) => e.otherId));
  const items = slice.flatMap((e) => {
    const profile = profiles.get(e.otherId);
    return profile ? [{ ...profile, lastMetAt: e.lastMetAt, encounters: e.count }] : [];
  });
  return page(items, candidates.length, limit, offset);
}
