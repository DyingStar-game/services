import { z } from 'zod';

import type { ObjectiveKind } from '../types.js';

/**
 * Subjective objective confirmed by the mission issuer (Eco's custom clause): no automatic
 * verification, the player (or the game server, internally) who posted the mission
 * confirms completion.
 */
export const manualKind: ObjectiveKind = {
  kind: 'manual',
  evaluation: 'issuer',
  quantity: false,
  paramsSchema: z.object({}).strict(),
  paramsJsonSchema: { type: 'object', properties: {}, additionalProperties: false },
  summary: { en: 'Subjective work: the mission issuer confirms completion (no automatic check)', fr: "Travail subjectif : l'émetteur de la mission confirme la complétion (vérification manuelle)" },
  categories: 'all',
};
