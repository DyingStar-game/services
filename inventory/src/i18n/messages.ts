/**
 * Message catalogs (inventory).
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
  "not_found.stack": "No stack of {goodType} for this holder",
  "not_found.stack_source": "No stack of {goodType} for the source holder",
  "not_found.instance_holder": "Instance {id} not owned by this holder",
  "not_found.instance_source": "Instance {id} not owned by the source holder",
  "not_found.hold": "Hold {id} not found",
  "not_found.instance_hold": "Instance {id} is not owned by the hold holder",
  "conflict.good_kind": "Good type {goodType} is registered as {registered}, not {expected}",
  "conflict.instance_other": "Instance {id} already belongs to another holder",
  "conflict.instance_already_held": "Instance {id} is already held",
  "conflict.instance_is_held": "Instance {id} is held",
  "conflict.hold_state": "Hold {id} is {status}",
  "goods.insufficient": "Only {available} {goodType} available",
  "goods.held_gone": "Held goods are no longer available",
  "quantity.positive": "Quantity must be positive",
  "quantity.stack_positive": "quantity must be positive for stack holds",
  "internal.good_type": "Failed to register good type {goodType}",
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
  "not_found.stack": "Aucune pile de {goodType} pour ce détenteur",
  "not_found.stack_source": "Aucune pile de {goodType} pour le détenteur source",
  "not_found.instance_holder": "Instance {id} pas détenue par ce détenteur",
  "not_found.instance_source": "Instance {id} pas détenue par le détenteur source",
  "not_found.hold": "Réservation {id} introuvable",
  "not_found.instance_hold": "Instance {id} n'est pas détenue par le détenteur de la réservation",
  "conflict.good_kind": "Le type de bien {goodType} est enregistré comme {registered}, pas {expected}",
  "conflict.instance_other": "L'instance {id} appartient déjà à un autre détenteur",
  "conflict.instance_already_held": "L'instance {id} est déjà réservée",
  "conflict.instance_is_held": "L'instance {id} est réservée",
  "conflict.hold_state": "La réservation {id} est {status}",
  "goods.insufficient": "Seulement {available} {goodType} disponible(s)",
  "goods.held_gone": "Les biens réservés ne sont plus disponibles",
  "quantity.positive": "La quantité doit être positive",
  "quantity.stack_positive": "la quantité doit être positive pour les réservations de pile",
  "internal.good_type": "Échec d'enregistrement du type de bien {goodType}",
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
  "INSTANCE_REQUIRED": "instanceId est requis pour les réservations d'instance",
  "REF_REQUIRED": "refId est requis pour les réservations non manuelles",
  "NOT_CORPORATION_MEMBER": "Vous n'êtes pas membre de cette corporation",
  "SOCIAL_AUTH_FAILED": "Échec d'authentification Social ({status}) : {body}",
  "SOCIAL_LOOKUP_FAILED": "Échec de l'interrogation de Social ({status}) : {body}",
  "SOCIAL_NOT_CONFIGURED": "Le service Social n'est pas configuré",
  "INVALID_JSON": "Corps JSON malformé",
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
