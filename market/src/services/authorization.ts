/**
 * Order/demand authorization. A player trades from their own wallet; a corporation trade
 * must be placed by one of its members. Corporation membership is authoritative in Social.
 */
import { env } from '../config/env.js';
import { HttpError } from '../lib/httpError.js';
import { isSocialConfigured, isCorporationMember } from './social.client.js';

/** Minimal party shape used for authorization. */
export interface AuthorizedParty {
  holderType: 'player' | 'npc' | 'corporation' | 'system';
  holderId: string;
}

/**
 * Ensures the acting player may trade on behalf of the holder.
 * @param party - Holder the order/demand belongs to.
 * @param actorPlayerId - Authenticated player id performing the action.
 */
export async function ensureCorporationMember(party: AuthorizedParty, actorPlayerId: string): Promise<void> {
  if (party.holderType !== 'corporation') return;
  if (!isSocialConfigured()) {
    // Local dev without Social: trust the authenticated player.
    if (env.authDevBypass) return;
    throw new HttpError(503, 'SOCIAL_NOT_CONFIGURED', 'SOCIAL_API_URL is not configured');
  }
  const member = await isCorporationMember(actorPlayerId, party.holderId);
  if (!member) throw new HttpError(403, 'NOT_CORPORATION_MEMBER', 'You are not a member of this corporation');
}
