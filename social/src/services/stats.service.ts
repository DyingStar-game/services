/**
 * Community analytics for the moderation dashboard.
 */
import { and, count, desc, eq, gt, isNull, ne, or, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import {
  guildMembers,
  guilds,
  playerActivity,
  playerPresence,
  playerProfiles,
  reports,
  sanctions,
} from '../db/schema/index.js';

/**
 * Aggregated community figures.
 * @returns Player, guild, report and sanction counters plus top lists.
 */
export async function getCommunityStats() {
  const dayAgo = new Date(Date.now() - 86_400_000);
  const [[players], [online], [guildCount], reportsByStatus, [activeSanctions], [activity24h], topGuilds, mostReported, lowestReputation] =
    await Promise.all([
      db.select({ n: count() }).from(playerProfiles),
      db.select({ n: count() }).from(playerPresence).where(ne(playerPresence.status, 'offline')),
      db.select({ n: count() }).from(guilds),
      db.select({ status: reports.status, n: count() }).from(reports).groupBy(reports.status),
      db
        .select({ n: count() })
        .from(sanctions)
        .where(and(isNull(sanctions.revokedAt), or(isNull(sanctions.expiresAt), gt(sanctions.expiresAt, new Date())))),
      db.select({ n: count() }).from(playerActivity).where(gt(playerActivity.createdAt, dayAgo)),
      db
        .select({ id: guilds.id, name: guilds.name, tag: guilds.tag, members: count(guildMembers.playerId) })
        .from(guilds)
        .leftJoin(guildMembers, eq(guildMembers.guildId, guilds.id))
        .groupBy(guilds.id)
        .orderBy(desc(count(guildMembers.playerId)))
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
        .orderBy(playerProfiles.reputation)
        .limit(5),
    ]);
  return {
    players: { total: players.n, online: online.n },
    guilds: { total: guildCount.n, top: topGuilds },
    reports: Object.fromEntries(reportsByStatus.map((r) => [r.status, r.n])),
    sanctions: { active: activeSanctions.n },
    activityLast24h: activity24h.n,
    mostReported,
    lowestReputation,
  };
}
