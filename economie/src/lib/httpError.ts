/**
 * Error carrying an HTTP status and a machine-readable code, rendered by the error handler.
 */
export class HttpError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = 'HttpError';
  }
}

/** 404 helper. */
export function notFound(message: string): HttpError {
  return new HttpError(404, 'NOT_FOUND', message);
}

/** 409 helper. */
export function conflict(message: string): HttpError {
  return new HttpError(409, 'CONFLICT', message);
}

/** 403 helper. */
export function forbidden(message: string): HttpError {
  return new HttpError(403, 'FORBIDDEN', message);
}