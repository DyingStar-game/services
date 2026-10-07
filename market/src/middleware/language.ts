/**
 * Resolves the request language from `Accept-Language` and installs it both on
 * `req.lang` (error handler) and as the ambient language of the async chain (`t()`,
 * outbound clients). Mounted first, before every route.
 */
import type { NextFunction, Request, Response } from 'express';

import { resolveLang, runWithLang } from '../i18n/index.js';

/**
 * Attaches the request language and runs the rest of the chain under it.
 * @param req - Express request.
 * @param _res - Express response.
 * @param next - Next middleware.
 */
export function languageMiddleware(req: Request, _res: Response, next: NextFunction): void {
  const lang = resolveLang(req.header('accept-language'));
  req.lang = lang;
  runWithLang(lang, () => next());
}
