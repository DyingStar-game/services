/**
 * Permission catalog: the authoritative list of organisation actions Social knows how to
 * evaluate, plus their compatibility rules. Nothing outside this file may hardcode a
 * permission name.
 *
 * Two kinds of action exist:
 * - **Catalogued** actions carry metadata (`defaultMember`, `satisfiedBy`) and are
 *   resolved by `authorize.service`.
 * - **Free** actions are not listed: they only ever match an exact permission stored on
 *   the rank/office (no implicit grant). Their format is still enforced by `actionSchema`.
 *
 * Keying is `(holder, action)`: `manage_members` exists for both organisations with
 * different meanings, so a descriptor is always scoped to one holder kind.
 */

/** Organisation kinds a permission applies to. */
export type OrgKind = 'corporation' | 'political';

/** Canonical shape of an action string (`legacy`, `mission:treasury:commit`, …). */
export const ACTION_RE = /^[a-z][a-z0-9_]*(:[a-z][a-z0-9_]*)*$/;

/** Maximum length of an action string. */
export const ACTION_MAX_LENGTH = 64;

/** One catalogued permission and its evaluation rules. */
export interface PermissionDescriptor {
  /** Action identifier (canonical, lowercase). */
  action: string;
  /** Organisation kind this rule applies to. */
  holder: OrgKind;
  /** True when the action is a pre-ACL permission stored as-is on ranks/offices. */
  legacy: boolean;
  /** True when every member passes, whatever their rank/office holds. */
  defaultMember?: boolean;
  /** Legacy permissions that satisfy this action (checked after an exact match). */
  satisfiedBy?: string[];
  /** Human description, per language. */
  description: { en: string; fr: string };
}

/**
 * The catalog. Adding a cross-service check means adding an entry here (and nothing else):
 * the calling service only ever sends the action string.
 */
export const PERMISSION_CATALOG: readonly PermissionDescriptor[] = [
  // ── Corporation (legacy) ───────────────────────────────────────────────────
  {
    action: 'manage_corporation',
    holder: 'corporation',
    legacy: true,
    description: {
      en: 'Edit the corporation and attach it to a parent or political entity',
      fr: 'Modifier la corporation et la rattacher à un parent ou à une entité politique',
    },
  },
  {
    action: 'manage_ranks',
    holder: 'corporation',
    legacy: true,
    description: { en: 'Create, edit and delete ranks and their permissions', fr: 'Créer, modifier et supprimer les grades et leurs permissions' },
  },
  {
    action: 'manage_members',
    holder: 'corporation',
    legacy: true,
    description: { en: 'Add, rank and remove corporation members', fr: 'Ajouter, classer et retirer les membres de la corporation' },
  },
  {
    action: 'invite',
    holder: 'corporation',
    legacy: true,
    description: { en: 'Send join invitations', fr: "Envoyer des invitations à rejoindre" },
  },
  {
    action: 'recruit',
    holder: 'corporation',
    legacy: true,
    description: { en: 'Accept applications and recruit players', fr: "Accepter les candidatures et recruter des joueurs" },
  },

  // ── Corporation (cross-service) ───────────────────────────────────────────
  {
    action: 'mission:treasury:commit',
    holder: 'corporation',
    legacy: false,
    satisfiedBy: ['manage_corporation'],
    description: { en: 'Commit the corporation treasury to fund a mission', fr: 'Engager le trésor de la corporation pour financer une mission' },
  },
  {
    action: 'mission:event:manage',
    holder: 'corporation',
    legacy: false,
    satisfiedBy: ['manage_corporation'],
    description: { en: 'Flag a corporation mission as a big event', fr: 'Déclarer une mission corporation comme grand évènement' },
  },
  {
    action: 'inventory:poi:manage',
    holder: 'corporation',
    legacy: false,
    satisfiedBy: ['manage_corporation'],
    description: { en: 'Create, edit, share and delete the corporation POIs', fr: 'Créer, modifier, partager et supprimer les POI de la corporation' },
  },
  {
    action: 'economie:treasury:manage',
    holder: 'corporation',
    legacy: false,
    description: { en: 'Operate the corporate treasury (report, taxes, payroll)', fr: "Opérer le trésor de la corporation (rapport, taxes, paie)" },
  },
  {
    action: 'market:trade',
    holder: 'corporation',
    legacy: false,
    defaultMember: true,
    description: { en: 'Place and settle market trades for the corporation', fr: 'Placer et régler des échanges de marché pour la corporation' },
  },

  // ── Political entity (legacy) ─────────────────────────────────────────────
  {
    action: 'manage_entity',
    holder: 'political',
    legacy: true,
    description: { en: 'Edit the political entity', fr: "Modifier l'entité politique" },
  },
  {
    action: 'manage_offices',
    holder: 'political',
    legacy: true,
    description: { en: 'Create, edit and delete offices and their permissions', fr: 'Créer, modifier et supprimer les postes et leurs permissions' },
  },
  {
    action: 'manage_members',
    holder: 'political',
    legacy: true,
    description: { en: 'Add, seat and remove political members', fr: 'Ajouter, nommer et retirer les membres politiques' },
  },
  {
    action: 'manage_hierarchy',
    holder: 'political',
    legacy: true,
    description: { en: 'Attach the entity to a parent entity', fr: "Rattacher l'entité à une entité parente" },
  },
  {
    action: 'manage_treasury',
    holder: 'political',
    legacy: true,
    description: { en: 'Operate the political treasury', fr: "Opérer le trésor politique" },
  },
  {
    action: 'issue_currency',
    holder: 'political',
    legacy: true,
    description: { en: 'Issue (mint) currency into the treasury', fr: 'Émettre (frapper) de la monnaie dans le trésor' },
  },

  // ── Political entity (cross-service) ──────────────────────────────────────
  {
    action: 'mission:treasury:commit',
    holder: 'political',
    legacy: false,
    satisfiedBy: ['manage_treasury'],
    description: { en: 'Commit the political treasury to fund a mission', fr: 'Engager le trésor politique pour financer une mission' },
  },
  {
    action: 'inventory:poi:manage',
    holder: 'political',
    legacy: false,
    satisfiedBy: ['manage_entity'],
    description: { en: 'Create, edit, share and delete the entity POIs', fr: "Créer, modifier, partager et supprimer les POI de l'entité" },
  },
];

/** Index by `holder:action` for O(1) lookup. */
const BY_KEY = new Map<string, PermissionDescriptor>(
  PERMISSION_CATALOG.map((d) => [`${d.holder}:${d.action}`, d]),
);

/**
 * Catalog entry for one action on one organisation kind, or null for a free action.
 * @param holder - Organisation kind.
 * @param action - Action string.
 * @returns Descriptor, or null.
 */
export function describeAction(holder: OrgKind, action: string): PermissionDescriptor | null {
  return BY_KEY.get(`${holder}:${action}`) ?? null;
}

/**
 * Whether an action carries catalog metadata (free actions return false).
 * @param holder - Organisation kind.
 * @param action - Action string.
 * @returns True when catalogued.
 */
export function isCatalogued(holder: OrgKind, action: string): boolean {
  return describeAction(holder, action) !== null;
}
