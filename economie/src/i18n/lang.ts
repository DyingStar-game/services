/**
 * Request language: resolved from the standard `Accept-Language` header (default `en`)
 * and carried both on the request (`req.lang`, used by the error handler) and in an
 * AsyncLocalStorage (used by `t()` in deep helpers and by outbound clients).
 *
 * Kept free of imports so `messages.ts` can depend on it without a cycle.
 */
import { AsyncLocalStorage } from 'node:async_hooks';

/** Supported response languages. */
export type Lang = 'en' | 'fr';

/**
 * Picks the language from an `Accept-Language` header: client preference order, q-values
 * honoured (`q=0` rejects a tag), `fr-FR` style tags matched by prefix. Default `en`.
 * @param header - Raw `Accept-Language` header value.
 * @returns The selected language.
 */
export function resolveLang(header?: string | null): Lang {
  if (!header) return 'en';
  for (const part of header.split(',')) {
    const [rawTag, ...rest] = part.trim().split(';');
    const tag = rawTag.trim().toLowerCase();
    if (!tag || tag === '*') continue;
    let quality = 1;
    for (const param of rest) {
      const p = param.trim();
      if (p.startsWith('q=')) {
        const q = Number(p.slice(2));
        if (Number.isFinite(q)) quality = q;
      }
    }
    if (quality <= 0) continue;
    if (tag === 'fr' || tag.startsWith('fr-')) return 'fr';
    if (tag === 'en' || tag.startsWith('en-')) return 'en';
  }
  return 'en';
}

const langStorage = new AsyncLocalStorage<Lang>();

/**
 * Runs `fn` with `lang` as the ambient language; the store propagates through
 * async/await, so every helper in the request chain sees it.
 * @param lang - Language to install.
 * @param fn - Work to run.
 * @returns What `fn` returns.
 */
export function runWithLang<T>(lang: Lang, fn: () => T): T {
  return langStorage.run(lang, fn);
}

/** Ambient language of the current request, defaulting to `en`. */
export function currentLang(): Lang {
  return langStorage.getStore() ?? 'en';
}
