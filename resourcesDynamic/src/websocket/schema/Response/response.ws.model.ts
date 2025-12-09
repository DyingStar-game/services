import { z } from "zod";

import { connectedMessageWsSchema } from "./connected.ws.model";
import { errorMessageWsSchema } from "./error.ws.model";
import { responseInitWsSchema } from "./init.ws.model";
import { responseUpdateObjectWsSchema } from "./updateObject.ws.model";

export const responseWsSchema = z.union([
  responseInitWsSchema,
  connectedMessageWsSchema,
  errorMessageWsSchema,
  responseUpdateObjectWsSchema,
]);

export type ResponseWsType = z.infer<typeof responseWsSchema>;
