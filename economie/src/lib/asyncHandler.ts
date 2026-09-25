/**
 * Wraps an async Express handler so rejections reach the error handler (Express 4 does not do it).
 */
import type { NextFunction, Request, Response } from 'express';

type AsyncHandler = (req: Request, res: Response, next: NextFunction) => Promise<unknown>;

/**
 * @param fn - Async route handler.
 * @returns Handler forwarding rejections to `next`.
 */
export function asyncHandler(fn: AsyncHandler) {
  return (req: Request, res: Response, next: NextFunction): void => {
    fn(req, res, next).catch(next);
  };
}