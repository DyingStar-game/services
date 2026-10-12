/**
 * Minimal Express app mounting the internal router behind a stub service identity: the
 * calling client and its roles come from the `x-test-client` / `x-test-roles` headers, so
 * role checks run exactly as in production (Keycloak verification is out of scope).
 */
import type { AddressInfo } from 'net';
import type { Server } from 'http';
import express from 'express';

import { errorHandler } from '../../src/middleware/errorHandler.js';
import { languageMiddleware } from '../../src/middleware/language.js';
import { internalRoutes } from '../../src/routes/internal.routes.js';

/** Starts the app on an ephemeral port. */
export async function startTestApp(): Promise<{ baseUrl: string; server: Server }> {
  const app = express();
  app.use(languageMiddleware);
  app.use(express.json());
  app.use('/api/internal', (req, _res, next) => {
    req.service = {
      clientId: req.header('x-test-client') ?? 'svc-universesimulator',
      subject: '00000000-0000-0000-0000-000000000000',
      roles: (req.header('x-test-roles') ?? '').split(',').map((r) => r.trim()).filter(Boolean),
    };
    next();
  }, internalRoutes);
  app.use(errorHandler);

  const server = await new Promise<Server>((resolve) => {
    const listening = app.listen(0, () => resolve(listening));
  });
  const { port } = server.address() as AddressInfo;
  return { baseUrl: `http://127.0.0.1:${port}`, server };
}
