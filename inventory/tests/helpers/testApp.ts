/**
 * Minimal Express app mounting the player routes behind a stub player identity: the
 * caller player comes from the `x-test-player-id` header, so ownership checks run
 * exactly as in production (JWT verification is out of scope).
 */
import type { AddressInfo } from 'net';
import type { Server } from 'http';
import express from 'express';

import { errorHandler } from '../../src/middleware/errorHandler.js';
import { meRoutes } from '../../src/routes/me.routes.js';

/** Starts the app on an ephemeral port. */
export async function startTestApp(): Promise<{ baseUrl: string; server: Server }> {
  const app = express();
  app.use(express.json());
  app.use('/api/me', (req, _res, next) => {
    req.player = {
      id: req.header('x-test-player-id') ?? '00000000-0000-0000-0000-000000000000',
      username: req.header('x-test-player-id') ?? 'test',
      roles: [],
    };
    next();
  }, meRoutes);
  app.use(errorHandler);

  const server = await new Promise<Server>((resolve) => {
    const listening = app.listen(0, () => resolve(listening));
  });
  const { port } = server.address() as AddressInfo;
  return { baseUrl: `http://127.0.0.1:${port}`, server };
}
