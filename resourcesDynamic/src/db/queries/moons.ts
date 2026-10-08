import { eq } from "drizzle-orm";

import { db } from "../connection";
import type { Moon, NewMoon } from "../schema";
import { moons, planets } from "../schema";

export const getAllMoons = async (): Promise<Moon[]> => {
  return await db.select().from(moons);
};

export const getMoonsByPlanetId = async (planetId: number) => {
  return await db.query.moons.findMany({
    where: eq(planets.id, planetId),
  });
};

export const createMoon = async (moon: NewMoon): Promise<Moon> => {
  const result = await db.insert(moons).values(moon).returning();
  return result[0];
};

/**
 * Insert or update a moon identified by its internal name (keeps id and uuid)
 */
export const upsertMoon = async (moon: NewMoon): Promise<Moon> => {
  const { uuid: _uuid, ...data } = moon;
  const result = await db
    .insert(moons)
    .values(moon)
    .onConflictDoUpdate({ target: moons.internalName, set: data })
    .returning();
  return result[0];
};
