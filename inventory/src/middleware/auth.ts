/**
 * Authentication middlewares: Keycloak JWT for players, Keycloak service-account JWT for
 * internal callers (per-capability realm roles). The shared `X-Internal-Key` is only
 * honoured in non-production when `INTERNAL_DEV_BYPASS=true` (see `serviceAuth`).
 */
import { timingSafeEqual } from 'crypto';
import { t } from '../i18n/index.js';
import type { NextFunction, Request, Response } from 'express';
import { createRemoteJWKSet, jwtVerify, type JWTPayload } from 'jose';

import { env } from '../config/env.js';
import { HttpError } from '../lib/httpError.js';

/** Identity extracted from the access token. */
export interface AuthenticatedPlayer {
  /** Keycloak subject (UUID) — the player id everywhere in this service. */
  id: string;
  username: string;
  roles: string[];
}

/** Identity of a trusted calling service, extracted from a Keycloak service-account token. */
export interface AuthenticatedService {
  /** Keycloak client id that requested the token (`azp` / `client_id`). */
  clientId: string;
  /** Service-account subject (`sub`, a UUID). */
  subject: string;
  /** Realm and client roles granted to that service account. */
  roles: string[];
}

/** Service-account capability roles, checked per route by `requireServiceRole`. */
export const SERVICE_ROLES = {
  read: 'inventory:read',
  hold: 'inventory:hold',
  transfer: 'inventory:transfer',
  credit: 'inventory:credit',
  corporationManage: 'inventory:corporation:manage',
} as const;

/** Moderation roles, lowest to highest; used by the admin read endpoints. */
export const MODERATION_ROLES = ['moderator', 'admin', 'supervisor'] as const;
export type ModerationRole = (typeof MODERATION_ROLES)[number];

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// Lazily created so a missing Keycloak does not break startup in dev-bypass mode.
let jwks: ReturnType<typeof createRemoteJWKSet> | null = null;

function getJwks() {
  jwks ??= createRemoteJWKSet(new URL(env.oidc.jwksUrl));
  return jwks;
}

function extractBearer(req: Request): string | null {
  const header = req.header('authorization') ?? '';
  return header.startsWith('Bearer ') ? header.slice(7) : null;
}

function playerFromClaims(payload: JWTPayload): AuthenticatedPlayer | null {
  if (!payload.sub || !UUID_RE.test(payload.sub)) return null;
  const realmAccess = payload.realm_access as { roles?: string[] } | undefined;
  return {
    id: payload.sub,
    username: (payload.preferred_username as string | undefined) ?? payload.sub,
    roles: realmAccess?.roles ?? [],
  };
}

/**
 * Requires a valid Keycloak bearer token and sets `req.player`.
 * With `AUTH_DEV_BYPASS`, `X-Player-Id` (+ optional `X-Player-Name`) is accepted instead.
 */
export async function playerAuth(req: Request, _res: Response, next: NextFunction): Promise<void> {
  if (env.authDevBypass) {
    const id = req.header('x-player-id');
    if (id && UUID_RE.test(id)) {
      const roles = (req.header('x-player-roles') ?? '').split(',').map((r) => r.trim()).filter(Boolean);
      req.player = { id, username: req.header('x-player-name') ?? id, roles };
      next();
      return;
    }
  }

  const token = extractBearer(req);
  if (!token) {
    next(new HttpError(401, 'UNAUTHORIZED', t('auth.missing_bearer')));
    return;
  }
  try {
    const { payload } = await jwtVerify(token, getJwks(), {
      issuer: env.oidc.issuer,
      audience: env.oidc.audience,
    });
    const player = playerFromClaims(payload);
    if (!player) {
      next(new HttpError(401, 'UNAUTHORIZED', t('auth.bad_subject')));
      return;
    }
    req.player = player;
    next();
  } catch {
    next(new HttpError(401, 'UNAUTHORIZED', t('auth.invalid_token')));
  }
}

/**
 * Highest moderation role held by a player, or null.
 * @param player - Authenticated player.
 * @returns Role.
 */
export function moderationRoleOf(player: AuthenticatedPlayer): ModerationRole | null {
  for (let i = MODERATION_ROLES.length - 1; i >= 0; i--) {
    if (player.roles.includes(MODERATION_ROLES[i])) return MODERATION_ROLES[i];
  }
  return null;
}

/**
 * Middleware requiring at least `role` (after `playerAuth`).
 * @param role - Minimum moderation role.
 * @returns Middleware responding 403 otherwise.
 */
export function requireRole(role: ModerationRole) {
  return (req: Request, _res: Response, next: NextFunction): void => {
    const held = req.player ? moderationRoleOf(req.player) : null;
    if (held === null || MODERATION_ROLES.indexOf(held) < MODERATION_ROLES.indexOf(role)) {
      next(new HttpError(403, 'FORBIDDEN', t('auth.requires_role', { role })));
      return;
    }
    next();
  };
}

/** Roles carried by a token: realm roles plus the roles of this API's client. */
function serviceRolesFromClaims(payload: JWTPayload): string[] {
  const realmAccess = payload.realm_access as { roles?: string[] } | undefined;
  const resourceAccess = payload.resource_access as Record<string, { roles?: string[] }> | undefined;
  const clientRoles = resourceAccess?.[env.oidc.serviceAudience]?.roles ?? [];
  return [...new Set([...(realmAccess?.roles ?? []), ...clientRoles])];
}

/**
 * Requires a Keycloak service-account token (client_credentials grant) whose `azp` is an
 * allowed service client and whose audience targets this API, then sets `req.service`.
 *
 * In non-production, `INTERNAL_DEV_BYPASS=true` additionally accepts the legacy
 * `X-Internal-Key` (grants every service role) for local tests.
 */
export async function serviceAuth(req: Request, _res: Response, next: NextFunction): Promise<void> {
  if (env.internalDevBypass) {
    const given = req.header('x-internal-key') ?? '';
    const expected = env.internalApiKey;
    if (expected && given.length === expected.length && timingSafeEqual(Buffer.from(given), Buffer.from(expected))) {
      req.service = {
        clientId: 'dev-internal-key',
        subject: '00000000-0000-0000-0000-000000000000',
        roles: Object.values(SERVICE_ROLES),
      };
      next();
      return;
    }
  }

  const token = extractBearer(req);
  if (!token) {
    next(new HttpError(401, 'UNAUTHORIZED', t('auth.missing_service_token')));
    return;
  }
  let payload: JWTPayload;
  try {
    ({ payload } = await jwtVerify(token, getJwks(), {
      issuer: env.oidc.issuer,
      audience: env.oidc.serviceAudience,
    }));
  } catch {
    next(new HttpError(401, 'UNAUTHORIZED', t('auth.invalid_service_token')));
    return;
  }

  const clientId = (payload.azp as string | undefined) ?? (payload.client_id as string | undefined);
  if (!clientId || !env.internal.serviceClients.includes(clientId)) {
    next(new HttpError(403, 'SERVICE_FORBIDDEN', t('auth.client_forbidden', { clientId: clientId ?? '(none)' })));
    return;
  }
  if (typeof payload.sub !== 'string' || !UUID_RE.test(payload.sub)) {
    next(new HttpError(403, 'SERVICE_FORBIDDEN', t('auth.service_bad_subject')));
    return;
  }
  const username = payload.preferred_username as string | undefined;
  if (username && !username.startsWith('service-account-')) {
    next(new HttpError(403, 'SERVICE_FORBIDDEN', t('auth.service_account_token')));
    return;
  }
  req.service = { clientId, subject: payload.sub, roles: serviceRolesFromClaims(payload) };
  next();
}

/**
 * Middleware requiring a service capability (after `serviceAuth`).
 * @param role - Realm role the calling service must hold.
 * @returns Middleware responding 403 otherwise.
 */
export function requireServiceRole(role: string) {
  return (req: Request, _res: Response, next: NextFunction): void => {
    if (!req.service || !req.service.roles.includes(role)) {
      next(new HttpError(403, 'FORBIDDEN', t('auth.requires_service_role', { role })));
      return;
    }
    next();
  };
}

/** Returns the service bound on the request; throws if `serviceAuth` did not run. */
export function requireService(req: Request): AuthenticatedService {
  if (!req.service) throw new HttpError(401, 'UNAUTHORIZED', t('auth.not_service'));
  return req.service;
}

/**
 * Returns the player bound on the request; throws if `playerAuth` did not run.
 * @param req - Express request.
 * @returns Authenticated player.
 */
export function requirePlayer(req: Request): AuthenticatedPlayer {
  if (!req.player) throw new HttpError(401, 'UNAUTHORIZED', t('auth.not_authenticated'));
  return req.player;
}
