import { apiLogger, logRequestError } from "@lib/logger";
import { DrizzleQueryError } from "drizzle-orm";
import type { Context } from "hono";
import { ZodError } from "zod";

export const errorHandler = (error: Error, c: Context) => {
  logRequestError(apiLogger, c, error);

  if (error instanceof ZodError) {
    return c.json(
      {
        success: false,
        error: "Validation failed",
        details: error.issues,
      },
      400,
    );
  }

  if (error instanceof DrizzleQueryError) {
    return c.json(
      {
        success: false,
        error: "Database operation failed",
        message: error.cause?.message,
        cause: error.cause,
      },
      400,
    );
  }

  return c.json(
    {
      success: false,
      error: "An unexpected error occurred",
      message: error.message,
      cause: error.cause,
    },
    500,
  );
};
