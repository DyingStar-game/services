/**
 * Request validation middleware backed by zod schemas.
 */
import type { NextFunction, Request, Response } from 'express';
import type { ZodType } from 'zod';

import { HttpError } from '../lib/httpError.js';

type Part = 'body' | 'query' | 'params';

/**
 * Validates `req[part]` against `schema` and replaces it with the parsed value.
 * @param schema - Zod schema.
 * @param part - Request part to validate (default `body`).
 * @returns Middleware responding 400 `VALIDATION_ERROR` on failure.
 */
export function validate(schema: ZodType, part: Part = 'body') {
  return (req: Request, _res: Response, next: NextFunction): void => {
    const result = schema.safeParse(req[part]);
    if (!result.success) {
      const issues = result.error.issues
        .map((i) => `${i.path.join('.') || part}: ${i.message}`)
        .join('; ');
      next(new HttpError(400, 'VALIDATION_ERROR', issues));
      return;
    }
    if (part === 'query') {
      // req.query is a getter in Express 4; keep parsed values reachable without reassigning it.
      Object.assign(req.query, result.data as object);
    } else {
      req[part] = result.data as never;
    }
    next();
  };
}