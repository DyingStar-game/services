/**
 * Player-side reporting (`/api/reports`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { createReport, listMyReports } from '../services/reports.service.js';
import { createReportBody, limitQuery } from './schemas.js';

/** Router for filing and listing one's own reports. */
export const reportsRoutes: IRouter = Router();

/** POST / — Report a player or a corporation. */
reportsRoutes.post(
  '/',
  validate(createReportBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await createReport(requirePlayer(req).id, req.body));
  }),
);

/** GET /?limit= — Reports I filed. */
reportsRoutes.get(
  '/',
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listMyReports(requirePlayer(req).id, Number(req.query.limit)));
  }),
);
