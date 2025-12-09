import { z } from "zod";

export const systemSchema = z.object({
  name: z.string(),
  seed: z.string(),
  age_Gyr: z.number(),
  binary: z.boolean(),
  snow_line_AU: z.number(),
});

export type SystemType = z.infer<typeof systemSchema>;
