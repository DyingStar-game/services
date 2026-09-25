/**
 * Community analytics for the moderation dashboard.
 */
import { and, count, desc, eq, gt, isNull, ne, or, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  corporationMembers,
  corporations,
  playerActivity,
  playerPresence,
  playerProfiles,
  reports,
  sanctions,
} from '../db/schema/index.js';

/**
 * Aggregated community figures. NPC profiles are excluded from the player-facing counters
 * (total, online, activity) and from the reputation ranking, but their corporation memberships count.
 * @returns Player, corporation, report and sanction counters plus top lists.
 */
export async function getCommunityStats() {
  const dayAgo = new Date(Date.now() - 86_400_000);
  const playersOnly = eq(playerProfiles.entityType, 'player');
  const [[players], [online], [corporationCount], reportsByStatus, [activeSanctions], [activity24h], topCorporations, mostReported, lowestReputation] =
    await Promise.all([
      db.select({ n: count() }).from(playerProfiles).where(playersOnly),
      db
        .select({ n: count() })
        .from(playerPresence)
        .innerJoin(playerProfiles, eq(playerProfiles.playerId, playerPresence.playerId))
        .where(and(ne(playerPresence.status, 'offline'), playersOnly)),
      db.select({ n: count() }).from(corporations),
      db.select({ status: reports.status, n: count() }).from(reports).groupBy(reports.status),
      db
        .select({ n: count() })
        .from(sanctions)
        .where(and(isNull(sanctions.revokedAt), or(isNull(sanctions.expiresAt), gt(sanctions.expiresAt, new Date())))),
      db
        .select({ n: count() })
        .from(playerActivity)
        .innerJoin(playerProfiles, eq(playerProfiles.playerId, playerActivity.playerId))
        .where(and(gt(playerActivity.createdAt, dayAgo), playersOnly)),
      db
        .select({
          id: corporations.id,
          name: corporations.name,
          ticker: corporations.ticker,
          members: count(corporationMembers.playerId),
        })
        .from(corporations)
        .leftJoin(corporationMembers, eq(corporationMembers.corporationId, corporations.id))
        .groupBy(corporations.id)
        .orderBy(desc(count(corporationMembers.playerId)))
        .limit(5),
      db
        .select({ playerId: reports.targetPlayerId, reports: count() })
        .from(reports)
        .where(sql`${reports.targetPlayerId} is not null and ${reports.status} in ('open', 'reviewing')`)
        .groupBy(reports.targetPlayerId)
        .orderBy(desc(count()))
        .limit(5),
      db
        .select({ playerId: playerProfiles.playerId, displayName: playerProfiles.displayName, reputation: playerProfiles.reputation })
        .from(playerProfiles)
        .where(playersOnly)
        .orderBy(playerProfiles.reputation)
        .limit(5),
    ]);
  return {
    players: { total: players.n, online: online.n },
    corporations: { total: corporationCount.n, top: topCorporations },
    reports: Object.fromEntries(reportsByStatus.map((r) => [r.status, r.n])),
    sanctions: { active: activeSanctions.n },
    activityLast24h: activity24h.n,
    mostReported,
    lowestReputation,
  };
}
