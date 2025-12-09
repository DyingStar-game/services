import { relations } from "drizzle-orm";
import {
  boolean,
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

import { planets } from "./planets";

export const moons = pgTable("planet_moons", {
  id: integer("id").primaryKey().generatedAlwaysAsIdentity(),
  planetId: integer("planet_id").references(() => planets.id),
  uuid: uuid("uuid").defaultRandom().unique(),
  name: text("name").notNull().unique(),
  internalName: text("internal_name").notNull().unique(),
  massKg: doublePrecision("mass_kg").notNull(),
  periapsisAu: doublePrecision("periapsis_au").notNull(),
  apoapsisAu: doublePrecision("apoapsis_au").notNull(),
  incRad: doublePrecision("inc_rad").notNull(),
  nodeRad: doublePrecision("node_rad").notNull(),
  argPeriRad: doublePrecision("arg_peri_rad").notNull(),
  meanAnomalyRad: doublePrecision("mean_anomaly_rad").notNull(),
  radiusM: doublePrecision("radius_m").notNull(),
  radiusGravityInfluenceM: doublePrecision(
    "radius_gravity_influence_m",
  ).notNull(),
  tidalLocked: boolean("tidal_locked").notNull(),
  tiltRad: doublePrecision("tilt_rad").notNull(),
  rotationH: doublePrecision("rotation_h").notNull(),
});

// Relations
export const moonsRelations = relations(moons, ({ one }) => ({
  planet: one(planets, {
    fields: [moons.planetId],
    references: [planets.id],
  }),
}));

export const moonSchema = createSelectSchema(moons);
export const createMoonSchema = createInsertSchema(moons);
export const updateMoonSchema = createUpdateSchema(moons);

export type Moon = z.infer<typeof moonSchema>;
export type NewMoon = z.infer<typeof createMoonSchema>;
export type UpdateMoon = z.infer<typeof updateMoonSchema>;
