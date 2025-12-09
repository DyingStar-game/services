import { apiLogger } from "@lib/logger";
import type { Context, Next } from "hono";

/**
 * Middleware de logging Hono avec Pino
 */
export const loggerMiddleware = async (c: Context, next: Next) => {
  const start = Date.now();
  const method = c.req.method;
  const path = c.req.path;

  // Log requête entrante (optionnel)
  apiLogger.debug({ method, path }, `→ ${method} ${path}`);

  await next();

  const duration = Date.now() - start;
  const status = c.res.status;

  // Déterminer le niveau de log
  if (status >= 500) {
    apiLogger.error(
      { method, path, status, duration },
      `❌ ${method} ${path} ${status} ${duration}ms`,
    );
  } else if (status >= 400) {
    apiLogger.warn(
      { method, path, status, duration },
      `⚠️  ${method} ${path} ${status} ${duration}ms`,
    );
  } else {
    apiLogger.info(
      { method, path, status, duration },
      `✅ ${method} ${path} ${status} ${duration}ms`,
    );
  }
};
