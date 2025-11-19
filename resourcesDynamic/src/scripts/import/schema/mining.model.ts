import { z } from "zod";

export const miningSchema = z.record(
  z.string(),
  z.object({
    mines: z.number().nullable(),
    total_tonnes: z.number().nullable(),
    typical_mine_tonnage: z.number().nullable(),
  }),
);

export type MiningType = z.infer<typeof miningSchema>;
