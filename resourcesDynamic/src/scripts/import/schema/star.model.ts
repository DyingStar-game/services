import { z } from "zod";

export const starSchema = z.object({
  name: z.string(),
  type: z.string(),
  temp_K: z.number(),
  luminosity_Lsun: z.number(),
  luminosity_W: z.number(),
  mass_Sun: z.number(),
  radius_Sun: z.number(),
  age_Gyr: z.number(),
  stage: z.string(),
});

export type StarType = z.infer<typeof starSchema>;
