import { DrizzleQueryError } from "drizzle-orm";
import * as fs from "fs";
import type { Context } from "hono";
import * as path from "path";
import pino from "pino";
import { ZodError } from "zod";

// Détection de l'environnement
const isDev = process.env.NODE_ENV !== "production";
const isTest = process.env.NODE_ENV === "test";

// ===================================
// Configuration Pino
// ===================================

let baseLogger: pino.Logger;

if (!isDev && !isTest) {
  // En production : logs vers console + fichiers
  const logsDir = path.join(process.cwd(), "logs");

  // Créer le dossier logs s'il n'existe pas
  if (!fs.existsSync(logsDir)) {
    fs.mkdirSync(logsDir, { recursive: true });
  }

  const dateStr = new Date().toISOString().split("T")[0];

  // Stream pour tous les logs
  const allLogsStream = fs.createWriteStream(
    path.join(logsDir, `app-${dateStr}.log`),
    { flags: "a" },
  );

  // Stream pour les erreurs uniquement
  const errorLogsStream = fs.createWriteStream(
    path.join(logsDir, `error-${dateStr}.log`),
    { flags: "a" },
  );

  // Logger avec plusieurs destinations
  baseLogger = pino(
    {
      level: "info",
      formatters: {
        level: (label) => {
          return { level: label.toUpperCase() };
        },
      },
    },
    pino.multistream([
      { stream: process.stdout }, // Console
      { stream: allLogsStream }, // Tous les logs
      { level: "error", stream: errorLogsStream }, // Erreurs uniquement
    ]),
  );

  const closeStreams = () => {
    allLogsStream.end();
    errorLogsStream.end();
  };

  process.on("beforeExit", closeStreams);
  process.on("SIGINT", closeStreams);
  process.on("SIGTERM", closeStreams);
} else {
  baseLogger = pino({
    level: isDev ? "trace" : "info",
    transport: {
      target: "pino-pretty",
      options: {
        colorize: true,
        translateTime: "HH:mm:ss.l",
        ignore: "pid,hostname",
        singleLine: false,
        messageFormat: "{component} {msg}",
      },
    },
  });
}

// ===================================
// Sous-loggers avec composants
// ===================================

export const logger = baseLogger.child({ component: "App" });

export const wsLogger = baseLogger.child({ component: "🔌 WebSocket" });

export const duckDbLogger = baseLogger.child({ component: "🦆 DuckDB" });

export const pgDbLogger = baseLogger.child({ component: "🐘 Postgres" });

export const apiLogger = baseLogger.child({ component: "🌐 API" });

export const cacheLogger = baseLogger.child({ component: "💾 Cache" });

export const cacheTransformLogger = baseLogger.child({ component: "💾 Cache" });

export const keplerOrbitLogger = baseLogger.child({
  component: "🪐 KeplerOrbit",
});

export const keplerOrbitServiceLogger = baseLogger.child({
  component: "🚀 KeplerOrbitService",
});

// ===================================
// Helpers pour logs structurés
// ===================================

/**
 * Formatte une ZodError en structure lisible
 */
const formatZodError = (error: ZodError) => {
  return {
    type: "ZodError",
    issues: error.issues.map((issue) => ({
      path: issue.path.join("."),
      message: issue.message,
      code: issue.code,
      expected: "expected" in issue ? issue.expected : undefined,
      received: "received" in issue ? issue.received : undefined,
    })),
    summary: error.issues
      .map((issue) => `${issue.path.join(".")}: ${issue.message}`)
      .join(" | "),
  };
};

/**
 * Formatte une erreur Drizzle ORM
 */
const formatDrizzleQueryError = (error: DrizzleQueryError) => {
  return {
    type: "DrizzleQueryError",
    name: error.name,
    cause: error.cause,
  };
};

/**
 * Log une query DB avec durée
 */
export const logQuery = (
  logger: pino.Logger,
  query: string,
  duration: number,
  rowCount?: number,
) => {
  const truncatedQuery =
    query.length > 100 ? `${query.substring(0, 100)}...` : query;

  logger.debug(
    {
      query: truncatedQuery,
      duration,
      rowCount,
    },
    `Query executed in ${duration}ms`,
  );
};

const getRequestContext = (c: Context): Record<string, unknown> => {
  return {
    method: c.req.method,
    path: c.req.path,
    url: c.req.url,
    userAgent: c.req.header("user-agent"),
    ip: c.req.header("x-forwarded-for") || c.req.header("x-real-ip"),
  };
};

export const logRequestError = (
  logger: pino.Logger,
  c: Context,
  error: unknown,
  additionalContext?: Record<string, unknown>,
) => {
  const requestContext = getRequestContext(c);
  logError(logger, error, {
    ...requestContext,
    ...additionalContext,
  });
};

/**
 * Log une erreur avec contexte enrichi
 * Gère automatiquement ZodError, DrizzleError, DatabaseError, et Error standard
 */
export const logError = (
  logger: pino.Logger,
  error: unknown,
  context?: Record<string, unknown>,
) => {
  if (error instanceof ZodError) {
    const formattedError = formatZodError(error);

    logger.error(
      {
        error: formattedError,
        ...context,
      },
      `Validation error: ${formattedError.summary}`,
    );
    return;
  }

  if (error instanceof DrizzleQueryError) {
    const formattedError = formatDrizzleQueryError(error);

    logger.error(
      {
        error: formattedError,
        ...context,
      },
      `Drizzle error: ${formattedError.cause}`,
    );
    return;
  }

  if (error instanceof Error) {
    logger.error(
      {
        error: {
          name: error.name,
          message: error.message,
          stack: isDev ? error.stack : undefined,
        },
        ...context,
      },
      error.message,
    );
    return;
  }

  logger.error(
    {
      error: {
        type: typeof error,
        value: error,
      },
      ...context,
    },
    String(error),
  );
};

/**
 * Log performance (pour benchmarks)
 */
export const logPerformance = (
  logger: pino.Logger,
  operation: string,
  duration: number,
  metadata?: Record<string, unknown>,
) => {
  const level = duration > 1000 ? "warn" : duration > 100 ? "info" : "debug";

  logger[level](
    {
      operation,
      duration,
      ...metadata,
    },
    `⏱️  ${operation} took ${duration.toFixed(3)}ms`,
  );
};

// ===================================
// Utilitaires
// ===================================

/**
 * Crée un timer pour mesurer la durée
 */
export const createTimer = () => {
  const start = performance.now();
  return {
    end: () => Math.round(performance.now() - start),
  };
};

/**
 * Middleware pour logger toutes les requêtes non-catchées
 */
export const setupGlobalErrorHandlers = () => {
  process.on("uncaughtException", (error: Error) => {
    logger.fatal(
      {
        error: {
          name: error.name,
          message: error.message,
          stack: error.stack,
        },
      },
      "Uncaught Exception",
    );
    process.exit(1);
  });

  process.on("unhandledRejection", (reason: unknown) => {
    logger.fatal(
      {
        reason: String(reason),
      },
      "Unhandled Promise Rejection",
    );
    process.exit(1);
  });
};
