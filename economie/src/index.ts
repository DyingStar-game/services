/**
 * DyingStar Economie API entrypoint — Express app, middleware, migrations and HTTP listener.
 */
import cors from 'cors';
import express from 'express';
import morgan from 'morgan';

import { env } from './config/env.js';
import { testConnection } from './db/connection.js';
import { runMigrations } from './db/migrate.js';
import { errorHandler } from './middleware/errorHandler.js';
import { apiRouter } from './routes/index.js';

const app = express();

app.use(morgan(env.nodeEnv === 'production' ? 'combined' : 'dev'));
app.use(cors({ origin: env.corsOrigin, credentials: true }));
app.use(express.json({ limit: '1mb' }));

/** Root discovery endpoint. */
app.get('/', (_req, res) => {
  res.json({ name: 'DyingStar Economie API', health: '/api/health' });
});

app.use('/api', apiRouter);

app.use((_req, res) => {
  res.status(404).json({ error: 'NOT_FOUND', message: 'Route not found', status: 404 });
});
app.use(errorHandler);

async function start(): Promise<void> {
  if (!(await testConnection())) {
    console.error('Failed to connect to database. Exiting...');
    process.exit(1);
  }
  await runMigrations();
  console.log('Database migrations applied');

  if (env.authDevBypass) {
    console.warn('AUTH_DEV_BYPASS is enabled: X-Player-Id headers are trusted without a JWT');
  }
  if (!env.internalApiKey) {
    console.warn('INTERNAL_API_KEY is empty: /api/internal/* will answer 503');
  }

  app.listen(env.port, () => {
    console.log(`DyingStar Economie API listening on http://localhost:${env.port}`);
  });
}

process.on('SIGINT', () => process.exit(0));
process.on('SIGTERM', () => process.exit(0));

start().catch((err) => {
  console.error(err);
  process.exit(1);
});