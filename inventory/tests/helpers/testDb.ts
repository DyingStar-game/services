/**
 * In-process PostgreSQL (PGlite) standing in for `src/db/connection.ts` in tests: same
 * drizzle schema, migrated from `./drizzle`, no external database required.
 */
import { PGlite } from '@electric-sql/pglite';
import { drizzle } from 'drizzle-orm/pglite';
import { migrate } from 'drizzle-orm/pglite/migrator';

import * as schema from '../../src/db/schema/index.js';

/**
 * Creates a migrated in-memory database exposing the `connection.ts` exports.
 * @returns `db`, a `pool`-like client and `testConnection`.
 */
export async function createTestDb() {
  const client = new PGlite();
  const db = drizzle({ client, schema });
  await migrate(db, { migrationsFolder: './drizzle' });
  return { db, pool: client, testConnection: async () => true };
}
