/**
 * Message catalogs (mission).
 *
 * Two mechanisms:
 * - `frCodeMessages`: French override keyed by **error code**, rendered by the error
 *   handler when the request asks for French (English stays the thrown message — the
 *   English source of truth lives at the throw site). Only codes whose every site shares
 *   a single template belong here; multi-text codes go through construction-time `t()`.
 * - `en`/`fr`: paired templates for messages built at construction time (dynamic text,
 *   not-found messages, auth messages, prerequisite details) through `t()`.
 *
 * Convention: every parameter/field reference uses `{name}` placeholders — identical in
 * both languages — and `interpolate` only replaces placeholders present in `params`
 * (summaries keep a literal `{itemId}` on purpose). Technical identifiers (kind names,
 * error codes, enum values, field names) are never translated.
 */
import { currentLang, type Lang } from './lang.js';

/** English source templates for construction-time messages (`t()`). */
export const en = {
  // ── not found ──
  'not_found.mission': 'Mission {id} not found',
  'not_found.mission_bare': 'Mission not found',
  'not_found.objective': 'Objective {id} not found on mission {missionId}',
  'not_found.group': 'Group {id} not found',
  'not_found.assignment': 'Assignment not found',
  'not_found.no_assignment': 'No assignment for mission {missionId}',
  // ── auth ──
  'auth.missing_bearer': 'Missing bearer token',
  'auth.bad_subject': 'Token has no usable subject',
  'auth.invalid_token': 'Invalid token',
  'auth.missing_service_token': 'Missing service token',
  'auth.invalid_service_token': 'Invalid service token',
  'auth.not_service': 'Not authenticated as a service',
  'auth.not_authenticated': 'Not authenticated',
  'auth.requires_role': 'Requires role {role}',
  'auth.client_forbidden': 'Client {clientId} is not an allowed service',
  'auth.service_bad_subject': 'Service token has no usable subject',
  'auth.service_account_token': 'Token is not issued to a service account',
  'auth.requires_service_role': 'Requires service role {role}',
  // ── specification ──
  'spec.unknown_prereq': 'Unknown prerequisite kind: {kind}',
  'prereq.missing_kind': 'Prerequisite kind missing from the registry: {kind}',
  // ── corporation checks (several distinct messages on one code) ──
  'corp.reserved': 'This mission is reserved to the corporation members',
  'corp.not_member_create': 'You are not a member of this corporation',
  'corp.not_member_manage': 'This mission belongs to a corporation you are not a member of',
  // ── group sharing ──
  'share.blocked': 'A mission with active assignees cannot be shared',
  'share.unblocked': 'A mission with active assignees cannot be unshared',
  // ── event capacity ──
  'event.capacity_1000': 'Event missions allow at most 1000 assignees',
  'event.capacity_range': 'maxAssignees must be between 1 and 1000',
  // ── economy client ──
  'economy.credit_failed': 'Economy credit failed ({status}): {message}',
  'economy.debit_failed': 'Economy debit failed ({status}): {message}',
  // ── prerequisite details ──
  'prereq.detail.balance': 'balance {balance} < {amount} {currency}',
  'prereq.detail.owns': '{scope} {itemId}: {held} < {quantity}',
  'prereq.detail.no_profile': 'profile not found',
  'prereq.detail.reputation': 'reputation {reputation} < {min}',
  'prereq.detail.not_corp': 'not a member of corporation {corporationId}',
} as const;

/** Keys of the construction-time catalog (typed from the English source). */
export type MessageKey = keyof typeof en & string;

/** French pairs — TypeScript enforces full coverage of `en`. */
export const fr: Record<MessageKey, string> = {
  // ── not found ──
  'not_found.mission': 'Mission {id} introuvable',
  'not_found.mission_bare': 'Mission introuvable',
  'not_found.objective': 'Objectif {id} introuvable sur la mission {missionId}',
  'not_found.group': 'Groupe {id} introuvable',
  'not_found.assignment': 'Assignation introuvable',
  'not_found.no_assignment': 'Aucune assignation pour la mission {missionId}',
  // ── auth ──
  'auth.missing_bearer': 'Token Bearer manquant',
  'auth.bad_subject': "Le token n'a pas de sujet exploitable",
  'auth.invalid_token': 'Token invalide',
  'auth.missing_service_token': 'Token de service manquant',
  'auth.invalid_service_token': 'Token de service invalide',
  'auth.not_service': "Authentification de service manquante",
  'auth.not_authenticated': 'Non authentifié',
  'auth.requires_role': 'Nécessite le rôle {role}',
  'auth.client_forbidden': "Le client {clientId} n'est pas un service autorisé",
  'auth.service_bad_subject': "Le token de service n'a pas de sujet exploitable",
  'auth.service_account_token': "Le token n'est pas émis pour un compte de service",
  'auth.requires_service_role': 'Nécessite le rôle de service {role}',
  // ── specification ──
  'spec.unknown_prereq': 'Kind de prérequis inconnu : {kind}',
  'prereq.missing_kind': 'Kind de prérequis absent du registre : {kind}',
  // ── corporation checks ──
  'corp.reserved': 'Cette mission est réservée aux membres de la corporation',
  'corp.not_member_create': "Vous n'êtes pas membre de cette corporation",
  'corp.not_member_manage': "Cette mission appartient à une corporation dont vous n'êtes pas membre",
  // ── group sharing ──
  'share.blocked': 'Une mission avec des assignés actifs ne peut pas être partagée',
  'share.unblocked': 'Une mission avec des assignés actifs ne peut pas être retirée du partage',
  // ── event capacity ──
  'event.capacity_1000': 'Les missions event autorisent au plus 1000 assignés',
  'event.capacity_range': 'maxAssignees doit être compris entre 1 et 1000',
  // ── economy client ──
  'economy.credit_failed': "Échec du crédit Economy ({status}) : {message}",
  'economy.debit_failed': "Échec du débit Economy ({status}) : {message}",
  // ── prerequisite details ──
  'prereq.detail.balance': 'solde {balance} < {amount} {currency}',
  'prereq.detail.owns': '{scope} {itemId} : {held} < {quantity}',
  'prereq.detail.no_profile': 'profil introuvable',
  'prereq.detail.reputation': 'réputation {reputation} < {min}',
  'prereq.detail.not_corp': 'pas membre de la corporation {corporationId}',
};

/**
 * French overrides keyed by error code (English remains the thrown message). Rendered by
 * the error handler for `Accept-Language: fr`; `{name}` placeholders take
 * `HttpError.params`.
 */
export const frCodeMessages: Record<string, string> = {
  // ── mission domain (single template per code) ──
  DELIVERY_TARGET_SELF: 'La cible de livraison doit différer du détenteur',
  NOT_MISSION_ISSUER: 'Seul le créateur de la mission peut confirmer ses objectifs',
  OUT_OF_ZONE: "Vous n'êtes pas dans une zone où cette mission est disponible",
  MISSING_OBJECTIVES: 'Une mission nécessite au moins un objectif',
  REWARD_REQUIRED: 'Les missions créées par des joueurs nécessitent au moins un composant de récompense',
  ALREADY_ASSIGNED: 'Vous avez déjà accepté cette mission',
  ALREADY_COMPLETED: 'Vous avez déjà complété cette mission',
  NOT_GROUP_MEMBER: "Cette mission est réservée aux membres d'un groupe dont vous ne faites pas partie",
  NO_GROUP: "Cette mission est réservée à un groupe : créez ou rejoignez d'abord un groupe",
  GROUP_ALREADY_CLAIMED: 'Un autre groupe a déjà réclamé cette mission',
  MISSION_FULL: "La mission n'a plus de place libre",
  ASSIGNMENT_NOT_ACTIVE: "L'assignation est {status}",
  OBJECTIVES_INCOMPLETE: 'Tous les objectifs ne sont pas complétés',
  DUPLICATE_SCRIPT_ID: 'Une mission avec ce scriptId existe déjà',
  CORPORATION_REQUIRED: 'Une mission corporation nécessite un issuerId',
  OBJECTIVE_NOT_CONFIRMABLE: "Seuls les objectifs confirmés par l'émetteur peuvent être confirmés",
  NOT_MISSION_MANAGER: 'Seul le créateur de la mission peut la gérer',
  NOT_CORPORATION_MISSION: 'Seules les missions corporation peuvent être déclarées en event',
  EVENT_FORBIDDEN: 'Nécessite la permission manage_corporation',
  MISSION_NOT_OPEN: 'La mission est {status}',
  MISSION_EXPIRED: 'La mission a expiré',
  MISSION_COMPLETED: 'Une mission complétée ne peut pas être annulée',
  OBJECTIVE_LOCKED: 'Les objectifs précédents doivent être complétés d’abord',
  // ── spec validation ──
  UNKNOWN_OBJECTIVE_KIND: 'Kind objectif inconnu : {kind}',
  OBJECTIVE_NOT_IN_CATEGORY: 'Le kind {kind} ne peut pas être utilisé sur une mission de catégorie {category}',
  PREREQ_NOT_IN_CATEGORY: 'Le kind de prérequis {kind} ne peut pas être utilisé sur une mission de catégorie {category}',
  INVALID_OBJECTIVE_PARAMS: 'Params invalides pour le kind objectif {kind} : {issues}',
  INVALID_PREREQUISITE_PARAMS: 'Params invalides pour le kind de prérequis {kind} : {issues}',
  // ── configuration (one generic text covering every variant of the code) ──
  INVENTORY_NOT_CONFIGURED: 'Le service Inventory n’est pas configuré',
  ECONOMY_NOT_CONFIGURED: 'Le service Economy n’est pas configuré',
  SOCIAL_NOT_CONFIGURED: 'Le service Social n’est pas configuré',
  // ── outbound clients (templates with params) ──
  SOCIAL_AUTH_FAILED: 'Échec d’authentification Social ({status}) : {body}',
  SOCIAL_LOOKUP_FAILED: 'Échec de l’interrogation de Social ({status}) : {body}',
  ECONOMY_AUTH_FAILED: 'Échec d’authentification Economy ({status}) : {body}',
  ECONOMY_READ_FAILED: 'Échec de lecture Economy ({status}) : {body}',
  INVENTORY_AUTH_FAILED: 'Échec d’authentification Inventory ({status}) : {body}',
  INVENTORY_READ_FAILED: 'Échec de lecture Inventory ({status}) : {body}',
  INVENTORY_CALL_FAILED: 'Échec de l’appel Inventory {method} {path} ({status}) : {message}',
  ITEM_REWARD_NOT_SPLITABLE: 'Récompense item non partageable entre plusieurs assignés (instance unique ou séquestrée)',
  EVENT_CAPACITY_REQUIRES_FLAG: 'Ramenez maxAssignees à 100 ou moins avant de retirer le flag event',
  // ── HTTP plumbing ──
  INVALID_JSON: 'Corps JSON malformé',
  INTERNAL_ERROR: 'Erreur interne du serveur',
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
