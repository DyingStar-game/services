/**
 * Prerequisite evaluation: every gate in `mission.prerequisites` is checked (in order)
 * against live service state before a player may accept a mission. Fails fast with a
 * structured 403 naming the kind and the reason.
 */
import { getPrerequisiteKind } from '../kinds/index.js';
import { t } from '../i18n/index.js';
import type { Mission } from '../db/schema/index.js';
import { HttpError } from '../lib/httpError.js';

/**
 * Evaluates a mission's prerequisites for a player.
 * @param mission - Mission whose `prerequisites` are checked.
 * @param playerId - Player trying to accept.
 * @throws 403 `PREREQ_FAILED` on the first unmet prerequisite, 500 for a kind missing
 *   from the registry (specs are validated at creation, so this indicates drift).
 */
export async function evaluatePrerequisites(mission: Mission, playerId: string): Promise<void> {
  for (const prereq of mission.prerequisites ?? []) {
    const kindDef = getPrerequisiteKind(prereq.kind);
    if (!kindDef) {
      throw new HttpError(500, 'UNKNOWN_PREREQ_KIND', t('prereq.missing_kind', { kind: prereq.kind }));
    }
    const params = kindDef.paramsSchema.parse(prereq.params ?? {}) as Record<string, unknown>;
    const result = await kindDef.check({ playerId, missionId: mission.id, params });
    if (!result.ok) {
      throw new HttpError(
        403,
        'PREREQ_FAILED',
        `Prerequisite ${prereq.kind} failed${result.detail ? `: ${result.detail}` : ''}`,
      );
    }
  }
}
