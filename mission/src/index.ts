/**
 * DyingStar Mission API entrypoint — Express app, middleware, migrations and HTTP listener.
 */
import cors from 'cors';
import express from 'express';
import morgan from 'morgan';

import { env } from './config/env.js';
import { testConnection } from './db/connection.js';
import { runMigrations } from './db/migrate.js';
import { serviceAuth } from './middleware/auth.js';
import { errorHandler } from './middleware/errorHandler.js';
import { apiRouter } from './routes/index.js';
import { internalRoutes } from './routes/internal.routes.js';

const app = express();

app.use(morgan(env.nodeEnv === 'production' ? 'combined' : 'dev'));
app.use(cors({ origin: env.corsOrigin, credentials: true }));
app.use(express.json({ limit: '1mb' }));

/** Root discovery endpoint. */
app.get('/', (_req, res) => {
  res.json({ name: 'DyingStar Mission API', health: '/api/health' });
});

// The service guard sits on the mount point, not on the inner route list: any route
// added later under /api/internal is protected without touching the router.
app.use('/api/internal', serviceAuth, internalRoutes);

app.use('/api', apiRouter);

app.use((_req, res) => {
  res.status(404).json({ error: 'NOT_FOUND', message: 'Route not found', status: 404 });
});
app.use(errorHandler);

async function start(): Promise<void> {
  if (env.nodeEnv === 'production' && process.env.INTERNAL_DEV_BYPASS === 'true') {
    console.error('INTERNAL_DEV_BYPASS must never be enabled in production. Exiting...');
    process.exit(1);
  }

  if (!(await testConnection())) {
    console.error('Failed to connect to database. Exiting...');
    process.exit(1);
  }
  await runMigrations();
  console.log('Database migrations applied');

  if (env.authDevBypass) {
    console.warn('AUTH_DEV_BYPASS is enabled: X-Player-Id headers are trusted without a JWT');
  }
  if (env.internalDevBypass) {
    console.warn('INTERNAL_DEV_BYPASS is enabled: X-Internal-Key is accepted on /api/internal/*');
  } else if (env.internalApiKey) {
    console.warn('INTERNAL_API_KEY is set but ignored: only service-account JWTs are accepted');
  }
  if (env.internal.serviceClients.length === 0) {
    console.warn('INTERNAL_SERVICE_CLIENTS is empty: every service-account token is rejected with 403');
  }
  if (!env.economy.serviceClientSecret && !env.economy.internalApiKey) {
    console.warn('No Economy credentials: reward settlement will fail until ECONOMY_SERVICE_CLIENT_SECRET is set');
  }

  app.listen(env.port, () => {
    console.log(`DyingStar Mission API listening on http://localhost:${env.port}`);
  });
}

process.on('SIGINT', () => process.exit(0));
process.on('SIGTERM', () => process.exit(0));

start().catch((err) => {
  console.error(err);
  process.exit(1);
});
