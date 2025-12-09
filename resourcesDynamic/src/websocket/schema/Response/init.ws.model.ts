import { vector3Schema } from "@lib/math/schema/vector3.model";
import { z } from "zod";

const responseInitDataWsSchema = z.object({
  object_type: z.enum(["planet", "moon", "system", "star"]),
  object_uuid: z.uuidv4(),
  object_data: z.object({
    name: z.string(),
    scenename: z.string(),
    parent_id: z.string().optional(),
    soi: z.number().optional(),
    from_timestamp: z.number().positive(),
    positions: z.array(vector3Schema).optional(),
    rotations: z.array(vector3Schema).optional(),
  }),
});

export const responseInitWsSchema = z.object({
  namespace: z.literal("genericprops"),
  event: z.literal("create_object"),
  data: z.array(responseInitDataWsSchema),
});

export type ResponseInitWsType = z.infer<typeof responseInitWsSchema>;
export type ResponseInitDataWsType = z.infer<typeof responseInitDataWsSchema>;
