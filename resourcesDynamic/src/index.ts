import { serve } from "@hono/node-server";
import * as dotenv from "dotenv";

import app from "./app";
import { testConnection } from "./db/connection";
import { logger } from "./lib/logger";
import { createStandaloneWebSocket } from "./websocket/server";

dotenv.config();

const WS_PORT = 9200;
const API_PORT = 3001;

// Démarrage du serveur
const start = async () => {
  logger.info("🎬 Starting Resources Dynamic Server...");

  // Test connexion DB
  const dbConnected = await testConnection();
  if (!dbConnected) {
    logger.error("❌ Failed to connect to database. Exiting...");
    process.exit(1);
  }

  // WebSocket standalone
  createStandaloneWebSocket(WS_PORT);

  // API Hono
  serve({
    fetch: app.fetch,
    port: API_PORT,
  });
};

process.on("SIGINT", () => {
  logger.info("🛑 Shutting down gracefully...");
  process.exit(0);
});

start().catch(logger.error);
