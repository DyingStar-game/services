import { vector3Schema } from "@lib/math/schema/vector3.model";
import { z } from "zod";

const responseUpdateObjectDataWsSchema = z.object({
  object_type: z.enum(["planet", "moon", "system", "star"]),
  object_uuid: z.uuidv4(),
  object_data: z.object({
    from_timestamp: z.number().int().positive(),
    positions: z.array(vector3Schema).optional(),
    rotations: z.array(vector3Schema).optional(),
  }),
});

export const responseUpdateObjectWsSchema = z.object({
  namespace: z.literal("genericprops"),
  event: z.literal("update_object"),
  data: responseUpdateObjectDataWsSchema,
});

export type ResponseUpdateObjectWsType = z.infer<
  typeof responseUpdateObjectWsSchema
>;
export type ResponseUpdateObjectDataWsType = z.infer<
  typeof responseUpdateObjectDataWsSchema
>;
