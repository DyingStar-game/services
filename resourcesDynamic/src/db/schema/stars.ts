import { relations } from "drizzle-orm";
import {
  doublePrecision,
  integer,
  pgTable,
  text,
  uuid,
} from "drizzle-orm/pg-core";
import {
  createInsertSchema,
  createSelectSchema,
  createUpdateSchema,
} from "drizzle-zod";
import type { z } from "zod";

import { systems } from "./systems";

export const stars = pgTable("stars", {
  id: integer("id").primaryKey().generatedAlwaysAsIdentity(),
  uuid: uuid("uuid").defaultRandom().unique(),
  systemId: integer("system_id").references(() => systems.id),
  name: text("name").notNull().unique(),
  internalName: text("internal_name").notNull().unique(),
  massKg: doublePrecision("mass_kg").notNull(),
});

// Relations
export const starsRelations = relations(stars, ({ one }) => ({
  system: one(systems, {
    fields: [stars.systemId],
    references: [systems.id],
  }),
}));

export const starSchema = createSelectSchema(stars);
export const createStarSchema = createInsertSchema(stars);
export const updateStarSchema = createUpdateSchema(stars);

export type Star = z.infer<typeof starSchema>;
export type NewStar = z.infer<typeof createStarSchema>;
export type UpdateStar = z.infer<typeof updateStarSchema>;
