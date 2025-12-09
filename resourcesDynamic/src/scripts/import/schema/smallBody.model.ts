import { z } from "zod";

export const smallBodySchema = z.object({
  type: z.string(),
  albedo: z.number().nullable(),
  radius_km: z.number().nullable(),
  period_days: z.number().nullable(),
  period_years: z.number().nullable(),
  composition_pct: z.record(z.string(), z.number().nullable()),
  orbit: z.object({
    a_AU: z.number().nullable(),
    eccentricity: z.number().nullable(),
    inclination_deg: z.number().nullable(),
    ascending_node_deg: z.number().nullable(),
    orbit_center: z.string().nullable(),
  }),
});
