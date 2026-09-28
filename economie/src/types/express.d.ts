/**
 * Express request type augmentation for the authenticated player and calling service.
 */
import type { AuthenticatedPlayer, AuthenticatedService } from '../middleware/auth.js';

declare global {
  namespace Express {
    interface Request {
      /** Player resolved from the bearer JWT (set by `playerAuth`). */
      player?: AuthenticatedPlayer;
      /** Calling service resolved from its Keycloak service-account JWT (set by `serviceAuth`). */
      service?: AuthenticatedService;
    }
  }
}

export {};