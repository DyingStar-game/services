import { createPlanet, createSystem } from "@db/queries";
import { createMoon } from "@db/queries/moons";
import { createStar } from "@db/queries/stars";
import type {
  Moon,
  NewMoon,
  NewPlanet,
  NewStar,
  NewSystem,
  Planet,
  Star,
  System,
} from "@db/schema";
import { DrizzleQueryError } from "drizzle-orm";
import { readFileSync, readdirSync } from "fs";
import path from "path";

import type { FlatType } from "./schema/importSystem.model";
import { importSystemSchema } from "./schema/importSystem.model";
import type { MoonType } from "./schema/moon.model";
import type { PlanetType } from "./schema/planet.model";
import type { StarType } from "./schema/star.model";
import type { SystemType } from "./schema/system.model";

const folderPath = path.join(process.cwd(), "data/system");

const files = readdirSync(folderPath);

const jsonFiles = files.filter((file) => file.endsWith(".json"));

const MASS_SUN = 1.989e30;
const MASS_EARTH = 5.972e24;
const AU = 1.496e11;
const DISTANCE_FACTOR = 3;

let flatInfo: FlatType;

const formatDrizzleQueryError = (error: DrizzleQueryError) => {
  return {
    type: "DrizzleQueryError",
    name: error.name,
    cause: error.cause,
  };
};

export const importSystem = async () => {
  for (const file of jsonFiles) {
    const filePath = path.join(folderPath, file);

    // Lire le contenu du fichier
    const jsonString = readFileSync(filePath, "utf8");

    // Parser le JSON
    const data = importSystemSchema.parse(JSON.parse(jsonString));
    flatInfo = data.flat;

    const system = await importSystemDb(data.structured.system);

    const stars = await Promise.all(
      data.structured.stars.map(
        async (star) => await importStarDb(star, system),
      ),
    );

    const moons: Moon[] = [];
    const planets = await Promise.all(
      data.structured.planets.map(async (planet, planetIndex) => {
        const newPlanet = await importPlanetDb(
          planet,
          planetIndex + 1,
          stars[0],
        );

        if (planet.moons && planet.moons.length > 0) {
          await Promise.all(
            planet.moons.map(async (moon, moonIndex) => {
              const newMoon = await importMoonDb(
                moon,
                moonIndex + 1,
                newPlanet,
              );
              if (newMoon) {
                moons.push(newMoon);
              }
            }),
          );
        }

        return newPlanet;
      }),
    );

    console.log(`📄 ${file}: import done`, {
      system: system.name,
      stars: stars.map((star) => star.name),
      planets: planets.map((planet) => planet.name),
      moons: moons.map((moon) => moon.name),
    });
  }
};

const importSystemDb = async (system: SystemType) => {
  const newSystem: NewSystem = {
    name: system.name,
    internalName: system.name.toLowerCase(),
  };
  return await createSystem(newSystem);
};

const importStarDb = async (star: StarType, system: System) => {
  const newStar: NewStar = {
    name: star.name,
    internalName: star.name.toLowerCase(),
    massKg: star.mass_Sun * MASS_SUN,
    systemId: system.id,
  };
  return await createStar(newStar);
};

const getFlatInfo = (planetNumber: number, flatName: string) => {
  if (!flatInfo || typeof flatInfo !== "object") {
    return null;
  }
  const key = `P${planetNumber}_${flatName}`;
  return Object.prototype.hasOwnProperty.call(flatInfo, key)
    ? flatInfo[key]
    : null;
};

const importPlanetDb = async (
  planet: PlanetType,
  planetNumber: number,
  star: Star,
) => {
  const internalName = `${star.name.toLowerCase()}_${planetNumber}`;

  const newPlanet: NewPlanet = {
    name: (getFlatInfo(planetNumber, "Name") ?? internalName).toString(),
    internalName,
    massKg: planet.mass_Me * MASS_EARTH,
    systemId: star.systemId,
    apoapsisAu: planet.apoapsis_AU / DISTANCE_FACTOR,
    periapsisAu: planet.periapsis_AU / DISTANCE_FACTOR,
    argPeriRad: convertDegToRad(planet.arg_peri_deg),
    incRad: convertDegToRad(planet.inclination_deg),
    meanAnomalyRad: convertDegToRad(planet.M0_deg),
    nodeRad: convertDegToRad(planet.ascending_node_deg),
    radiusM: (planet.radius_km * 1000) / DISTANCE_FACTOR,
    radiusGravityInfluenceM:
      getRadiusGravityInfluenceM(planet, star.massKg) / DISTANCE_FACTOR,
    rotationH: planet.rotation_h,
    tidalLocked: planet.tidal_locked,
    tiltRad: convertDegToRad(planet.tilt_deg),
  };
  return await createPlanet(newPlanet);
};

const importMoonDb = async (
  moon: MoonType,
  moonNumber: number,
  planet: Planet,
) => {
  const newMoon: NewMoon = {
    name: moon.name,
    apoapsisAu: (moon.apoapsis_km * 1000) / AU / DISTANCE_FACTOR,
    periapsisAu: (moon.periapsis_km * 1000) / AU / DISTANCE_FACTOR,
    argPeriRad: convertDegToRad(moon.arg_peri_deg),
    incRad: convertDegToRad(moon.inclination_deg),
    internalName: `${planet.internalName.toLowerCase()}_${moonNumber}`,
    massKg: moon.mass_Me * MASS_EARTH,
    meanAnomalyRad: convertDegToRad(moon.M0_deg),
    nodeRad: convertDegToRad(moon.ascending_node_deg),
    radiusM: (moon.radius_km * 1000) / DISTANCE_FACTOR,
    planetId: planet.id,
    radiusGravityInfluenceM:
      getRadiusGravityInfluenceM(moon, planet.massKg) / DISTANCE_FACTOR,
    rotationH: moon.spin_period_h,
    tidalLocked: moon.spin_locked,
    tiltRad: 0, // TODO: Missing data
  };
  return await createMoon(newMoon);
};

const convertDegToRad = (degree: number): number => {
  return (degree * Math.PI) / 180;
};

const getRadiusGravityInfluenceM = (
  object: MoonType | PlanetType,
  primaryMassKg: number,
) => {
  let radiusGravityInfluenceM = 0;
  const objectMassKg = object.mass_Me * MASS_EARTH;
  const semiMajorAxeM =
    "semi_major_km" in object
      ? object.semi_major_km * 1000
      : object.semi_major_AU * AU;
  if (semiMajorAxeM > 0 && primaryMassKg > 0) {
    radiusGravityInfluenceM =
      semiMajorAxeM * Math.pow(objectMassKg / primaryMassKg, 2 / 5);
  }

  return radiusGravityInfluenceM;
};

try {
  await importSystem();
} catch (error) {
  if (error instanceof DrizzleQueryError) {
    console.error(formatDrizzleQueryError(error));
    process.exit(1);
  }
  console.error(error);
  process.exit(1);
}

process.exit(0);
