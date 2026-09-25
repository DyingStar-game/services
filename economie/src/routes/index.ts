/**
 * API router: public routes, player routes (JWT) and internal routes (shared key).
 */
import { Router, type IRouter } from 'express';

import { internalAuth, playerAuth } from '../middleware/auth.js';
import { corporationsRoutes } from './corporations.routes.js';
import { internalRoutes } from './internal.routes.js';
import { meRoutes } from './me.routes.js';
import { publicRoutes } from './public.routes.js';
import { transfersRoutes } from './transfers.routes.js';

/** Top-level `/api` router. */
export const apiRouter: IRouter = Router();

apiRouter.use(publicRoutes);

apiRouter.use('/internal', internalAuth, internalRoutes);

// Auth is attached per prefix so unknown paths fall through to the 404 handler.
apiRouter.use('/me', playerAuth, meRoutes);
apiRouter.use('/transfers', playerAuth, transfersRoutes);
apiRouter.use('/corporations', playerAuth, corporationsRoutes);