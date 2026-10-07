/**
 * Message catalogs (economie).
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
  "not_found.corp_member": "Player {id} is not a member of this corporation",
  "not_found.account": "Account {id} not found",
  "not_found.treasury": "Treasury account for {corporationId} in {currency} not found",
  "not_found.labeled_account": "{label} account {id} not found",
  "tax.insufficient": "Insufficient balance to settle any due tax debt",
  "insufficient.plain": "Insufficient balance",
  "treasury.insufficient": "Insufficient treasury balance",
  "amount.positive": "Amount must be positive",
  "amount.max": "Maximum transfer is {max}",
  "amount.min": "Minimum transfer is {min}",
  "target.same_account": "Cannot transfer to the same account",
  "target.self": "You cannot send credits to yourself",
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
  "corp.requires_role": "Requires the {min} role in this corporation",
  "corp.requires_permission": "Requires the {action} permission in this corporation",
} as const;

/** Keys of the construction-time catalog (typed from the English source). */
export type MessageKey = keyof typeof en & string;

/** French pairs — TypeScript enforces full coverage of `en`. */
export const fr: Record<MessageKey, string> = {
  "not_found.corp_member": "Le joueur {id} n'est pas membre de cette corporation",
  "not_found.account": "Compte {id} introuvable",
  "not_found.treasury": "Compte trésorerie pour {corporationId} en {currency} introuvable",
  "not_found.labeled_account": "Compte {label} {id} introuvable",
  "tax.insufficient": "Solde insuffisant pour régler une dette fiscale due",
  "insufficient.plain": "Solde insuffisant",
  "treasury.insufficient": "Solde de trésorerie insuffisant",
  "amount.positive": "Le montant doit être positif",
  "amount.max": "Le transfert maximum est {max}",
  "amount.min": "Le transfert minimum est {min}",
  "target.same_account": "Impossible de transférer vers le même compte",
  "target.self": "Vous ne pouvez pas vous envoyer de crédits vous-même",
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
  "corp.requires_role": "Nécessite le rôle {min} dans cette corporation",
  "corp.requires_permission": "Nécessite la permission {action} dans cette corporation",
};

/** French overrides keyed by error code (English remains the thrown message). */
export const frCodeMessages: Record<string, string> = {
  "ACCOUNT_LOCKED": "Le compte est {status}",
  "DONATIONS_DISABLED": "Les donations sont désactivées pour cette corporation",
  "DUPLICATE_EXTERNAL_ID": "Transaction déjà enregistrée",
  "MINTING_DISABLED": "Cette entité politique n'est pas autorisée à créer de monnaie",
  "MINT_CEILING_EXCEEDED": "Le montant dépasse le plafond de frappe ({ceiling})",
  "NOT_CORPORATION_MEMBER": "Vous n'êtes pas membre de cette corporation",
  "SOCIAL_AUTH_FAILED": "Échec d'authentification Social ({status}) : {body}",
  "SOCIAL_LOOKUP_FAILED": "Échec de l'interrogation de Social ({status}) : {body}",
  "SOCIAL_NOT_CONFIGURED": "Le service Social n'est pas configuré",
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
