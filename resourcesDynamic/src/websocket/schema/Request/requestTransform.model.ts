import { z } from "zod";

export const requestTransformSchema = z.object({
  duration_s: z.number().min(1),
  frequency: z.number().min(0),
  from_timestamp: z.number().min(0),
});

export type RequestTransformType = z.infer<typeof requestTransformSchema>;
