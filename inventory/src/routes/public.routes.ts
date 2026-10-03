/**
 * Public API routes (no authentication).
 */
import { Router, type IRouter } from 'express';

import { serveOpenapi } from '../lib/openapi.js';

/** Router mounted at `/api` for unauthenticated endpoints. */
export const publicRoutes: IRouter = Router();

/** GET /health — API liveness check. */
publicRoutes.get('/health', (_req, res) => {
  res.json({ status: 'ok', service: 'inventory', version: '0.1.0', openapi: '/openapi.yaml' });
});

/** GET /openapi.yaml — This service's OpenAPI document. */
publicRoutes.get('/openapi.yaml', serveOpenapi);
