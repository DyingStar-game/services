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
import { z } from "zod";

import { moonSchema, moons } from "./moons";
import { systems } from "./systems";

export const planets = pgTable("planets", {
  id: integer("id").primaryKey().generatedAlwaysAsIdentity(),
  uuid: uuid("uuid").defaultRandom().unique(),
  systemId: integer("system_id").references(() => systems.id),
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
export const planetsRelations = relations(planets, ({ one, many }) => ({
  system: one(systems, {
    fields: [planets.systemId],
    references: [systems.id],
  }),
  moons: many(moons),
}));

export const planetSchema = createSelectSchema(planets);
export const createPlanetSchema = createInsertSchema(planets);
export const updatePlanetSchema = createUpdateSchema(planets);
export const planetWithMoonsSchema = createSelectSchema(planets).extend({
  moons: z.array(moonSchema),
});

export type Planet = z.infer<typeof planetSchema>;
export type NewPlanet = z.infer<typeof createPlanetSchema>;
export type UpdatePlanet = z.infer<typeof updatePlanetSchema>;
export type PlanetWithMoon = z.infer<typeof planetWithMoonsSchema>;
