/**
 * Serves a mission's texts in the request language. A mission or an objective may carry its title and
 * description per language (`titleI18n` / `descriptionI18n`, `{en, fr}`): every JSON response replaces
 * `title` / `description` with the request's language (`req.lang`, from `Accept-Language`), when the
 * map has it. Mounted once, so every route that returns missions, objectives or assignments agrees.
 */
import type { NextFunction, Request, Response } from 'express';

const PAIRS: [string, string][] = [
  ['titleI18n', 'title'],
  ['descriptionI18n', 'description'],
];

/**
 * A copy of [value] with every `title` / `description` in [lang] where its `*I18n` map has it.
 * @param value - Any JSON value.
 * @param lang - Language code (`en`, `fr`).
 * @returns The localized copy (the input is untouched).
 */
export function localize(value: unknown, lang: string): unknown {
  if (Array.isArray(value)) return value.map((v) => localize(v, lang));
  if (value === null || typeof value !== 'object' || value instanceof Date) return value;
  const out: Record<string, unknown> = {};
  for (const [key, v] of Object.entries(value as Record<string, unknown>)) out[key] = localize(v, lang);
  for (const [mapKey, textKey] of PAIRS) {
    const map = out[mapKey];
    if (map && typeof map === 'object' && typeof (map as Record<string, unknown>)[lang] === 'string') {
      out[textKey] = (map as Record<string, string>)[lang];
    }
  }
  return out;
}

/**
 * Wraps `res.json` so the body is localized to `req.lang`.
 * @param req - Express request.
 * @param res - Express response.
 * @param next - Next middleware.
 */
export function localizeResponse(req: Request, res: Response, next: NextFunction): void {
  const json = res.json.bind(res);
  res.json = (body?: unknown) => json(localize(body, req.lang ?? 'en'));
  next();
}
