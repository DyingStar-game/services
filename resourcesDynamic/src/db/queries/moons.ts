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
