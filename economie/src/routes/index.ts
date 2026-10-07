/**
 * API router: public routes and player routes (JWT). Internal routes are mounted directly
 * on the app in `index.ts` so their service guard lives on the mount point.
 */
import { Router, type IRouter } from 'express';

import { playerAuth } from '../middleware/auth.js';
import { adminRoutes } from './admin.routes.js';
import { corporationsRoutes } from './corporations.routes.js';
import { meRoutes } from './me.routes.js';
import { publicRoutes } from './public.routes.js';
import { transfersRoutes } from './transfers.routes.js';

/** Top-level `/api` router. */
export const apiRouter: IRouter = Router();

apiRouter.use(publicRoutes);

// Auth is attached per prefix so unknown paths fall through to the 404 handler.
apiRouter.use('/me', playerAuth, meRoutes);
apiRouter.use('/transfers', playerAuth, transfersRoutes);
apiRouter.use('/corporations', playerAuth, corporationsRoutes);
apiRouter.use('/admin', playerAuth, adminRoutes);