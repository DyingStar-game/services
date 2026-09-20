/**
 * Authentication middlewares: Keycloak JWT for players, shared key for internal callers.
 */
import { timingSafeEqual } from 'crypto';
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

/** Moderation roles, lowest to highest; each level implies the ones below. */
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
    next(new HttpError(401, 'UNAUTHORIZED', 'Missing bearer token'));
    return;
  }
  try {
    const { payload } = await jwtVerify(token, getJwks(), {
      issuer: env.oidc.issuer,
      audience: env.oidc.audience,
    });
    const player = playerFromClaims(payload);
    if (!player) {
      next(new HttpError(401, 'UNAUTHORIZED', 'Token has no usable subject'));
      return;
    }
    req.player = player;
    next();
  } catch {
    next(new HttpError(401, 'UNAUTHORIZED', 'Invalid token'));
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
 * Whether a player holds at least the given moderation role.
 * @param player - Authenticated player.
 * @param role - Minimum role.
 * @returns True if allowed.
 */
export function hasRole(player: AuthenticatedPlayer, role: ModerationRole): boolean {
  const held = moderationRoleOf(player);
  return held !== null && MODERATION_ROLES.indexOf(held) >= MODERATION_ROLES.indexOf(role);
}

/**
 * Middleware requiring at least `role` (after `playerAuth`).
 * @param role - Minimum moderation role.
 * @returns Middleware responding 403 otherwise.
 */
export function requireRole(role: ModerationRole) {
  return (req: Request, _res: Response, next: NextFunction): void => {
    if (!req.player || !hasRole(req.player, role)) {
      next(new HttpError(403, 'FORBIDDEN', `Requires role ${role}`));
      return;
    }
    next();
  };
}

/**
 * Requires `X-Internal-Key` to match `INTERNAL_API_KEY` (game server / trusted services).
 */
export function internalAuth(req: Request, _res: Response, next: NextFunction): void {
  if (!env.internalApiKey) {
    next(new HttpError(503, 'INTERNAL_AUTH_NOT_CONFIGURED', 'INTERNAL_API_KEY is not set'));
    return;
  }
  const given = req.header('x-internal-key') ?? '';
  const expected = env.internalApiKey;
  const ok =
    given.length === expected.length && timingSafeEqual(Buffer.from(given), Buffer.from(expected));
  if (!ok) {
    next(new HttpError(401, 'UNAUTHORIZED', 'Invalid internal key'));
    return;
  }
  next();
}

/**
 * Returns the player bound on the request; throws if `playerAuth` did not run.
 * @param req - Express request.
 * @returns Authenticated player.
 */
export function requirePlayer(req: Request): AuthenticatedPlayer {
  if (!req.player) throw new HttpError(401, 'UNAUTHORIZED', 'Not authenticated');
  return req.player;
}
