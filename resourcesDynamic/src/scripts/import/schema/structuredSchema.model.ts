import { z } from "zod";

import { beltSchema } from "./belt.model";
import { planetSchema } from "./planet.model";
import { smallBodySchema } from "./smallBody.model";
import { starSchema } from "./star.model";
import { systemSchema } from "./system.model";

export const structuredSchema = z.object({
  planets: z.array(planetSchema),
  belts: z.array(beltSchema),
  small_bodies: z.array(smallBodySchema),
  stars: z.array(starSchema),
  system: systemSchema,
});
