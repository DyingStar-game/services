import { eq } from "drizzle-orm";
import { z } from "zod";

import { db } from "../connection";
import {
  type NewPlanet,
  type Planet,
  type UpdatePlanet,
  moons,
  planets,
} from "../schema";

export const getAllPlanets = async (): Promise<Planet[]> => {
  return await db.select().from(planets);
};

export const getPlanetByUuid = async (uuid: string) => {
  const validated = z.uuid().parse(uuid);
  return await db.query.planets.findFirst({
    where: eq(planets.uuid, validated),
    with: {
      system: true,
      moons: true,
    },
  });
};

export const getPlanetsBySystemId = async (systemId: number) => {
  return await db.query.planets.findMany({
    where: eq(planets.systemId, systemId),
    with: {
      moons: true,
    },
  });
};

export const getPlanetsByMoonUuId = async (moonUuid: string) => {
  const result = await db.query.planets.findMany({
    where: eq(moons.uuid, moonUuid),
  });
  return result[0];
};

export const createPlanet = async (planet: NewPlanet): Promise<Planet> => {
  const result = await db.insert(planets).values(planet).returning();
  return result[0];
};

export const updatePlanet = async (
  uuid: string,
  data: UpdatePlanet,
): Promise<Planet | undefined> => {
  const validatedUuid = z.uuid().parse(uuid);
  const result = await db
    .update(planets)
    .set(data)
    .where(eq(planets.uuid, validatedUuid))
    .returning();
  return result[0];
};

export const deletePlanet = async (uuid: string): Promise<boolean> => {
  const validated = z.uuid().parse(uuid);
  const result = await db
    .delete(planets)
    .where(eq(planets.uuid, validated))
    .returning();
  return result.length > 0;
};
