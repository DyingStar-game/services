/**
 * Character standing: a per-domain integer score stored on the player profile
 * (`player_profiles.standing`). Domains are the mission categories (delivery, transport,
 * mining, exploration, divers); this service validates the shape and applies bounded
 * deltas.
 *
 * `general` is a **derived** domain (RMS of the five categories) and is never granted
 * directly — `addStanding` refuses it. Social is the system of record: the game writes
 * deltas through the internal API (`social:standing:write`), mission reads the map for
 * prerequisites and settlement. `reputation` (moderation/social score) is a different
 * column and is never touched here.
 */
import { eq, sql } from 'drizzle-orm';

import { db } from '../db/connection.js';
import { playerProfiles, type StandingMap } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';
import { requireProfile } from './profiles.service.js';

/** Bounds of a single standing score (any domain). */
export const STANDING_MIN = -100_000;
export const STANDING_MAX = 100_000;

/**
 * Valid standing domains: the five mission categories. `general` is reserved (derived)
 * and is refused by `addStanding`.
 */
export const STANDING_DOMAINS = ['delivery', 'transport', 'mining', 'exploration', 'divers'] as const;

/** Domain name: short label, no slash/whitespace (mission categories fit). */
const DOMAIN_RE = /^[a-z][a-z0-9_]{0,31}$/;

function assertDomain(domain: string): void {
  if (!DOMAIN_RE.test(domain)) {
    throw new HttpError(400, 'STANDING_DOMAIN_INVALID', `Invalid standing domain: ${domain}`, { domain });
  }
  if (domain === 'general') {
    throw new HttpError(400, 'STANDING_DOMAIN_RESERVED', 'Domain "general" is derived and cannot be granted directly', { domain });
  }
  if (!STANDING_DOMAINS.includes(domain as (typeof STANDING_DOMAINS)[number])) {
    throw new HttpError(400, 'STANDING_DOMAIN_UNKNOWN', `Unknown standing domain: ${domain}`, { domain });
  }
}

/**
 * Reads the standing map of a player, including the derived `general` domain.
 * `general` = RMS of the five categories: `round(sqrt(Σ value² / 5))`.
 * @param playerId - Player id.
 * @returns Domain → score (empty object when the profile has none). Includes `general`.
 * @throws 404 when the profile does not exist.
 */
export async function getStanding(playerId: string): Promise<StandingMap> {
  const profile = await requireProfile(playerId);
  const raw = profile.standing ?? {};
  // Derive `general` as the RMS of the five categories (never stored).
  const sumSquares = STANDING_DOMAINS.reduce((sum, d) => sum + (raw[d] ?? 0) ** 2, 0);
  const general = Math.round(Math.sqrt(sumSquares / STANDING_DOMAINS.length));
  return { ...raw, general };
}

/**
 * Applies a delta to one domain of a player's standing (creates the profile field on
 * first write, clamps into `[STANDING_MIN, STANDING_MAX]`). `general` is refused.
 * @param playerId - Player id.
 * @param domain - Standing domain (must be one of `STANDING_DOMAINS`).
 * @param amount - Signed delta to apply.
 * @returns The updated standing map (including derived `general`).
 * @throws 404 when the profile does not exist, 400 on an invalid/reserved domain or delta.
 */
export async function addStanding(playerId: string, domain: string, amount: number): Promise<StandingMap> {
  assertDomain(domain);
  if (!Number.isInteger(amount) || amount === 0) {
    throw new HttpError(400, 'STANDING_AMOUNT_INVALID', 'Standing delta must be a non-zero integer', { amount });
  }
  await requireProfile(playerId);
  await db
    .update(playerProfiles)
    .set({
      // jsonb increment for a single key, clamped in SQL so concurrent writers never
      // escape the bounds (the clamp is applied to the merged value).
      standing: sql`jsonb_set(
        coalesce(${playerProfiles.standing}, '{}'::jsonb),
        ARRAY[${domain}]::text[],
        to_jsonb(least(${STANDING_MAX}, greatest(${STANDING_MIN}, coalesce(
          (coalesce(${playerProfiles.standing}, '{}'::jsonb)->>${domain})::int, 0
        ) + ${amount})))
      )`,
      updatedAt: new Date(),
    })
    .where(eq(playerProfiles.playerId, playerId))
    .returning({ standing: playerProfiles.standing });
  return getStanding(playerId);
}
