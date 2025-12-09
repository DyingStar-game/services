import { z } from "zod";

export const errorMessageWsSchema = z.object({
  error: z.string(),
  message: z.string(),
});

export type ErrorMessageWsType = z.infer<typeof errorMessageWsSchema>;
