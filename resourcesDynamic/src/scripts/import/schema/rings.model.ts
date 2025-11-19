import { z } from "zod";

import { ringAnnulusSchema } from "./ringAnnulus.model";

export const ringsSchema = z.object({
  annuli_detail: z.array(ringAnnulusSchema),
  annuli_km: z.array(z.string()),
  color: z.string(),
  dominant_resource: z.string(),
  gaps_km: z.array(z.string()),
  inner_km: z.number(),
  outer_km: z.number(),
  thickness_km: z.number(),
  tilt_rel_deg: z.number(),
  opacity: z.number(),
  source: z.string(),
  resources_total: z.record(z.string(), z.number()),
});

export type RingsType = z.infer<typeof ringsSchema>;
