import { Hono } from "hono";

import { getClientsCount } from "../websocket/handlers";
import planetsRouter from "./planets";
import systemsRouter from "./systems";

const api = new Hono();

// Routes
api.route("/planets", planetsRouter);
api.route("/systems", systemsRouter);

// Health check
api.get("/health", (c) =>
  c.json({
    status: "ok",
    timestamp: new Date().toISOString(),
    websocket: {
      connected: getClientsCount(),
    },
  }),
);

export default api;
