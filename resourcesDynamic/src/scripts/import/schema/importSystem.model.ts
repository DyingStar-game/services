import { z } from "zod";

import { structuredSchema } from "./structuredSchema.model";

export const importSystemSchema = z.object({
  flat: z.record(z.string(), z.union([z.number(), z.string()])),
  structured: structuredSchema,
});

export type ImportSystemType = z.infer<typeof importSystemSchema>;
export type FlatType = ImportSystemType["flat"];
