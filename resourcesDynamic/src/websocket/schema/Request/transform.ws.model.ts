import { z } from "zod";

import { requestTransformSchema } from "./requestTransform.model";

export const requestTransformWsSchema = z.object({
  event_type: z.literal("transform"),
  data: requestTransformSchema.extend({
    uuid: z.uuid(),
  }),
});

export type RequestTransformWsType = z.infer<typeof requestTransformWsSchema>;
