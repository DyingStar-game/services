/**
 * Inventory ownership model (shared holder concept).
 *
 * A *holder* is whoever owns goods: a player, an NPC, a corporation (treasury) or the
 * reserved `system`. Ids are opaque UUIDs — players/NPCs come from the Social service and
 * corporations from Social too; this service keeps no local copy of those tables.
 */

/** Kinds of inventory holders, mirroring the Economy account holder types. */
export const HOLDER_TYPES = ['player', 'npc', 'corporation', 'system'] as const;
export type HolderType = (typeof HOLDER_TYPES)[number];

/** Minimal holder reference used across the service. */
export interface HolderRef {
  holderType: HolderType;
  holderId: string;
}
