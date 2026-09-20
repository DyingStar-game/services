/**
 * API router: public routes, player routes (JWT) and internal routes (shared key).
 */
import { Router, type IRouter, type NextFunction, type Request, type Response } from 'express';

import { internalAuth, playerAuth } from '../middleware/auth.js';
import { enforceSanctions } from '../middleware/sanctions.js';
import { adminRoutes } from './admin.routes.js';
import { blocksRoutes } from './blocks.routes.js';
import { friendsRoutes } from './friends.routes.js';
import { guildsRoutes } from './guilds.routes.js';
import { internalRoutes } from './internal.routes.js';
import { meRoutes } from './me.routes.js';
import { profilesRoutes } from './profiles.routes.js';
import { publicRoutes } from './public.routes.js';
import { reportsRoutes } from './reports.routes.js';

/** Top-level `/api` router. */
export const apiRouter: IRouter = Router();

apiRouter.use(publicRoutes);

apiRouter.use('/internal', internalAuth, internalRoutes);

// Auth is attached per prefix so unknown paths fall through to the 404 handler.
// Sanctioned (suspended/banned) players keep read access to /me/reputation and /me/sanctions only.
apiRouter.use('/me', playerAuth, exemptSanctioned, meRoutes);
apiRouter.use('/profiles', playerAuth, enforceSanctions, profilesRoutes);
apiRouter.use('/friends', playerAuth, enforceSanctions, friendsRoutes);
apiRouter.use('/blocks', playerAuth, enforceSanctions, blocksRoutes);
apiRouter.use('/guilds', playerAuth, enforceSanctions, guildsRoutes);
apiRouter.use('/reports', playerAuth, enforceSanctions, reportsRoutes);
apiRouter.use('/admin', playerAuth, adminRoutes);

/** Skips the sanction check for the two self-service moderation endpoints. */
function exemptSanctioned(req: Request, res: Response, next: NextFunction): void {
  if (req.path === '/reputation' || req.path === '/sanctions') {
    next();
    return;
  }
  enforceSanctions(req, res, next).catch(next);
}
