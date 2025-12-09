import { z } from "zod";

import { atmosphereSchema } from "./atmosphere.model";
import { compositionSchema } from "./composition.model";
import { moonSchema } from "./moon.model";
import { resourcesSchema } from "./resources.model";
import { ringsSchema } from "./rings.model";
import { temperaturesSchema } from "./temperatures.model";

export const planetSchema = z.object({
  M0_deg: z.number(),
  apoapsis_AU: z.number(),
  periapsis_AU: z.number(),
  semi_major_AU: z.number(),
  eccentricity: z.number(),
  inclination_deg: z.number(),
  arg_peri_deg: z.number(),
  ascending_node_deg: z.number(),
  type: z.string(), // e.g. "Terrestrial", "GasGiant"
  albedo: z.number(),
  radius_km: z.number(),
  radius_Re: z.number(),
  density_kg_m3: z.number(),
  mass_Me: z.number(),
  rotation_h: z.number(),
  tilt_deg: z.number(),
  tidal_locked: z.boolean(),
  orbit_center: z.string(),
  atmosphere: atmosphereSchema.nullable(),
  composition_bulk_pct: compositionSchema.nullable(),
  resources: resourcesSchema.optional(),
  rings: z.union([ringsSchema, z.boolean(), z.null()]),
  moons: z.array(moonSchema).optional(),
  temperatures: temperaturesSchema,
});

export type PlanetType = z.infer<typeof planetSchema>;
