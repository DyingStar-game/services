/**
 * Mission spec validation: the single gate every creation path (internal and player)
 * goes through before touching the database. Looks up objective/prerequisite kinds in the
 * registry, validates their structured params, and enforces reward rules (component
 * shape, splitability, escrow configuration).
 */
import { getObjectiveKind, getPrerequisiteKind } from '../kinds/index.js';
import { HttpError } from '../lib/httpError.js';
import { isInventoryConfigured } from './inventory.client.js';
import type { PrerequisiteSpec, RewardComponent } from '../db/schema/index.js';

/** Raw objective as accepted by the creation endpoints. */
export interface SpecObjectiveInput {
  type: string;
  title: string;
  description?: string;
  targetQuantity?: number;
  unit?: string;
  locationFrom?: Record<string, unknown>;
  locationTo?: Record<string, unknown>;
  order?: number;
  params?: Record<string, unknown>;
  payload?: Record<string, unknown>;
}

/** Normalized objective, ready to insert. */
export interface ValidatedObjective {
  type: string;
  title: string;
  description?: string;
  targetQuantity: number;
  unit?: string;
  locationFrom?: Record<string, unknown>;
  locationTo?: Record<string, unknown>;
  order: number;
  params: Record<string, unknown>;
  payload?: Record<string, unknown>;
}

/** Validated spec: normalized objectives, prerequisites and reward components. */
export interface ValidatedSpec {
  objectives: ValidatedObjective[];
  prerequisites: PrerequisiteSpec[];
  rewards: RewardComponent[];
}

/** Spec fields shared by every creation path. */
export interface MissionSpecInput {
  objectives: SpecObjectiveInput[];
  prerequisites?: PrerequisiteSpec[];
  rewards?: RewardComponent[] | null;
  maxAssignees: number;
}

export interface ValidateSpecOptions {
  /** True for player missions: item rewards are escrowed from the creator. */
  escrowed: boolean;
  /** True for player missions: at least one reward component is required. */
  requireReward?: boolean;
}

/** Renders zod issues as `path: message` lines. */
function formatIssues(issues: { path: PropertyKey[]; message: string }[]): string {
  return issues.map((i) => `${i.path.map(String).join('.') || '(root)'}: ${i.message}`).join('; ');
}

/**
 * Validates a mission spec against the kind registry and reward rules.
 * @param input - Spec fields (objectives, prerequisites, rewards, capacity).
 * @param opts - Creation path options (`escrowed`/`requireReward` for player missions).
 * @returns The normalized spec to persist.
 * @throws 400 for unknown kinds, invalid params or reward rule violations, 503 when item
 *   rewards are required but Inventory is not configured.
 */
export function validateMissionSpec(input: MissionSpecInput, opts: ValidateSpecOptions): ValidatedSpec {
  if (input.objectives.length === 0) {
    throw new HttpError(400, 'MISSING_OBJECTIVES', 'A mission requires at least one objective');
  }

  const rewards = input.rewards ?? [];
  if (opts.requireReward && rewards.length === 0) {
    throw new HttpError(400, 'REWARD_REQUIRED', 'Player-created missions require at least one reward component');
  }

  for (const component of rewards) {
    if (component.type !== 'item') continue;
    if (component.instanceId && input.maxAssignees > 1) {
      throw new HttpError(
        400,
        'ITEM_REWARD_NOT_SPLITABLE',
        'A unique-instance reward cannot be shared by several assignees',
      );
    }
    if (opts.escrowed && input.maxAssignees > 1) {
      throw new HttpError(
        400,
        'ITEM_REWARD_NOT_SPLITABLE',
        'A player-escrowed item reward cannot be shared by several assignees',
      );
    }
    if (opts.escrowed && !isInventoryConfigured()) {
      throw new HttpError(503, 'INVENTORY_NOT_CONFIGURED', 'Item rewards require the Inventory service to be configured');
    }
  }

  const prerequisites: PrerequisiteSpec[] = [];
  for (const prereq of input.prerequisites ?? []) {
    const kindDef = getPrerequisiteKind(prereq.kind);
    if (!kindDef) {
      throw new HttpError(400, 'UNKNOWN_PREREQ_KIND', `Unknown prerequisite kind: ${prereq.kind}`);
    }
    const parsed = kindDef.paramsSchema.safeParse(prereq.params ?? {});
    if (!parsed.success) {
      throw new HttpError(
        400,
        'INVALID_PREREQUISITE_PARAMS',
        `Invalid params for prerequisite kind ${prereq.kind}: ${formatIssues(parsed.error.issues)}`,
      );
    }
    prerequisites.push({ kind: prereq.kind, params: parsed.data as Record<string, unknown> });
  }

  const objectives: ValidatedObjective[] = input.objectives.map((objective, index) => {
    const kindDef = getObjectiveKind(objective.type);
    if (!kindDef) {
      throw new HttpError(400, 'UNKNOWN_OBJECTIVE_KIND', `Unknown objective kind: ${objective.type}`);
    }
    const parsed = kindDef.paramsSchema.safeParse(objective.params ?? {});
    if (!parsed.success) {
      throw new HttpError(
        400,
        'INVALID_OBJECTIVE_PARAMS',
        `Invalid params for objective kind ${objective.type}: ${formatIssues(parsed.error.issues)}`,
      );
    }
    return {
      type: objective.type,
      title: objective.title,
      description: objective.description,
      targetQuantity: kindDef.quantity ? (objective.targetQuantity ?? 1) : 1,
      unit: objective.unit,
      locationFrom: objective.locationFrom,
      locationTo: objective.locationTo,
      order: objective.order ?? index,
      params: parsed.data as Record<string, unknown>,
      payload: objective.payload,
    };
  });

  return { objectives, prerequisites, rewards };
}
