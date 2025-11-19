import { z } from "zod";

import { miningSchema } from "./mining.model";

export const resourcesSchema = z
  .object({
    accessibility: z.string().nullable(),
    budget_tonnes: z.number().nullable(),
    mining: miningSchema.optional(),
  })
  .catchall(z.number().nullable());

export type ResourcesType = z.infer<typeof resourcesSchema>;
