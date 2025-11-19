import { relations } from "drizzle-orm";
import { integer, pgTable, text, uuid } from "drizzle-orm/pg-core";
import {
  createInsertSchema,
  createSelectSchema,
  createUpdateSchema,
} from "drizzle-zod";
import { z } from "zod";

import { planetWithMoonsSchema, planets } from "./planets";
import { starSchema, stars } from "./stars";

export const systems = pgTable("systems", {
  id: integer("id").primaryKey().generatedAlwaysAsIdentity(),
  uuid: uuid("uuid").defaultRandom().unique(),
  name: text("name").notNull().unique(),
  internalName: text("internal_name").notNull().unique(),
});

// Relations
export const systemsRelations = relations(systems, ({ many }) => ({
  stars: many(stars),
  planets: many(planets),
}));

export const systemSchema = createSelectSchema(systems);
export const createSystemSchema = createInsertSchema(systems);
export const updateSystemSchema = createUpdateSchema(systems);
export const systemWithDetailsSchema = createSelectSchema(systems).extend({
  planets: z.array(planetWithMoonsSchema),
  stars: z.array(starSchema),
});

export type System = z.infer<typeof systemSchema>;
export type NewSystem = z.infer<typeof createSystemSchema>;
export type UpdateSystem = z.infer<typeof updateSystemSchema>;
export type SystemWithDetails = z.infer<typeof systemWithDetailsSchema>;
