import { z } from "zod";

export const compositionSchema = z.record(z.string(), z.number().nullable());
