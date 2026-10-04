/**
 * Message catalogs (market).
 *
 * `frCodeMessages`: French override keyed by error code (English stays the thrown
 * message), rendered by the error handler for `Accept-Language: fr`. `en`/`fr`: paired
 * templates for construction-time messages through `t()`.
 *
 * Convention: `{name}` placeholders are identical in both languages; `interpolate` only
 * replaces placeholders present in `params`. Technical identifiers are never translated.
 */
import { currentLang, type Lang } from './lang.js';

/** English source templates for construction-time messages (`t()`). */
export const en = {
  "not_found.trade": "Trade {id} not found",
  "not_found.good": "Unknown good type {goodType}",
  "not_found.order": "Order {id} not found",
  "not_found.demand": "Demand {id} not found",
  "conflict.order_state": "Order {id} is {status}",
  "conflict.demand_state": "Demand {id} is {status}",
  "forbidden.no_ownership": "You do not own this {label}",
  "holder.buyer": "Buyer holder type '{holder}' cannot settle money",
  "holder.seller": "Seller holder type '{holder}' cannot settle money",
  "auth.missing_bearer": "Missing bearer token",
  "auth.bad_subject": "Token has no usable subject",
  "auth.invalid_token": "Invalid token",
  "auth.missing_service_token": "Missing service token",
  "auth.invalid_service_token": "Invalid service token",
  "auth.not_service": "Not authenticated as a service",
  "auth.not_authenticated": "Not authenticated",
  "auth.requires_role": "Requires role {role}",
  "auth.client_forbidden": "Client {clientId} is not an allowed service",
  "auth.service_bad_subject": "Service token has no usable subject",
  "auth.service_account_token": "Token is not issued to a service account",
  "auth.requires_service_role": "Requires service role {role}",
} as const;

/** Keys of the construction-time catalog (typed from the English source). */
export type MessageKey = keyof typeof en & string;

/** French pairs — TypeScript enforces full coverage of `en`. */
export const fr: Record<MessageKey, string> = {
  "not_found.trade": "Échange {id} introuvable",
  "not_found.good": "Type de bien inconnu {goodType}",
  "not_found.order": "Ordre {id} introuvable",
  "not_found.demand": "Demande {id} introuvable",
  "conflict.order_state": "L'ordre {id} est {status}",
  "conflict.demand_state": "La demande {id} est {status}",
  "forbidden.no_ownership": "Vous ne possédez pas cet élément ({label})",
  "holder.buyer": "Le type de détenteur acheteur '{holder}' ne peut pas régler d'argent",
  "holder.seller": "Le type de détenteur vendeur '{holder}' ne peut pas régler d'argent",
  "auth.missing_bearer": "Token Bearer manquant",
  "auth.bad_subject": "Le token n'a pas de sujet exploitable",
  "auth.invalid_token": "Token invalide",
  "auth.missing_service_token": "Token de service manquant",
  "auth.invalid_service_token": "Token de service invalide",
  "auth.not_service": "Authentification de service manquante",
  "auth.not_authenticated": "Non authentifié",
  "auth.requires_role": "Nécessite le rôle {role}",
  "auth.client_forbidden": "Le client {clientId} n'est pas un service autorisé",
  "auth.service_bad_subject": "Le token de service n'a pas de sujet exploitable",
  "auth.service_account_token": "Le token n'est pas émis pour un compte de service",
  "auth.requires_service_role": "Nécessite le rôle de service {role}",
};

/** French overrides keyed by error code (English remains the thrown message). */
export const frCodeMessages: Record<string, string> = {
  "ECONOMY_AUTH_FAILED": "Échec d'authentification Economy ({status}) : {body}",
  "ECONOMY_NOT_CONFIGURED": "Le service Economy n'est pas configuré",
  "INVENTORY_AUTH_FAILED": "Échec d'authentification Inventory ({status}) : {body}",
  "INVENTORY_CALL_FAILED": "Échec de l'appel Inventory {method} {path} ({status}) : {message}",
  "INVENTORY_NOT_CONFIGURED": "Le service Inventory n'est pas configuré",
  "SOCIAL_AUTH_FAILED": "Échec d'authentification Social ({status}) : {body}",
  "SOCIAL_LOOKUP_FAILED": "Échec de l'interrogation de Social ({status}) : {body}",
  "SOCIAL_NOT_CONFIGURED": "Le service Social n'est pas configuré",
  "GOOD_DISABLED": "Le type de bien {goodType} n'est pas échangeable",
  "GOOD_KIND_MISMATCH": "Le type de bien {goodType} est un {actual}, pas {expected}",
  "INSTANCE_REQUIRED": "instanceId est requis pour un ordre de vente d'instance",
  "INVALID_SIDE": "Les ordres d'achat ne concernent que les biens fongibles ; utilisez une demande pour les instances",
  "PRICE_ABOVE_MAX": "Le prix unitaire {price} dépasse le prix max de la demande {max}",
  "NOT_CORPORATION_MEMBER": "Vous n'êtes pas membre de cette corporation",
  "INVALID_JSON": "Corps JSON malformé",
  "INTERNAL_ERROR": "Erreur interne du serveur",
};

/**
 * Replaces `{name}` placeholders with the given params; unknown placeholders stay literal.
 * @param template - Message template.
 * @param params - Values to interpolate.
 * @returns Rendered message.
 */
export function interpolate(template: string, params?: Record<string, string | number>): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.hasOwn(params, name) ? String(params[name]) : match,
  );
}

/**
 * Localized message for a construction-time key (ambient request language, default `en`).
 * @param key - Message key.
 * @param params - Values to interpolate.
 * @returns Rendered message.
 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const template = (currentLang() === 'fr' ? fr[key] : en[key]) ?? en[key];
  return interpolate(template, params);
}

/**
 * French override for an error code, or null when there is none (or the request is not
 * French). Used by the error handler.
 * @param lang - Request language (`req.lang`).
 * @param code - Error code.
 * @param params - Interpolation params (`HttpError.params`).
 * @returns French message, or null.
 */
export function localizedCodeMessage(
  lang: Lang | undefined,
  code: string,
  params?: Record<string, string | number>,
): string | null {
  if (lang !== 'fr') return null;
  const template = frCodeMessages[code];
  return template ? interpolate(template, params) : null;
}
