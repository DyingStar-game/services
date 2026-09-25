/**
 * Global Express error handler for uncaught route errors.
 */
import type { NextFunction, Request, Response } from 'express';

import { HttpError } from '../lib/httpError.js';

/**
 * Renders {@link HttpError} with its status, anything else as a generic 500.
 * @param err - Thrown error.
 * @param _req - Express request.
 * @param res - Express response.
 * @param _next - Express next (unused in error handlers).
 */
export function errorHandler(err: Error, _req: Request, res: Response, _next: NextFunction): void {
  if (err instanceof HttpError) {
    res.status(err.status).json({ error: err.code, message: err.message, status: err.status });
    return;
  }
  // Malformed JSON body from express.json()
  if ('type' in err && (err as { type?: string }).type === 'entity.parse.failed') {
    res.status(400).json({ error: 'INVALID_JSON', message: 'Malformed JSON body', status: 400 });
    return;
  }
  console.error(err);
  res.status(500).json({
    error: 'INTERNAL_ERROR',
    message: err.message || 'Erreur interne du serveur',
    status: 500,
  });
}