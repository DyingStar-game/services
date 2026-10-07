/**
 * Bilingual markers inside JSON Schema fragments served to mission builders.
 *
 * `paramsJsonSchema` descriptions are display text: they are written once as an
 * `I18nText` marker through `d(en, fr)` and resolved to a plain string (ambient request
 * language) by `localizeJsonSchema` when the catalogue is serialized — so the HTTP
 * response stays a standard JSON Schema (`description: string`).
 *
 * Convention: `{name}` placeholders, identical in both languages; technical identifiers
 * never translated.
 */
import { currentLang, type Lang } from '../i18n/index.js';
import type { JsonSchema } from './types.js';

/** Marker object standing in for a `description` until serialization. */
export interface I18nText {
  __i18n: { en: string; fr: string };
}

/**
 * Bilingual description marker.
 * @param en - English text (source of truth).
 * @param fr - French text.
 * @returns Marker to embed anywhere in a JSON Schema fragment.
 */
export function d(en: string, fr: string): I18nText {
  return { __i18n: { en, fr } };
}

/** Whether a value is a bilingual marker. */
function isI18nText(value: unknown): value is I18nText {
  return (
    typeof value === 'object' &&
    value !== null &&
    '__i18n' in value &&
    typeof (value as I18nText).__i18n === 'object'
  );
}

/**
 * Deep-copies a JSON Schema fragment, replacing every `d()` marker with the string of the
 * requested language (default: ambient request language). The input is never mutated.
 * @param node - JSON Schema fragment (possibly containing markers).
 * @param lang - Language to resolve with (default: ambient).
 * @returns A plain, marker-free JSON Schema fragment.
 */
export function localizeJsonSchema(node: JsonSchema, lang: Lang = currentLang()): JsonSchema {
  const resolve = (value: unknown): unknown => {
    if (isI18nText(value)) return lang === 'fr' ? value.__i18n.fr : value.__i18n.en;
    if (Array.isArray(value)) return value.map(resolve);
    if (typeof value === 'object' && value !== null) {
      const out: Record<string, unknown> = {};
      for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
        out[key] = resolve(item);
      }
      return out;
    }
    return value;
  };
  return resolve(node) as JsonSchema;
}
