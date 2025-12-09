import { z } from "zod";

import { atmosphereSchema } from "./atmosphere.model";
import { compositionSchema } from "./composition.model";
import { resourcesSchema } from "./resources.model";
import { temperaturesSchema } from "./temperatures.model";

export const moonSchema = z.object({
  M0_deg: z.number(),
  name: z.string(),
  index: z.number(),
  albedo: z.number().nullable(),
  radius_km: z.number(),
  radius_Re: z.number(),
  mass_Me: z.number(),
  mass_kg: z.number(),
  semi_major_km: z.number(),
  apoapsis_km: z.number(),
  periapsis_km: z.number(),
  eccentricity: z.number(),
  inclination_deg: z.number(),
  arg_peri_deg: z.number(),
  ascending_node_deg: z.number(),
  spin_locked: z.boolean(),
  spin_period_h: z.number(),
  period_days: z.number(),
  atmosphere: atmosphereSchema,
  composition_bulk_pct: compositionSchema,
  resources: resourcesSchema,
  temperatures: temperaturesSchema,
});

export type MoonType = z.infer<typeof moonSchema>;
