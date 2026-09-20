/**
 * API router: public routes, player routes (JWT) and internal routes (shared key).
 */
import { Router, type IRouter } from 'express';

import { internalAuth, playerAuth } from '../middleware/auth.js';
import { blocksRoutes } from './blocks.routes.js';
import { friendsRoutes } from './friends.routes.js';
import { internalRoutes } from './internal.routes.js';
import { meRoutes } from './me.routes.js';
import { profilesRoutes } from './profiles.routes.js';
import { publicRoutes } from './public.routes.js';

/** Top-level `/api` router. */
export const apiRouter: IRouter = Router();

apiRouter.use(publicRoutes);

apiRouter.use('/internal', internalAuth, internalRoutes);

// Auth is attached per prefix so unknown paths fall through to the 404 handler.
apiRouter.use('/me', playerAuth, meRoutes);
apiRouter.use('/profiles', playerAuth, profilesRoutes);
apiRouter.use('/friends', playerAuth, friendsRoutes);
apiRouter.use('/blocks', playerAuth, blocksRoutes);
