/**
 * PostgreSQL connection pool and drizzle database handle.
 */
import { drizzle } from 'drizzle-orm/node-postgres';
import pg from 'pg';

import { env } from '../config/env.js';
import * as schema from './schema/index.js';

if (!env.databaseUrl) {
  throw new Error('DATABASE_URL must be set');
}

/** Shared pg pool (also used by the migrator). */
export const pool = new pg.Pool({ connectionString: env.databaseUrl });

/** Drizzle handle with the full schema registered. */
export const db = drizzle({ client: pool, schema });

export type Db = typeof db;

/**
 * Runs a trivial query to confirm the database is reachable.
 * @returns True when `select 1` succeeds.
 */
export async function testConnection(): Promise<boolean> {
  try {
    await db.execute('select 1');
    return true;
  } catch (err) {
    console.error('Database connection failed:', err);
    return false;
  }
}
