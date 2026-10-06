/**
 * Drops this service's own database objects so the next start applies `drizzle/0000_init.sql`
 * from a clean slate. **Schema only: every table, sequence and view of `public` is deleted**
 * (the `drizzle` migrations schema goes too).
 *
 *   pnpm db:reset -- --yes
 *
 * `DATABASE_URL` is read from the environment or `.env`. The script refuses to run without the
 * explicit `--yes` confirmation, and refuses a database that also holds the sentinel table of
 * another service (wrong target / shared database).
 */
import { config } from 'dotenv';
import pg from 'pg';

config();

const SERVICE = 'market';

/** Sentinel table of each service: any of them (but this one) means the target is wrong. */
const SENTINELS = {
  accounts: 'economie',
  market_catalog: 'market',
  inventory_good_types: 'inventory',
  missions: 'mission',
  player_profiles: 'social',
};
const OWN_SENTINEL = 'market_catalog';
delete SENTINELS[OWN_SENTINEL];

const RESET_SQL = `
  drop schema if exists drizzle cascade;

  do $$
  declare r record;
  begin
    for r in (select tablename from pg_tables where schemaname = 'public') loop
      execute 'drop table if exists public.' || quote_ident(r.tablename) || ' cascade';
    end loop;
  end
  $$;

  do $$
  declare r record;
  begin
    for r in (select sequencename from pg_sequences where schemaname = 'public') loop
      execute 'drop sequence if exists public.' || quote_ident(r.sequencename) || ' cascade';
    end loop;
  end
  $$;

  do $$
  declare r record;
  begin
    for r in (select table_name from information_schema.views where table_schema = 'public') loop
      execute 'drop view if exists public.' || quote_ident(r.table_name) || ' cascade';
    end loop;
  end
  $$;
`;

const url = process.env.DATABASE_URL ?? '';
if (!process.argv.includes('--yes')) {
  console.error(`Missing confirmation: pnpm db:reset -- --yes (wipes every table of ${SERVICE})`);
  process.exit(1);
}
if (!url) {
  console.error('DATABASE_URL must be set (environment or .env)');
  process.exit(1);
}

const client = new pg.Client({ connectionString: url });
try {
  await client.connect();

  const { rows } = await client.query(
    "select tablename from pg_tables where schemaname = 'public' and tablename = any($1::text[])",
    [Object.keys(SENTINELS)],
  );
  const foreign = rows.map((r) => SENTINELS[r.tablename]);
  if (foreign.length > 0) {
    throw new Error(
      `this database also holds tables of ${[...new Set(foreign)].join(', ')} — it is not dedicated to ${SERVICE}, wrong target?`,
    );
  }

  await client.query(RESET_SQL);
  console.log(`${SERVICE}: schema dropped — start the service to apply drizzle/0000_init.sql`);
} catch (err) {
  console.error('Reset failed:', err instanceof Error ? err.message : err);
  process.exitCode = 1;
} finally {
  await client.end();
}
