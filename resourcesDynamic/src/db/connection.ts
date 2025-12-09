import { logError, pgDbLogger } from "@lib/logger";
import * as dotenv from "dotenv";
import { drizzle } from "drizzle-orm/postgres-js";

import * as schema from "./schema";

dotenv.config();

const DATABASE_URL = process.env.DATABASE_URL;

if (!DATABASE_URL) {
  throw new Error("DATABASE_URL must be set in environment variables");
}

export const db = drizzle({ connection: DATABASE_URL, schema });

export const testConnection = async () => {
  try {
    await db.execute("select 1");
    pgDbLogger.info("✅ Database connected successfully");
    return true;
  } catch (error) {
    logError(pgDbLogger, error, { context: "testConnection" });
    return false;
  }
};
