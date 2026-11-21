/**
 * Kind registry: every objective and prerequisite of a mission is a *kind* — a named,
 * schema-validated entry in a registry (the "clauses" of a mission spec). Adding a
 * capability means adding one file here; the pipeline (create → accept → progress →
 * reward) never hardcodes mission rules.
 *
 * Two evaluation modes:
 * - `service` the mission service measures progress against Inventory/player-state
 *             through the `measure` handler (called by the verify endpoint);
 * - `issuer`  the mission creator confirms completion (subjective work, Eco's custom
 *             clause).
 *
 * There is deliberately **no** `game` mode: the game server never certifies a mission
 * objective (the server topology changes constantly, an objective bound to a server is
 * invalidated at every split). The game only feeds the player-state store and the
 * inventory; the service decides.
 */
import type { z } from 'zod';

import type { AssigneeHolderType, MissionCategory, MissionObjective } from '../db/schema/index.js';

/** Who evaluates an objective's progress. */
export type EvaluationMode = 'service' | 'issuer';

/**
 * A player-state row, preloaded by the caller. Kinds read it through
 * `loadPlayerState()` instead of issuing one SELECT per objective.
 */
export interface PrefetchedPlayerState {
  playerId: string;
  place: { system?: string; scene?: string; x?: number; y?: number; z?: number } | null;
  onFoot: boolean;
  passenger: boolean;
  carried: Record<string, number>;
  receivedAt: Date;
}

/** Minimal JSON Schema fragment served by `GET /api/missions/kinds`. */
export type JsonSchema = Record<string, unknown>;

/**
 * Mission categories a kind is allowed on. `'all'` means every category; the `divers`
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
  /**
   * Batch measurement only: player states preloaded in one SELECT. Kinds read it through
   * `loadPlayerState()`, which falls back to the database on a miss (a passenger's state
   * is usually not in the batch) and, when the cache is absent at all, always reads the
   * database — so a single-objective measurement is never slower than before.
   */
  stateCache?: ReadonlyMap<string, PrefetchedPlayerState | null>;
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
  /**
   * How `measure()` progress is stored:
   * - `snapshot` (default): stored = measured (can regress if the holder loses goods);
   * - `highWater`: stored = max(stored, measured) (never regresses; used for delivery,
   *   transport and passenger objectives where a completed delivery stays complete).
   */
  progress?: 'snapshot' | 'highWater';
  /**
   * `measure()` issues a remote read (an Inventory/Economy HTTP call) rather than a local
   * database one. The auto-measure triggered by player-state pushes budgets these: a
   * remote read costs an HTTP round trip per assignee, so the number served per push is
   * capped and spread over successive pushes. Purely informational for the catalogue.
   */
  remoteRead?: boolean;
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
