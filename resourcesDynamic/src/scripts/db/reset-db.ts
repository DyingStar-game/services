import { drizzle } from "drizzle-orm/node-postgres";
import { Client } from "pg";

// Paramètres de ta connexion PostgreSQL
const client = new Client({
  connectionString: process.env.DATABASE_URL,
});

async function resetDb() {
  await client.connect();
  const db = drizzle(client);

  // Script SQL tel que tu l’as donné
  const resetSQL = `
    DROP SCHEMA IF EXISTS drizzle CASCADE;

    DO $$
    DECLARE
      r RECORD;
    BEGIN
      FOR r IN (SELECT tablename FROM pg_tables WHERE schemaname = 'public') LOOP
        EXECUTE 'DROP TABLE IF EXISTS public.' || quote_ident(r.tablename) || ' CASCADE';
      END LOOP;
    END
    $$;

    DO $$
    DECLARE
      r RECORD;
    BEGIN
      FOR r IN (SELECT sequencename FROM pg_sequences WHERE schemaname = 'public') LOOP
        EXECUTE 'DROP SEQUENCE IF EXISTS public.' || quote_ident(r.sequencename) || ' CASCADE';
      END LOOP;
    END
    $$;

    DO $$
    DECLARE
      r RECORD;
    BEGIN
      FOR r IN (SELECT table_name FROM information_schema.views WHERE table_schema = 'public') LOOP
        EXECUTE 'DROP VIEW IF EXISTS public.' || quote_ident(r.table_name) || ' CASCADE';
      END LOOP;
    END
    $$;
  `;

  await db.execute(resetSQL);
  await client.end();
}

resetDb()
  .then(() => console.log("Database reset !"))
  .catch((err) => {
    console.error("Erreur lors du reset:", err);
    process.exit(1);
  });
