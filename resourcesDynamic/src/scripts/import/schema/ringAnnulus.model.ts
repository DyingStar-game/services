import { z } from "zod";

export const ringAnnulusSchema = z.object({
  mid_km: z.number(),
  range_km: z.string(),
  resources: z.record(z.string(), z.number()),
});
