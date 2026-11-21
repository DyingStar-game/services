/**
 * In-game bug reports (`/api/bug-reports`, authenticated player), relayed to Discord.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { bugReportsEnabled, submitBugReport } from '../services/bugReports.service.js';
import { createBugReportBody } from './schemas.js';

/** Router for sending a bug report and asking whether this environment relays them. */
export const bugReportsRoutes: IRouter = Router();

/** GET /status — Whether a report sent now would be relayed (lets the game say so before the form). */
bugReportsRoutes.get('/status', (_req, res) => {
  res.json({ enabled: bugReportsEnabled() });
});

/** POST / — Send a bug report: title, description, context, capture and log tail. */
bugReportsRoutes.post(
  '/',
  validate(createBugReportBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await submitBugReport(requirePlayer(req), req.body));
  }),
);
