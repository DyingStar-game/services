import { eq } from "drizzle-orm";

import { db } from "../connection";
import type { UpdateSystem } from "../schema";
import { type NewSystem, type System, systems } from "../schema";

export const getAllSystems = async (): Promise<System[]> => {
  return await db.select().from(systems);
};

export const getAllSystemsWithDetails = async () => {
  return await db.query.systems.findMany({
    with: {
      planets: {
        with: {
          moons: true,
        },
      },
      stars: true,
    },
  });
};

/**
 * Get a system with its planets and stars
 */
export const getSystemWithDetails = async (systemId: number) => {
  return await db.query.systems.findFirst({
    where: eq(systems.id, systemId),
    with: {
      planets: {
        with: {
          moons: true,
        },
      },
      stars: true,
    },
  });
};

export const getSystemWithDetailsByInternalName = async (
  internalName: string,
) => {
  return await db.query.systems.findFirst({
    where: eq(systems.internalName, internalName),
    with: {
      planets: {
        with: {
          moons: true,
        },
      },
      stars: true,
    },
  });
};

export const createSystem = async (system: NewSystem): Promise<System> => {
  const result = await db.insert(systems).values(system).returning();
  return result[0];
};

export const updateSystem = async (
  systemId: number,
  data: UpdateSystem,
): Promise<System | undefined> => {
  const result = await db
    .update(systems)
    .set(data)
    .where(eq(systems.id, systemId))
    .returning();
  return result[0];
};
