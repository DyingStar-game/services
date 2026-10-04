/**
 * i18n entry point: language resolution, ambient language and message catalogs.
 */
export { currentLang, resolveLang, runWithLang, type Lang } from './lang.js';
export {
  en,
  fr,
  frCodeMessages,
  interpolate,
  localizedCodeMessage,
  t,
  type MessageKey,
} from './messages.js';
