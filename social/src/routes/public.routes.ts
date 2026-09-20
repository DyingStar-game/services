/**
 * Public API routes (no authentication).
 */
import { Router, type IRouter } from 'express';

/** Router mounted at `/api` for unauthenticated endpoints. */
export const publicRoutes: IRouter = Router();

/** GET /health — API liveness check. */
publicRoutes.get('/health', (_req, res) => {
  res.json({ status: 'ok', service: 'social', version: '0.1.0' });
});
