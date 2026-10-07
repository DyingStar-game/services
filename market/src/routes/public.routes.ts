/**
 * Public API routes (no authentication).
 */
import { Router, type IRouter } from 'express';

import { schemaReady } from '../db/migrate.js';
import { serveOpenapi } from '../lib/openapi.js';

/** Router mounted at `/api` for unauthenticated endpoints. */
export const publicRoutes: IRouter = Router();

/**
 * GET /health — API liveness check.
 * `?deep=1` also verifies the database schema: a probe configured with it restarts a pod
 * whose tables have disappeared (migrations never applied, database wiped under a running pod).
 */
publicRoutes.get('/health', async (req, res) => {
  const payload: Record<string, unknown> = {
    status: 'ok',
    service: 'market',
    version: '0.1.0',
    openapi: '/openapi.yaml',
  };
  if (req.query.deep === '1') {
    const ready = await schemaReady();
    payload.database = ready ? 'ok' : 'missing';
    if (!ready) {
      payload.status = 'unavailable';
      res.status(503).json(payload);
      return;
    }
  }
  res.json(payload);
});

/** GET /openapi.yaml — This service's OpenAPI document. */
publicRoutes.get('/openapi.yaml', serveOpenapi);
