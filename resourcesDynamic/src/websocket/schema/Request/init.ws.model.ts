import { z } from "zod";

import { requestTransformSchema } from "./requestTransform.model";

export const requestInitWsSchema = z.object({
  event_type: z.literal("init"),
  data: requestTransformSchema.extend({
    system_internal_name: z.string(),
  }),
});

export type RequestInitWsType = z.infer<typeof requestInitWsSchema>;
