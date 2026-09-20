/**
 * Moderation dashboard routes (`/api/admin`). Requires the `moderator` role at least;
 * reputation adjustments and sanctions beyond warnings require `admin`.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { hasRole, requirePlayer, requireRole } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { HttpError } from '../lib/httpError.js';
import { listActivity } from '../services/activity.service.js';
import { listModerationLog } from '../services/moderationLog.service.js';
import { requireProfile } from '../services/profiles.service.js';
import * as reports from '../services/reports.service.js';
import { adjustReputation, listReputationEvents } from '../services/reputation.service.js';
import * as sanctions from '../services/sanctions.service.js';
import { getCommunityStats } from '../services/stats.service.js';
import {
  limitQuery,
  playerIdParams,
  reportIdParams,
  reportStatusBody,
  reportsQuery,
  reputationAdjustBody,
  sanctionBody,
  sanctionIdParams,
  sanctionsQuery,
} from './schemas.js';

/** Router for moderators, admins and supervisors. */
export const adminRoutes: IRouter = Router();

adminRoutes.use(requireRole('moderator'));

/** GET /stats — Community analytics. */
adminRoutes.get(
  '/stats',
  asyncHandler(async (_req, res) => {
    res.json(await getCommunityStats());
  }),
);

/** GET /log?limit= — Moderation audit log. */
adminRoutes.get(
  '/log',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listModerationLog(Number(req.query.limit)));
  }),
);

// ── Reports ─────────────────────────────────────────────────────────────────

/** GET /reports?status=&escalation=&targetPlayerId=&limit= — Report queue. */
adminRoutes.get(
  '/reports',
  validate(reportsQuery, 'query'),
  asyncHandler(async (req, res) => {
    const { status, escalation, targetPlayerId, limit } = req.query as Record<string, string | undefined>;
    res.json(
      await reports.listReports(
        { status: status as never, escalation: escalation as never, targetPlayerId },
        Number(limit),
      ),
    );
  }),
);

/** GET /reports/:id — One report. */
adminRoutes.get(
  '/reports/:id',
  validate(reportIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await reports.requireReport(Number(req.params.id)));
  }),
);

/** PATCH /reports/:id — Set status (reviewing / resolved / dismissed). */
adminRoutes.patch(
  '/reports/:id',
  validate(reportIdParams, 'params'),
  validate(reportStatusBody),
  asyncHandler(async (req, res) => {
    res.json(await reports.updateReportStatus(Number(req.params.id), requirePlayer(req).id, req.body.status, req.body.note));
  }),
);

/** POST /reports/:id/escalate — Escalate one level up. */
adminRoutes.post(
  '/reports/:id/escalate',
  validate(reportIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await reports.escalateReport(Number(req.params.id), requirePlayer(req).id));
  }),
);

// ── Players ─────────────────────────────────────────────────────────────────

/** GET /players/:playerId — Profile with reputation history, sanctions, reports and activity. */
adminRoutes.get(
  '/players/:playerId',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const id = req.params.playerId;
    const [profile, reputationEvents, sanctionHistory, reportsAgainst, activity] = await Promise.all([
      requireProfile(id),
      listReputationEvents(id, 50),
      sanctions.listSanctionHistory(id, 50),
      reports.listReports({ targetPlayerId: id }, 50),
      listActivity(id, 50),
    ]);
    res.json({ ...profile, reputationEvents, sanctions: sanctionHistory, reports: reportsAgainst, activity });
  }),
);

/** POST /players/:playerId/reputation — Manual reputation adjustment (admin). */
adminRoutes.post(
  '/players/:playerId/reputation',
  requireRole('admin'),
  validate(playerIdParams, 'params'),
  validate(reputationAdjustBody),
  asyncHandler(async (req, res) => {
    const actor = requirePlayer(req);
    const reputation = await adjustReputation(req.params.playerId, req.body.delta, 'moderation', req.body.reason, { actorId: actor.id });
    res.json({ playerId: req.params.playerId, reputation });
  }),
);

/** POST /players/:playerId/sanctions — Issue a sanction (moderators: warning/mute; admin+: suspension/ban). */
adminRoutes.post(
  '/players/:playerId/sanctions',
  validate(playerIdParams, 'params'),
  validate(sanctionBody),
  asyncHandler(async (req, res) => {
    const actor = requirePlayer(req);
    if (['suspension', 'ban'].includes(req.body.type) && !hasRole(actor, 'admin')) {
      throw new HttpError(403, 'FORBIDDEN', 'Suspensions and bans require the admin role');
    }
    res.status(201).json(await sanctions.issueSanction(req.params.playerId, req.body, actor.id));
  }),
);

// ── Sanctions ───────────────────────────────────────────────────────────────

/** GET /sanctions?playerId=&active=&limit= — Sanctions listing. */
adminRoutes.get(
  '/sanctions',
  validate(sanctionsQuery, 'query'),
  asyncHandler(async (req, res) => {
    const q = req.query as unknown as { playerId?: string; active: boolean; limit: number };
    res.json(await sanctions.listSanctions({ playerId: q.playerId, activeOnly: q.active }, Number(q.limit)));
  }),
);

/** DELETE /sanctions/:id — Revoke a sanction. */
adminRoutes.delete(
  '/sanctions/:id',
  validate(sanctionIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await sanctions.revokeSanction(Number(req.params.id), requirePlayer(req).id));
  }),
);
