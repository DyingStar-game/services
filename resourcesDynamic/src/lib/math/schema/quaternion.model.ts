import { z } from "zod";

export const quaternionSchema = z.object({
  x: z.number(),
  y: z.number(),
  z: z.number(),
  w: z.number(),
});

export type QuaternionType = z.infer<typeof quaternionSchema>;
