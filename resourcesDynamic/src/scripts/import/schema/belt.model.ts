import { z } from "zod";

import { resourcesSchema } from "./resources.model";

export const beltSchema = z.object({
  source: z.string(),
  inner_AU: z.number(),
  outer_AU: z.number(),
  mass_Me: z.number(),
  inclination_deg: z.number(),
  ascending_node_deg: z.number(),
  resources: resourcesSchema,
});

export type BeltType = z.infer<typeof beltSchema>;
