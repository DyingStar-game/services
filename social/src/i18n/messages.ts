/**
 * Message catalogs (social).
 *
 * Two mechanisms:
 * - `frCodeMessages`: French override keyed by **error code**, rendered by the error
 *   handler when the request asks for French (English stays the thrown message). Only
 *   codes whose every site share a single template belong here; multi-text codes are
 *   localized at construction time through `t()`.
 * - `en`/`fr`: paired templates for messages built at construction time (conflict/
 *   forbidden/not-found texts, auth, sanctions) through `t()`.
 *
 * Convention: every parameter reference uses `{name}` placeholders — identical in both
 * languages — and `interpolate` only replaces placeholders present in `params`. Technical
 * identifiers (codes, enum values, field names) are never translated.
 */
import { currentLang, type Lang } from './lang.js';

/** English source templates for construction-time messages (`t()`). */
export const en = {
  "conflict.block_exists": "A block exists between these players",
  "conflict.group_name_taken": "A group with this name already exists",
  "conflict.corp_member": "Already a member of this corporation",
  "conflict.group_member": "Already a member of this group",
  "conflict.already_friends": "Already friends",
  "conflict.application_pending": "Application already pending",
  "conflict.invitation_pending": "Invitation already pending",
  "conflict.npc_corp_member": "NPC is already a member of this corporation",
  "conflict.npc_politics_member": "NPC is already a member of this political entity",
  "conflict.player_corp_member": "Player is already a member of this corporation",
  "conflict.profile_politics_member": "Profile is already a member of this political entity",
  "conflict.request_pending": "Request already pending",
  "conflict.default_office": "Set another office as default instead of unsetting this one",
  "conflict.default_rank": "Set another rank as default instead of unsetting this one",
  "conflict.id_taken": "This id already belongs to a player profile",
  "conflict.open_report": "You already have an open report on this target",
  "conflict.invite_self": "You cannot invite yourself",
  "conflict.report_state": "Report is already {status}",
  "conflict.rank_exists": "Rank \"{name}\" already exists",
  "conflict.office_exists": "Office \"{name}\" already exists",
  "conflict.name_alloc": "Could not allocate a unique display name for {username}",
  "conflict.npc_name_conflict": "NPC display name \"{name}\" conflicts with existing profile",
  "conflict.name_taken": "Display name \"{name}\" is already taken",
  "conflict.politics_name_taken": "Political entity name \"{name}\" is already taken",
  "conflict.corp_name_taken": "Corporation name \"{name}\" or ticker \"{ticker}\" is already taken",
  "conflict.already_in_group": "You already belong to a group",
  "conflict.player_in_group": "This player already belongs to a group",
  "forbidden.rank_outrank": "Cannot assign a rank equal or higher than your own",
  "forbidden.office_outrank": "Cannot assign an office equal or higher than your own",
  "forbidden.own_office": "Cannot change your own office",
  "forbidden.own_rank": "Cannot change your own rank",
  "forbidden.delete_rank_outrank": "Cannot delete a rank equal or higher than your own",
  "forbidden.delete_office_outrank": "Cannot delete an office equal or higher than your own",
  "forbidden.edit_rank_outrank": "Cannot edit a rank equal or higher than your own",
  "forbidden.edit_office_outrank": "Cannot edit an office equal or higher than your own",
  "forbidden.kick_outrank": "Cannot kick a member of equal or higher rank",
  "forbidden.manage_office_outrank": "Cannot manage a member of equal or higher office",
  "forbidden.manage_rank_outrank": "Cannot manage a member of equal or higher rank",
  "forbidden.remove_office_outrank": "Cannot remove a member of equal or higher office",
  "forbidden.corp_permission": "Missing corporation permission: recruit or invite",
  "forbidden.corp_permission_generic": "Missing corporation permission: {permission}",
  "forbidden.political_permission": "Missing political permission: {permission}",
  "forbidden.not_corp_member": "Not a member of this corporation",
  "forbidden.not_group_member": "Not a member of this group",
  "forbidden.not_politics_member": "Not a member of this political entity",
  "forbidden.not_party": "Not a party to this request",
  "forbidden.office_priority": "Office priority must be below your own",
  "forbidden.ceo_only_disband": "Only the CEO can disband the corporation",
  "forbidden.ceo_only_transfer": "Only the CEO can transfer leadership",
  "forbidden.addressee_only": "Only the addressee can accept a request",
  "forbidden.corp_only_application": "Only the corporation can accept an application",
  "forbidden.owner_only": "Only the group owner can do this",
  "forbidden.head_only_disband": "Only the head can disband the political entity",
  "forbidden.head_only_transfer": "Only the head can transfer leadership",
  "forbidden.invited_only": "Only the invited player can accept an invitation",
  "forbidden.ceo_rank_name": "Only the name of the CEO rank can be changed",
  "forbidden.head_office_name": "Only the name of the head office can be changed",
  "forbidden.rank_priority": "Rank priority must be below your own",
  "forbidden.report_max_escalation": "Report is already at the highest escalation level",
  "forbidden.ceo_must_transfer": "The CEO must transfer leadership before leaving",
  "forbidden.ceo_rank_undeletable": "The CEO rank cannot be deleted",
  "forbidden.default_rank_undeletable": "The default rank cannot be deleted; set another default first",
  "forbidden.default_office_undeletable": "The default office cannot be deleted; set another default first",
  "forbidden.head_must_transfer": "The head must transfer leadership before leaving",
  "forbidden.head_office_undeletable": "The head office cannot be deleted",
  "forbidden.owner_cannot_kicked": "The owner cannot be removed (they must leave)",
  "forbidden.not_recruiting": "This corporation is not recruiting",
  "forbidden.transfer_head_office_first": "Transfer the head office before removing its holder",
  "forbidden.use_leave": "Use leave to quit your own group",
  "forbidden.use_ceo_transfer": "Use the CEO transfer to assign the CEO rank",
  "forbidden.use_head_transfer": "Use the head transfer to assign the head office",
  "not_found.group": "Group {id} not found",
  "not_found.report": "Report {id} not found",
  "not_found.rank": "Rank {id} not found",
  "not_found.player": "Player {id} not found",
  "not_found.sanction": "Active sanction {id} not found",
  "not_found.friend_request": "Friend request {id} not found",
  "not_found.political_entity": "Political entity {id} not found",
  "not_found.office": "Office {id} not found",
  "not_found.request": "Request {id} not found",
  "not_found.invitation": "Invitation {id} not found",
  "not_found.corporation": "Corporation {id} not found",
  "not_found.politics_member": "Profile {id} is not a member of this political entity",
  "not_found.group_member": "Player {id} is not a member of this group",
  "not_found.corp_member": "Player {id} is not a member of this corporation",
  "target.report_self": "Cannot report yourself",
  "target.friend_self": "Cannot send a friend request to yourself",
  "target.already_head": "Already the head",
  "target.already_ceo": "Already the CEO",
  "target.meet_self": "A player cannot meet themselves",
  "target.block_self": "Cannot block yourself",
  "npc.add_only": "Only NPC profiles can be added this way",
  "npc.remove_only": "Only NPC profiles can be removed this way",
  "parent.self_politics": "A political entity cannot be its own parent",
  "parent.self_corp": "A corporation cannot be its own parent",
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
  "admin.moderation_role": "Suspensions and bans require the admin role",
  "sanction.banned_until": "Account banned until {until}: {reason}",
  "sanction.banned": "Account banned: {reason}",
  "sanction.suspended_until": "Account suspended until {until}: {reason}",
  "sanction.suspended": "Account suspended: {reason}",
} as const;

/** Keys of the construction-time catalog (typed from the English source). */
export type MessageKey = keyof typeof en & string;

/** French pairs — TypeScript enforces full coverage of `en`. */
export const fr: Record<MessageKey, string> = {
  "conflict.block_exists": "Un blocage existe entre ces joueurs",
  "conflict.group_name_taken": "Un groupe avec ce nom existe déjà",
  "conflict.corp_member": "Déjà membre de cette corporation",
  "conflict.group_member": "Déjà membre de ce groupe",
  "conflict.already_friends": "Déjà amis",
  "conflict.application_pending": "Candidature déjà en attente",
  "conflict.invitation_pending": "Invitation déjà en attente",
  "conflict.npc_corp_member": "Le PNJ est déjà membre de cette corporation",
  "conflict.npc_politics_member": "Le PNJ est déjà membre de cette entité politique",
  "conflict.player_corp_member": "Le joueur est déjà membre de cette corporation",
  "conflict.profile_politics_member": "Le profil est déjà membre de cette entité politique",
  "conflict.request_pending": "Demande déjà en attente",
  "conflict.default_office": "Définissez un autre office par défaut au lieu de retirer celui-ci",
  "conflict.default_rank": "Définissez un autre grade par défaut au lieu de retirer celui-ci",
  "conflict.id_taken": "Cet id appartient déjà à un profil joueur",
  "conflict.open_report": "Vous avez déjà un signalement ouvert sur cette cible",
  "conflict.invite_self": "Vous ne pouvez pas vous inviter vous-même",
  "conflict.report_state": "Le signalement est déjà {status}",
  "conflict.rank_exists": "Le grade \"{name}\" existe déjà",
  "conflict.office_exists": "L'office \"{name}\" existe déjà",
  "conflict.name_alloc": "Impossible d'attribuer un nom d'affichage unique à {username}",
  "conflict.npc_name_conflict": "Le nom d'affichage PNJ \"{name}\" entre en conflit avec un profil existant",
  "conflict.name_taken": "Le nom d'affichage \"{name}\" est déjà pris",
  "conflict.politics_name_taken": "Le nom de l'entité politique \"{name}\" est déjà pris",
  "conflict.corp_name_taken": "Le nom de corporation \"{name}\" ou le ticker \"{ticker}\" est déjà pris",
  "conflict.already_in_group": "Vous appartenez déjà à un groupe",
  "conflict.player_in_group": "Ce joueur appartient déjà à un groupe",
  "forbidden.rank_outrank": "Impossible d'attribuer un grade égal ou supérieur au vôtre",
  "forbidden.office_outrank": "Impossible d'attribuer un office égal ou supérieur au vôtre",
  "forbidden.own_office": "Impossible de changer votre propre office",
  "forbidden.own_rank": "Impossible de changer votre propre grade",
  "forbidden.delete_rank_outrank": "Impossible de supprimer un grade égal ou supérieur au vôtre",
  "forbidden.delete_office_outrank": "Impossible de supprimer un office égal ou supérieur au vôtre",
  "forbidden.edit_rank_outrank": "Impossible de modifier un grade égal ou supérieur au vôtre",
  "forbidden.edit_office_outrank": "Impossible de modifier un office égal ou supérieur au vôtre",
  "forbidden.kick_outrank": "Impossible d'exclure un membre de grade égal ou supérieur",
  "forbidden.manage_office_outrank": "Impossible de gérer un membre d'office égal ou supérieur",
  "forbidden.manage_rank_outrank": "Impossible de gérer un membre de grade égal ou supérieur",
  "forbidden.remove_office_outrank": "Impossible de retirer un membre d'office égal ou supérieur",
  "forbidden.corp_permission": "Permission corporation manquante : recruit ou invite",
  "forbidden.corp_permission_generic": "Permission corporation manquante : {permission}",
  "forbidden.political_permission": "Permission politique manquante : {permission}",
  "forbidden.not_corp_member": "Pas membre de cette corporation",
  "forbidden.not_group_member": "Pas membre de ce groupe",
  "forbidden.not_politics_member": "Pas membre de cette entité politique",
  "forbidden.not_party": "Pas partie à cette demande",
  "forbidden.office_priority": "La priorité de l'office doit être inférieure à la vôtre",
  "forbidden.ceo_only_disband": "Seul le CEO peut dissoudre la corporation",
  "forbidden.ceo_only_transfer": "Seul le CEO peut transférer le leadership",
  "forbidden.addressee_only": "Seul le destinataire peut accepter une demande",
  "forbidden.corp_only_application": "Seule la corporation peut accepter une candidature",
  "forbidden.owner_only": "Seul le propriétaire du groupe peut faire cela",
  "forbidden.head_only_disband": "Seul le chef peut dissoudre l'entité politique",
  "forbidden.head_only_transfer": "Seul le chef peut transférer le leadership",
  "forbidden.invited_only": "Seul le joueur invité peut accepter une invitation",
  "forbidden.ceo_rank_name": "Seul le nom du grade CEO peut être modifié",
  "forbidden.head_office_name": "Seul le nom de l'office de tête peut être modifié",
  "forbidden.rank_priority": "La priorité du grade doit être inférieure à la vôtre",
  "forbidden.report_max_escalation": "Le signalement est déjà au niveau d'escalade le plus haut",
  "forbidden.ceo_must_transfer": "Le CEO doit transférer le leadership avant de partir",
  "forbidden.ceo_rank_undeletable": "Le grade CEO ne peut pas être supprimé",
  "forbidden.default_rank_undeletable": "Le grade par défaut ne peut pas être supprimé ; définissez d'abord un autre grade par défaut",
  "forbidden.default_office_undeletable": "L'office par défaut ne peut pas être supprimé ; définissez d'abord un autre office par défaut",
  "forbidden.head_must_transfer": "Le chef doit transférer le leadership avant de partir",
  "forbidden.head_office_undeletable": "L'office de tête ne peut pas être supprimé",
  "forbidden.owner_cannot_kicked": "Le propriétaire ne peut pas être retiré (il doit quitter)",
  "forbidden.not_recruiting": "Cette corporation ne recrute pas",
  "forbidden.transfer_head_office_first": "Transférez l'office de tête avant de retirer son détenteur",
  "forbidden.use_leave": "Utilisez quitter pour sortir de votre propre groupe",
  "forbidden.use_ceo_transfer": "Utilisez le transfert CEO pour attribuer le grade CEO",
  "forbidden.use_head_transfer": "Utilisez le transfert de tête pour attribuer l'office de tête",
  "not_found.group": "Groupe {id} introuvable",
  "not_found.report": "Signalement {id} introuvable",
  "not_found.rank": "Grade {id} introuvable",
  "not_found.player": "Joueur {id} introuvable",
  "not_found.sanction": "Sanction active {id} introuvable",
  "not_found.friend_request": "Demande d'ami {id} introuvable",
  "not_found.political_entity": "Entité politique {id} introuvable",
  "not_found.office": "Office {id} introuvable",
  "not_found.request": "Demande {id} introuvable",
  "not_found.invitation": "Invitation {id} introuvable",
  "not_found.corporation": "Corporation {id} introuvable",
  "not_found.politics_member": "Le profil {id} n'est pas membre de cette entité politique",
  "not_found.group_member": "Le joueur {id} n'est pas membre de ce groupe",
  "not_found.corp_member": "Le joueur {id} n'est pas membre de cette corporation",
  "target.report_self": "Vous ne pouvez pas vous signaler vous-même",
  "target.friend_self": "Vous ne pouvez pas envoyer une demande d'ami à vous-même",
  "target.already_head": "Déjà le chef",
  "target.already_ceo": "Déjà le CEO",
  "target.meet_self": "Un joueur ne peut pas se rencontrer lui-même",
  "target.block_self": "Vous ne pouvez pas vous bloquer vous-même",
  "npc.add_only": "Seuls les profils PNJ peuvent être ajoutés ainsi",
  "npc.remove_only": "Seuls les profils PNJ peuvent être retirés ainsi",
  "parent.self_politics": "Une entité politique ne peut pas être sa propre parente",
  "parent.self_corp": "Une corporation ne peut pas être sa propre maison mère",
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
  "admin.moderation_role": "Les suspensions et bannissements nécessitent le rôle admin",
  "sanction.banned_until": "Compte banni jusqu'à {until} : {reason}",
  "sanction.banned": "Compte banni : {reason}",
  "sanction.suspended_until": "Compte suspendu jusqu'à {until} : {reason}",
  "sanction.suspended": "Compte suspendu : {reason}",
};

/** French overrides keyed by error code (English remains the thrown message). */
export const frCodeMessages: Record<string, string> = {
  "NO_HEAD": "L'entité politique n'a pas de chef",
  "GROUP_FULL": "Ce groupe n'a plus de place libre",
  "BLOCKED": "Un blocage existe entre ces joueurs",
  "INVALID_PARENT_LEVEL": "Le parent doit être d'un niveau politique supérieur",
  "POLITICAL_CYCLE": "Ce lien créerait un cycle dans la hiérarchie",
  "CORPORATION_CYCLE": "Ce lien créerait un cycle dans la hiérarchie",
  "NPC_NOT_APPLICABLE": "Cette action ne s'applique pas aux profils PNJ",
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

