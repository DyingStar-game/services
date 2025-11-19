import { z } from "zod";

import { requestInitWsSchema } from "./init.ws.model";
import { requestTransformWsSchema } from "./transform.ws.model";

export const requestWsSchema = z.discriminatedUnion("event_type", [
  requestInitWsSchema,
  requestTransformWsSchema,
]);

export type RequestWsType = z.infer<typeof requestWsSchema>;
