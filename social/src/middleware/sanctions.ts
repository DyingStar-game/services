/**
 * Blocks suspended/banned players from the social API (after `playerAuth`).
 */
import type { NextFunction, Request, Response } from 'express';

import { HttpError } from '../lib/httpError.js';
import { getBlockingSanction } from '../services/sanctions.service.js';

/**
 * Responds 403 `SANCTIONED` when the player has an active suspension or ban.
 * Mount it on routes that should stay closed; `/me/reputation` and `/me/sanctions` stay reachable.
 */
export async function enforceSanctions(req: Request, _res: Response, next: NextFunction): Promise<void> {
  if (!req.player) {
    next();
    return;
  }
  try {
    const sanction = await getBlockingSanction(req.player.id);
    if (sanction) {
      next(
        new HttpError(
          403,
          'SANCTIONED',
          `Account ${sanction.type === 'ban' ? 'banned' : 'suspended'}${sanction.expiresAt ? ` until ${sanction.expiresAt.toISOString()}` : ''}: ${sanction.reason}`,
        ),
      );
      return;
    }
    next();
  } catch (err) {
    next(err);
  }
}
