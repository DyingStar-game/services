/**
 * Shared zod schemas for route validation.
 */
import { z } from 'zod';

import {
  GOOD_KINDS,
  HOLD_KINDS,
  HOLD_REF_TYPES,
  HOLDER_TYPES,
  INSTANCE_STATUSES,
  POI_GRANTEE_TYPES,
  POI_OWNER_TYPES,
  POI_VISIBILITIES,
} from '../db/schema/index.js';

export const uuidSchema = z.string().uuid();

export const holderParams = z.object({
  holderType: z.enum(HOLDER_TYPES),
  holderId: uuidSchema,
});

export const holderStackParams = holderParams.extend({
  goodType: z.string().trim().min(1).max(128),
});

export const corporationParams = z.object({ corporationId: uuidSchema });

export const corporationStackParams = corporationParams.extend({
  goodType: z.string().trim().min(1).max(128),
});

export const corporationPoiParams = corporationParams.extend({ poiId: uuidSchema });

export const politicalEntityParams = z.object({ entityId: uuidSchema });

export const politicalPoiParams = politicalEntityParams.extend({ poiId: uuidSchema });

export const instanceParams = holderParams.extend({
  instanceId: uuidSchema,
});

/** Good type key; opaque string defined by the game/market catalog. */
const goodType = z.string().trim().min(1).max(128);

/** Quantities are positive integers. */
const quantity = z.number().int().min(1).max(10_000_000_000_000);

export const creditBody = z.object({
  goodType,
  quantity,
});

export const registerInstanceBody = z.object({
  instanceId: uuidSchema,
  goodType,
  metadata: z.record(z.string(), z.unknown()).optional(),
});

export const instanceStatusBody = z.object({
  status: z.enum(INSTANCE_STATUSES),
});

export const transferStackBody = z.object({
  from: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  to: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  goodType,
  quantity,
});

export const transferInstanceBody = z.object({
  from: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  to: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
  instanceId: uuidSchema,
});

export const createHoldBody = z
  .object({
    kind: z.enum(HOLD_KINDS),
    goodType,
    quantity: quantity.optional(),
    instanceId: uuidSchema.optional(),
    refType: z.enum(HOLD_REF_TYPES),
    refId: z.string().trim().max(128).optional(),
    expiresAt: z.coerce.date().optional(),
    details: z.record(z.string(), z.unknown()).optional(),
  })
  .refine((v) => (v.kind === 'stack' ? v.quantity !== undefined : v.instanceId !== undefined), {
    message: 'stack holds need quantity; instance holds need instanceId',
  });

export const consumeHoldBody = z.object({
  to: z.object({ holderType: z.enum(HOLDER_TYPES), holderId: uuidSchema }),
});

export const holdIdParams = z.object({ holdId: uuidSchema });

export const limitQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/** Bodies reused by player routes (holder derived from the JWT). */
export const playerCreditBody = creditBody;
export const playerRegisterInstanceBody = registerInstanceBody;
export const playerHoldBody = createHoldBody;

// ── POIs ─────────────────────────────────────────────────────────────────────

export const poiParams = z.object({ poiId: uuidSchema });

export const poiShareParams = poiParams.extend({
  granteeType: z.enum(POI_GRANTEE_TYPES),
  granteeId: uuidSchema,
});

/** POI holder params (wider than goods holders: political entities own POIs). */
export const poiHolderParams = z.object({
  holderType: z.enum(POI_OWNER_TYPES),
  holderId: uuidSchema,
});

const poiName = z.string().trim().min(1).max(128);
const poiDescription = z.string().trim().max(2000);
const poiSystem = z.string().trim().min(1).max(64);
const poiScene = z.string().trim().min(1).max(128);
/** Game units, assumed metres (same convention as mission zones). */
const poiCoordinate = z.number();
const poiRadius = z.number().min(0).max(1_000_000);
const poiMetadata = z.record(z.string(), z.unknown());

/** Shared patchable fields (each optional; at least one required by `updatePoiBody`). */
const poiPatchableFields = {
  name: poiName.optional(),
  description: poiDescription.nullish(),
  system: poiSystem.nullish(),
  scene: poiScene.nullish(),
  parentId: uuidSchema.nullish(),
  x: poiCoordinate.optional(),
  y: poiCoordinate.optional(),
  z: poiCoordinate.optional(),
  radiusM: poiRadius.nullish(),
  visibility: z.enum(POI_VISIBILITIES).optional(),
  metadata: poiMetadata.nullish(),
};

/** Creation body for a player-owned (or organization-owned) POI. */
export const createPoiBody = z.object({
  name: poiName,
  description: poiDescription.optional(),
  system: poiSystem.optional(),
  scene: poiScene.optional(),
  parentId: uuidSchema.optional(),
  x: poiCoordinate,
  y: poiCoordinate,
  z: poiCoordinate,
  radiusM: poiRadius.optional(),
  visibility: z.enum(POI_VISIBILITIES).optional(),
  metadata: poiMetadata.optional(),
  /** Organization owning the created POI (default: the caller). */
  owner: z.object({ type: z.enum(['corporation', 'political']), id: uuidSchema }).optional(),
});

/** Creation body on the internal API: the owner is always explicit. */
export const internalCreatePoiBody = z.object({
  owner: z.object({ type: z.enum(POI_OWNER_TYPES), id: uuidSchema }),
  name: poiName,
  description: poiDescription.optional(),
  system: poiSystem.optional(),
  scene: poiScene.optional(),
  parentId: uuidSchema.optional(),
  x: poiCoordinate,
  y: poiCoordinate,
  z: poiCoordinate,
  radiusM: poiRadius.optional(),
  visibility: z.enum(POI_VISIBILITIES).optional(),
  metadata: poiMetadata.optional(),
});

/** Partial update; at least one field must be present (an empty patch is a client bug). */
export const updatePoiBody = z
  .object(poiPatchableFields)
  .refine((value) => Object.values(value).some((field) => field !== undefined), {
    message: 'at least one field is required',
  });

export const sharePoiBody = z.object({
  granteeType: z.enum(POI_GRANTEE_TYPES),
  granteeId: uuidSchema,
});

/** Ownership transfer targets: every POI holder type except `system`. */
export const transferPoiBody = z.object({
  toType: z.enum(POI_GRANTEE_TYPES),
  toId: uuidSchema,
});

export const resolvePoisBody = z.object({
  ids: z.array(uuidSchema).min(1).max(100),
});

export type CreateHoldBody = z.infer<typeof createHoldBody>;
export type CreatePoiBody = z.infer<typeof createPoiBody>;
export type UpdatePoiBody = z.infer<typeof updatePoiBody>;
export { GOOD_KINDS };
