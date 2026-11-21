/**
 * Inventory ownership model (shared holder concept).
 *
 * A *holder* is whoever owns goods: a player, an NPC, a corporation (treasury), a
 * `warehouse` (village/city stock run by the universe simulator) or the reserved `system`.
 * Ids are opaque UUIDs — players/NPCs come from the Social service and corporations from
 * Social too; a warehouse id is the UUID of its persistence game object (POI). This service
 * keeps no local copy of those tables. `warehouse` holders are only reachable through
 * `/api/internal/*` (player routes derive their holder from the JWT/corporation).
 */

/** Kinds of inventory holders, mirroring the Economy account holder types (+ `warehouse`). */
export const HOLDER_TYPES = ['player', 'npc', 'corporation', 'political', 'system', 'warehouse'] as const;
export type HolderType = (typeof HOLDER_TYPES)[number];

/** Minimal holder reference used across the service. */
export interface HolderRef {
  holderType: HolderType;
  holderId: string;
}
