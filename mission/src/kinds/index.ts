/**
 * The kind registries and the discovery catalogue served by `GET /api/missions/kinds`.
 * Objective kinds live in `./objectives` (one file per kind); prerequisite kinds are
 * registered here as they land.
 */
import { MISSION_CATEGORIES } from '../db/schema/index.js';
import type { JsonSchema, ObjectiveKind, PrerequisiteKind } from './types.js';
import { customKind } from './objectives/custom.js';
import { deliverItemsKind } from './objectives/deliverItems.js';
import { deliverMaterialKind } from './objectives/deliverMaterial.js';
import { hasCreditsKind } from './objectives/hasCredits.js';
import { manualKind } from './objectives/manual.js';
import { ownsItemsKind } from './objectives/ownsItems.js';
import { transportKind } from './objectives/transport.js';
import { visitKind } from './objectives/visit.js';
import { corporationMemberKind } from './prerequisites/corporationMember.js';
import { hasCreditsPrerequisite } from './prerequisites/hasCredits.js';
import { minReputationKind } from './prerequisites/minReputation.js';
import { ownsItemsPrerequisite } from './prerequisites/ownsItems.js';

export type { EvaluationMode, JsonSchema, ObjectiveKind, PrerequisiteContext, PrerequisiteKind } from './types.js';

const OBJECTIVE_KIND_LIST: ObjectiveKind[] = [
  deliverMaterialKind,
  transportKind,
  visitKind,
  customKind,
  manualKind,
  ownsItemsKind,
  hasCreditsKind,
  deliverItemsKind,
];

/** Prerequisite kinds, checked at acceptance time. */
const PREREQUISITE_KIND_LIST: PrerequisiteKind[] = [
  hasCreditsPrerequisite,
  ownsItemsPrerequisite,
  minReputationKind,
  corporationMemberKind,
];

// Fail fast on an inconsistent registry: every `service` objective kind must define its
// measurement probe (the pipeline dispatches on it).
for (const kind of OBJECTIVE_KIND_LIST) {
  if (kind.evaluation === 'service' && !kind.measure) {
    throw new Error(`Service objective kind "${kind.kind}" must define a measure() handler`);
  }
}

export const OBJECTIVE_KINDS: ReadonlyMap<string, ObjectiveKind> = new Map(
  OBJECTIVE_KIND_LIST.map((k) => [k.kind, k]),
);
export const PREREQUISITE_KINDS: ReadonlyMap<string, PrerequisiteKind> = new Map(
  PREREQUISITE_KIND_LIST.map((k) => [k.kind, k]),
);

/** Looks up an objective kind, or undefined when unknown. */
export function getObjectiveKind(kind: string): ObjectiveKind | undefined {
  return OBJECTIVE_KINDS.get(kind);
}

/** Looks up a prerequisite kind, or undefined when unknown. */
export function getPrerequisiteKind(kind: string): PrerequisiteKind | undefined {
  return PREREQUISITE_KINDS.get(kind);
}

/** JSON Schema of a reward component list, served to mission builders. */
const REWARDS_JSON_SCHEMA: JsonSchema = {
  type: 'array',
  description: 'Reward components: at most one credits component, any number of item components (distinct items)',
  items: {
    oneOf: [
      {
        type: 'object',
        required: ['type', 'amount'],
        properties: {
          type: { const: 'credits' },
          currency: { type: 'string', maxLength: 16, default: 'credits' },
          amount: { type: 'integer', minimum: 1 },
        },
        additionalProperties: false,
      },
      {
        type: 'object',
        required: ['type', 'itemId', 'quantity'],
        properties: {
          type: { const: 'item' },
          itemId: { type: 'string', maxLength: 128 },
          quantity: { type: 'integer', minimum: 1 },
          instanceId: { type: 'string', format: 'uuid', description: 'Unique instance (single assignee only)' },
        },
        additionalProperties: false,
      },
    ],
  },
};

/** Serializable description of one kind (no functions) for the discovery endpoint. */
export interface ObjectiveKindInfo {
  kind: string;
  evaluation: ObjectiveKind['evaluation'];
  quantity: boolean;
  summary: string;
  params: JsonSchema;
}

/** Serializable description of one prerequisite kind. */
export interface PrerequisiteKindInfo {
  kind: string;
  summary: string;
  params: JsonSchema;
}

/** Full catalogue of categories, kinds and reward shape for mission builders. */
export function missionKindsCatalog(): {
  categories: readonly string[];
  objectiveKinds: ObjectiveKindInfo[];
  prerequisiteKinds: PrerequisiteKindInfo[];
  rewards: JsonSchema;
} {
  return {
    categories: MISSION_CATEGORIES,
    objectiveKinds: OBJECTIVE_KIND_LIST.map((k) => ({
      kind: k.kind,
      evaluation: k.evaluation,
      quantity: k.quantity,
      summary: k.summary,
      params: k.paramsJsonSchema,
    })),
    prerequisiteKinds: PREREQUISITE_KIND_LIST.map((k) => ({
      kind: k.kind,
      summary: k.summary,
      params: k.paramsJsonSchema,
    })),
    rewards: REWARDS_JSON_SCHEMA,
  };
}
