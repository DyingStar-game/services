import { z } from "zod";

export const connectedMessageWsSchema = z.object({
  type: z.literal("connected"),
  clientId: z.string(),
  timestamp: z.number(),
});

export type ConnectedMessageWsType = z.infer<typeof connectedMessageWsSchema>;
