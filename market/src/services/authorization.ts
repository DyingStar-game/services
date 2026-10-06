/**
 * Order/demand authorization. A player trades from their own wallet; a corporation trade
 * is placed by a member holding `market:trade` — decided by Social (granted to every
 * member, so this is a membership check in practice, centralized so the rule lives in
 * one place).
 */
import { env } from '../config/env.js';
import { t } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';
import { authorizeAction, isSocialConfigured } from './social.client.js';

/** Minimal party shape used for authorization. */
export interface AuthorizedParty {
  holderType: 'player' | 'npc' | 'corporation' | 'system';
  holderId: string;
}

/** Catalogued action gating every corporation trade of this service. */
export const TRADE_ACTION = 'market:trade';

/**
 * Ensures the acting player may trade on behalf of the holder.
 * @param party - Holder the order/demand belongs to.
 * @param actorPlayerId - Authenticated player id performing the action.
 * @throws 503 when Social is unavailable, 403 `NOT_CORPORATION_MEMBER` / `FORBIDDEN`.
 */
export async function ensureCorporationTrade(party: AuthorizedParty, actorPlayerId: string): Promise<void> {
  if (party.holderType !== 'corporation') return;
  if (!isSocialConfigured()) {
    // Local dev without Social: trust the authenticated player.
    if (env.authDevBypass) return;
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const decision = await authorizeAction({
    holderType: 'corporation',
    holderId: party.holderId,
    playerId: actorPlayerId,
    action: TRADE_ACTION,
  });
  if (decision.allowed) return;
  if (decision.reason === 'not_member') {
    throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'You are not a member of this corporation');
  }
  throw new HttpError(403, 'FORBIDDEN', t('forbidden.corp_permission', { action: TRADE_ACTION }));
}
