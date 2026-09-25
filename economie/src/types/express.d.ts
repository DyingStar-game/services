/**
 * Express request type augmentation for the authenticated player.
 */
import type { AuthenticatedPlayer } from '../middleware/auth.js';

declare global {
  namespace Express {
    interface Request {
      /** Player resolved from the bearer JWT (set by `playerAuth`). */
      player?: AuthenticatedPlayer;
    }
  }
}

export {};