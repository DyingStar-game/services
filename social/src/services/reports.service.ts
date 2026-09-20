/**
 * Player and guild reports, their moderation workflow and escalation.
 */
import { and, desc, eq, inArray } from 'drizzle-orm';

import { env } from '../config/env.js';
import { db } from '../db/connection.js';
import {
  ESCALATION_LEVELS,
  guilds,
  playerProfiles,
  reports,
  type EscalationLevel,
  type Report,
  type ReportReason,
  type ReportStatus,
  type ReportTargetType,
} from '../db/schema/index.js';
import { HttpError, conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordActivity } from './activity.service.js';
import { requireGuild } from './guilds.service.js';
import { logModeration } from './moderationLog.service.js';
import { requireProfile } from './profiles.service.js';
import { adjustReputation } from './reputation.service.js';

/** Report enriched with target labels for dashboards. */
export interface ReportView extends Report {
  reporterName: string | null;
  targetName: string | null;
}

const OPEN_STATUSES: ReportStatus[] = ['open', 'reviewing'];

/**
 * Files a report against a player or a guild. The reported player loses `reportPenalty` points.
 * @param reporterId - Reporting player.
 * @param input - Target and reason.
 * @returns Created report.
 */
export async function createReport(
  reporterId: string,
  input: { targetType: ReportTargetType; targetId: string; reason: ReportReason; message?: string },
): Promise<Report> {
  const targetPlayerId = input.targetType === 'player' ? input.targetId : null;
  const targetGuildId = input.targetType === 'guild' ? input.targetId : null;
  if (targetPlayerId === reporterId) throw new HttpError(400, 'INVALID_TARGET', 'Cannot report yourself');
  if (targetPlayerId) await requireProfile(targetPlayerId);
  if (targetGuildId) await requireGuild(targetGuildId);

  const duplicate = await db
    .select({ id: reports.id })
    .from(reports)
    .where(
      and(
        eq(reports.reporterId, reporterId),
        targetPlayerId ? eq(reports.targetPlayerId, targetPlayerId) : eq(reports.targetGuildId, targetGuildId!),
        inArray(reports.status, OPEN_STATUSES),
      ),
    )
    .limit(1);
  if (duplicate.length) throw conflict('You already have an open report on this target');

  const [report] = await db
    .insert(reports)
    .values({ reporterId, targetType: input.targetType, targetPlayerId, targetGuildId, reason: input.reason, message: input.message ?? null })
    .returning();
  await recordActivity(reporterId, 'report_filed', { reportId: report.id, targetType: input.targetType, targetId: input.targetId });
  if (targetPlayerId) {
    await adjustReputation(targetPlayerId, -env.reputation.reportPenalty, 'report', `Reported: ${input.reason}`, {
      actorId: reporterId,
      details: { reportId: report.id },
    });
  }
  return report;
}

/**
 * Reports filed by a player.
 * @param reporterId - Player id.
 * @param limit - Max rows.
 * @returns Reports, newest first.
 */
export async function listMyReports(reporterId: string, limit: number): Promise<Report[]> {
  return db.select().from(reports).where(eq(reports.reporterId, reporterId)).orderBy(desc(reports.createdAt)).limit(limit);
}

async function toViews(rows: Report[]): Promise<ReportView[]> {
  const playerIds = [...new Set(rows.flatMap((r) => [r.reporterId, r.targetPlayerId]).filter((id): id is string => !!id))];
  const guildIds = [...new Set(rows.map((r) => r.targetGuildId).filter((id): id is string => !!id))];
  const [players, guildRows] = await Promise.all([
    playerIds.length
      ? db.select({ id: playerProfiles.playerId, name: playerProfiles.displayName }).from(playerProfiles).where(inArray(playerProfiles.playerId, playerIds))
      : [],
    guildIds.length ? db.select({ id: guilds.id, name: guilds.name }).from(guilds).where(inArray(guilds.id, guildIds)) : [],
  ]);
  const names = new Map([...players, ...guildRows].map((r) => [r.id, r.name]));
  return rows.map((r) => ({
    ...r,
    reporterName: r.reporterId ? (names.get(r.reporterId) ?? null) : null,
    targetName: names.get(r.targetPlayerId ?? r.targetGuildId ?? '') ?? null,
  }));
}

/**
 * Dashboard listing with optional filters.
 * @param filter - Status, escalation level, target player.
 * @param limit - Max rows.
 * @returns Report views, newest first.
 */
export async function listReports(
  filter: { status?: ReportStatus; escalation?: EscalationLevel; targetPlayerId?: string },
  limit: number,
): Promise<ReportView[]> {
  const conditions = [];
  if (filter.status) conditions.push(eq(reports.status, filter.status));
  if (filter.escalation) conditions.push(eq(reports.escalation, filter.escalation));
  if (filter.targetPlayerId) conditions.push(eq(reports.targetPlayerId, filter.targetPlayerId));
  const query = db.select().from(reports);
  const rows = await (conditions.length ? query.where(and(...conditions)) : query).orderBy(desc(reports.createdAt)).limit(limit);
  return toViews(rows);
}

/**
 * Fetches one report or throws 404.
 * @param id - Report id.
 * @returns Report view.
 */
export async function requireReport(id: number): Promise<ReportView> {
  const [row] = await db.select().from(reports).where(eq(reports.id, id)).limit(1);
  if (!row) throw notFound(`Report ${id} not found`);
  const [view] = await toViews([row]);
  return view;
}

/**
 * Moves a report through the workflow. `resolved` (upheld) costs the reported player
 * `upheldReportPenalty`; `dismissed` refunds the initial report penalty.
 * @param id - Report id.
 * @param actorId - Moderator id.
 * @param status - New status.
 * @param note - Resolution note.
 * @returns Updated report.
 */
export async function updateReportStatus(
  id: number,
  actorId: string,
  status: Exclude<ReportStatus, 'open'>,
  note?: string,
): Promise<Report> {
  const report = await requireReport(id);
  if (!OPEN_STATUSES.includes(report.status)) throw conflict(`Report is already ${report.status}`);
  const closing = status !== 'reviewing';
  const [updated] = await db
    .update(reports)
    .set({
      status,
      resolvedBy: closing ? actorId : null,
      resolutionNote: note ?? null,
      resolvedAt: closing ? new Date() : null,
    })
    .where(eq(reports.id, id))
    .returning();
  await logModeration(actorId, `report_${status}`, report.targetPlayerId, { reportId: id, note });

  if (report.targetPlayerId && report.reporterId) {
    if (status === 'resolved') {
      await adjustReputation(report.targetPlayerId, -env.reputation.upheldReportPenalty, 'moderation', `Report upheld: ${report.reason}`, {
        actorId,
        details: { reportId: id },
      });
    } else if (status === 'dismissed') {
      await adjustReputation(report.targetPlayerId, env.reputation.reportPenalty, 'moderation', 'Report dismissed', {
        actorId,
        details: { reportId: id },
      });
    }
  }
  if (report.reporterId && closing) {
    await recordActivity(report.reporterId, `report_${status}`, { reportId: id });
  }
  return updated;
}

/**
 * Escalates a report one level up (moderator → admin → supervisor).
 * @param id - Report id.
 * @param actorId - Moderator id.
 * @returns Updated report.
 */
export async function escalateReport(id: number, actorId: string): Promise<Report> {
  const report = await requireReport(id);
  if (!OPEN_STATUSES.includes(report.status)) throw conflict(`Report is already ${report.status}`);
  const idx = ESCALATION_LEVELS.indexOf(report.escalation);
  if (idx >= ESCALATION_LEVELS.length - 1) throw forbidden('Report is already at the highest escalation level');
  const escalation = ESCALATION_LEVELS[idx + 1];
  const [updated] = await db.update(reports).set({ escalation, status: 'open' }).where(eq(reports.id, id)).returning();
  await logModeration(actorId, 'report_escalated', report.targetPlayerId, { reportId: id, to: escalation });
  return updated;
}
