/**
 * Central permission evaluation (PDP). Every organisation-scoped check performed by a
 * trusted service (`mission`, `inventory`, `economie`, `market`, the game server) goes
 * through `POST /api/internal/authorize`, so no service hardcodes a permission again.
 *
 * Resolution order for one check (fail-closed, no deny rules, no wildcards):
 *  1. not a member                → `not_member`
 *  2. CEO / head office           → allowed (implicitly holds everything)
 *  3. rank/office holds `action`  → allowed (exact match, works for free actions too)
 *  4. catalog `satisfiedBy`       → allowed when a legacy permission is satisfied
 *  5. catalog `defaultMember`     → allowed for every member
 *  6. otherwise                   → `missing_permission`
 */
import { currentLang } from '../i18n/index.js';
import { getCorporationMembership } from './corporations.service.js';
import { getPoliticalMembership } from './politics.service.js';
import {
  PERMISSION_CATALOG,
  describeAction,
  type OrgKind,
  type PermissionDescriptor,
} from './permissionCatalog.js';

/** A single authorization check. */
export interface AuthorizeCheck {
  holderType: OrgKind;
  holderId: string;
  playerId: string;
  action: string;
}

/** Why a check was granted or refused. */
export type AuthorizeReason = 'allowed' | 'not_member' | 'missing_permission';

/** Outcome of one check. Always returned with HTTP 200 — callers map `reason` to their own errors. */
export interface AuthorizeResult {
  holderType: OrgKind;
  holderId: string;
  playerId: string;
  action: string;
  allowed: boolean;
  reason: AuthorizeReason;
  /** Name of the rank/office held, or null when the player is not a member. */
  role: string | null;
  /** True when the rank is the CEO / the office is the head (grants everything). */
  leader: boolean;
  /** Permissions actually held by the rank/office (empty for non-members). */
  permissions: string[];
}

/**
 * Pure resolution rule (see the module doc).
 * @param holder - Organisation kind.
 * @param action - Requested action.
 * @param permissions - Permissions held by the rank/office.
 * @param leader - Whether the holder is the CEO/head.
 * @returns Allowed flag and reason.
 */
export function evaluateAction(
  holder: OrgKind,
  action: string,
  permissions: string[],
  leader: boolean,
): { allowed: boolean; reason: AuthorizeReason } {
  if (leader) return { allowed: true, reason: 'allowed' };
  if (permissions.includes(action)) return { allowed: true, reason: 'allowed' };
  const descriptor = describeAction(holder, action);
  if (descriptor?.satisfiedBy?.some((p) => permissions.includes(p))) {
    return { allowed: true, reason: 'allowed' };
  }
  if (descriptor?.defaultMember) return { allowed: true, reason: 'allowed' };
  return { allowed: false, reason: 'missing_permission' };
}

/**
 * Evaluates one authorization check against the organisation's membership data.
 * @param check - Holder, player and action.
 * @returns The check outcome.
 */
export async function authorize(check: AuthorizeCheck): Promise<AuthorizeResult> {
  const { holderType, holderId, playerId, action } = check;
  const base = { holderType, holderId, playerId, action };

  if (holderType === 'corporation') {
    const membership = await getCorporationMembership(holderId, playerId);
    if (!membership) {
      return { ...base, allowed: false, reason: 'not_member', role: null, leader: false, permissions: [] };
    }
    const { rank } = membership;
    const permissions = rank.permissions ?? [];
    const leader = rank.isCeo;
    return {
      ...base,
      ...evaluateAction(holderType, action, permissions, leader),
      role: rank.name,
      leader,
      permissions,
    };
  }

  const membership = await getPoliticalMembership(holderId, playerId);
  if (!membership) {
    return { ...base, allowed: false, reason: 'not_member', role: null, leader: false, permissions: [] };
  }
  const { office } = membership;
  const permissions = office.permissions ?? [];
  const leader = office.isHead;
  return {
    ...base,
    ...evaluateAction(holderType, action, permissions, leader),
    role: office.name,
    leader,
    permissions,
  };
}

/**
 * Evaluates a list of checks (in order).
 * @param checks - Checks to evaluate.
 * @returns One result per check.
 */
export async function authorizeAll(checks: AuthorizeCheck[]): Promise<AuthorizeResult[]> {
  return Promise.all(checks.map((check) => authorize(check)));
}

/** Catalog row served by `GET /api/internal/permissions/catalog`. */
export type PermissionCatalogRow = Omit<PermissionDescriptor, 'description'> & { description: string };

/**
 * The catalog with its descriptions localized for the ambient request language.
 * @returns Catalog rows.
 */
export function permissionCatalog(): PermissionCatalogRow[] {
  const fr = currentLang() === 'fr';
  return PERMISSION_CATALOG.map(({ description, ...rest }) => ({
    ...rest,
    description: fr ? description.fr : description.en,
  }));
}
