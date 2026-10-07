/**
 * Global Express error handler for uncaught route errors.
 *
 * When the request language is French and a catalog entry exists for the error code
 * (`frCodeMessages`), the message is rendered from it (with `HttpError.params`);
 * otherwise the original message (English source of truth) is kept.
 */
import type { NextFunction, Request, Response } from 'express';

import { localizedCodeMessage } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';

/**
 * Renders {@link HttpError} with its status, anything else as a generic 500.
 * @param err - Thrown error.
 * @param req - Express request (carries `req.lang`).
 * @param res - Express response.
 * @param _next - Express next (unused in error handlers).
 */
export function errorHandler(err: Error, req: Request, res: Response, _next: NextFunction): void {
  const message = (code: string, fallback: string, params?: Record<string, string | number>) =>
    localizedCodeMessage(req.lang, code, params) ?? fallback;

  if (err instanceof HttpError) {
    res.status(err.status).json({
      error: err.code,
      message: message(err.code, err.message, err.params),
      status: err.status,
    });
    return;
  }
  // Malformed JSON body from express.json()
  if ('type' in err && (err as { type?: string }).type === 'entity.parse.failed') {
    res.status(400).json({
      error: 'INVALID_JSON',
      message: message('INVALID_JSON', 'Malformed JSON body'),
      status: 400,
    });
    return;
  }
  console.error(err);
  res.status(500).json({
    error: 'INTERNAL_ERROR',
    message: message('INTERNAL_ERROR', err.message || 'Erreur interne du serveur'),
    status: 500,
  });
}
