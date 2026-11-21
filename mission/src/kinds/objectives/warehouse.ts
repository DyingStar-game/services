/**
 * What `deliver_material` and `transport` share when they are measured on a warehouse
 * (`via: 'warehouse'`): the param, its JSON schema, and the progress read from the objective's
 * state (what the goods moves put into the warehouse — see goodsMoves.service).
 */
import { z } from 'zod';

import type { MeasureContext } from '../types.js';

/**
 * How delivery is measured. `zone` (default): the goods the assignee carries while inside the zone
 * POI. `warehouse`: the goods put INTO the zone's warehouse (the POI's `metadata.persistence`),
 * reported by the game server as goods moves — walking in with the crates is not delivering them.
 */
export const viaSchema = z.enum(['zone', 'warehouse']).default('zone');

export const viaJsonSchema = {
  type: 'string',
  enum: ['zone', 'warehouse'],
  default: 'zone',
  description: "'zone' = carried inside the zone; 'warehouse' = put into the zone's warehouse (goods moves)",
} as const;

/** Progress of a `via: 'warehouse'` objective: what the goods moves put into its warehouse, capped. */
export function warehouseProgress(ctx: MeasureContext): number {
  const delivered = Number(((ctx.objective.state ?? {}) as { delivered?: number }).delivered ?? 0);
  return Math.min(delivered, ctx.objective.targetQuantity);
}
