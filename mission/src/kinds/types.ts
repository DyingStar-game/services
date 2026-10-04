/**
 * Kind registry: every objective and prerequisite of a mission is a *kind* — a named,
 * schema-validated entry in a registry (the "clauses" of a mission spec). Adding a
 * capability means adding one file here; the pipeline (create → accept → progress →
 * reward) never hardcodes mission rules.
 *
 * Three evaluation modes:
 * - `game`    the game server reports progress (gameplay the services cannot see);
 * - `service` the mission service measures progress against Inventory/Economy/Social
 *             state through the `measure` handler (called by the verify endpoint);
 * - `issuer`  the mission creator confirms completion (subjective work, Eco's custom
 *             clause).
 */
import type { z } from 'zod';

import type { AssigneeHolderType, MissionCategory, MissionObjective } from '../db/schema/index.js';

/** Who evaluates an objective's progress. */
export type EvaluationMode = 'game' | 'service' | 'issuer';

/** Minimal JSON Schema fragment served by `GET /api/missions/kinds`. */
export type JsonSchema = Record<string, unknown>;

/**
 * Mission categories a kind is allowed on. `'all'` means every category; the `generic`
 * category always bypasses the constraint (catch-all mission type).
 */
export type KindCategories = readonly MissionCategory[] | 'all';

/** Bilingual text: English source + French pair (placeholders `{name}` identical). */
export interface MessageText {
  en: string;
  fr: string;
}

/** Context handed to `service` kind measurements. */
export interface MeasureContext {
  /** Holder id (player or NPC) whose state is measured. */
  playerId: string;
  /** Holder kind: which Inventory/Economy holder is read. */
  holderType: AssigneeHolderType;
  missionId: string;
  /** The objective row (params, target and current progress). */
  objective: MissionObjective;
}

/** Context handed to prerequisite checks at acceptance time. */
export interface PrerequisiteContext {
  playerId: string;
  missionId: string;
  /** Prerequisite params, already parsed by the kind's `paramsSchema` (defaults applied). */
  params: Record<string, unknown>;
}

/** An objective kind: its evaluation mode, params schema and (for services) its probe. */
export interface ObjectiveKind {
  /** Stable kind name stored in `mission_objectives.type`. */
  kind: string;
  evaluation: EvaluationMode;
  /** Whether the objective uses `targetQuantity`/`currentProgress` accumulation. */
  quantity: boolean;
  /** Structured params, validated at creation (`mission_objectives.params`). */
  paramsSchema: z.ZodTypeAny;
  /** JSON Schema of `params`, served to mission builders. */
  paramsJsonSchema: JsonSchema;
  /** Short bilingual label shown to mission builders (no placeholder). */
  name: MessageText;
  summary: MessageText;
  /** Categories this objective kind may be used on (see {@link KindCategories}). */
  categories: KindCategories;
  /** `service` only: current progress measured against external state. */
  measure?: (ctx: MeasureContext) => Promise<number>;
}

/** A prerequisite kind: checked before a player may accept a mission. */
export interface PrerequisiteKind {
  kind: string;
  paramsSchema: z.ZodTypeAny;
  paramsJsonSchema: JsonSchema;
  /** Short bilingual label shown to mission builders (no placeholder). */
  name: MessageText;
  summary: MessageText;
  /** Categories this prerequisite may be used on (see {@link KindCategories}). */
  categories: KindCategories;
  check: (ctx: PrerequisiteContext) => Promise<{ ok: boolean; detail?: string }>;
}
