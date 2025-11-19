import { Hono } from "hono";
import { cors } from "hono/cors";

import { loggerMiddleware } from "./middleware/logger";
import apiRoutes from "./routes/api";

const app = new Hono();

// Middlewares
app.use("*", loggerMiddleware);
app.use("*", cors());

// Routes API
app.route("/api", apiRoutes);

export default app;
