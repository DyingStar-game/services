import { z } from "zod";

export const atmosphereSchema = z.object({
  gases_pct: z.record(z.string(), z.number().nullable()).optional(),
  pressure_bar: z.number().nullable().optional(),
  thickness_km: z.number().nullable().optional(),
  meta: z
    .object({
      pressure_bar: z.number().nullable(),
      scale_height_km: z.number().nullable(),
      thickness_km: z.number().nullable(),
    })
    .optional(),
});

export type AtmosphereType = z.infer<typeof atmosphereSchema>;
