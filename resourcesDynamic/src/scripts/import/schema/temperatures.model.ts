import { z } from "zod";

export const temperaturesSchema = z.object({
  T_day_K: z.number().nullable(),
  T_night_K: z.number().nullable(),
  T_surface_K: z.number().nullable().optional(),
  Teq_K: z.number().nullable(),
});
