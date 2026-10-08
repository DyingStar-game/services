import { eq } from "drizzle-orm";

import { db } from "../connection";
import type { NewStar } from "../schema";
import { type Star, stars } from "../schema";

export const getAllStars = async (): Promise<Star[]> => {
  return await db.select().from(stars);
};

export const getStarsBySystemId = async (systemId: number) => {
  return await db.query.planets.findMany({
    where: eq(stars.systemId, systemId),
    with: {
      moons: true,
    },
  });
};

export const createStar = async (star: NewStar): Promise<Star> => {
  const result = await db.insert(stars).values(star).returning();
  return result[0];
};

/**
 * Insert or update a star identified by its internal name (keeps id and uuid)
 */
export const upsertStar = async (star: NewStar): Promise<Star> => {
  const { uuid: _uuid, ...data } = star;
  const result = await db
    .insert(stars)
    .values(star)
    .onConflictDoUpdate({ target: stars.internalName, set: data })
    .returning();
  return result[0];
};
