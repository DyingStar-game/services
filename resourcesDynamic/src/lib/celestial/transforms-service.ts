import { getSystemWithDetailsByInternalName } from "@db/queries";
import type { Moon, Planet, Star, System } from "@db/schema";
import type { TransformType } from "@lib/cache/cache-transform";
import { OrbitDataHelper } from "@lib/celestial/orbit/orbit-data-helper";
import { keplerOrbitService } from "@lib/celestial/orbit/orbit-service";
import { createTimer, logError, logPerformance, logger } from "@lib/logger";
import type { RequestInitWsType } from "@websocket/schema/Request/init.ws.model";
import type { RequestTransformType } from "@websocket/schema/Request/requestTransform.model";
import type { RequestTransformWsType } from "@websocket/schema/Request/transform.ws.model";

let hasInitData: boolean = false;
let systems: System[] = [];
let planets: Planet[] = [];
let stars: Star[] = [];
let moons: Moon[] = [];

type ObjectTypeFrom<T extends Planet | Moon | System | Star> = T extends Planet
  ? "planet"
  : T extends Moon
    ? "moon"
    : T extends Star
      ? "star"
      : "system";

type TransformsResultType<T extends Planet | Moon | System | Star> = {
  object: T;
  transforms: TransformType[];
};

type ObjectDataType<T extends Planet | Moon | System | Star> = {
  target: T;
  parentId: string;
  objectType: ObjectTypeFrom<T>;
  soi?: number;
  transforms?: TransformType[];
};

export const getInit = async (requestInitData: RequestInitWsType["data"]) => {
  const timer = createTimer();

  try {
    await loadData(requestInitData.system_internal_name);

    const systemsData = getAllObjectsData<System>(
      systems,
      requestInitData,
      "system",
    );

    const starsData = getAllObjectsData<Star>(stars, requestInitData, "star");

    const planetsData = getAllObjectsData<Planet>(
      planets,
      requestInitData,
      "planet",
      (target, fromTime, duration, timesteps) =>
        getPlanetUpdateTransforms(target, fromTime, duration, timesteps),
    );

    const moonsData = getAllObjectsData<Moon>(
      moons,
      requestInitData,
      "moon",
      (target, fromTime, duration, timesteps) =>
        getMoonUpdateTransforms(target, fromTime, duration, timesteps),
    );

    logPerformance(logger, `Queries for init`, timer.end());

    return [...systemsData, ...starsData, ...planetsData, ...moonsData];
  } catch (error) {
    logError(logger, error, {
      context: "getInit",
    });
    throw error;
  }
};

const getAllObjectsData = <T extends Planet | Moon | System | Star>(
  dataDb: T[],
  requestTransform: RequestTransformType,
  objectType: ObjectTypeFrom<T>,
  nextTicksCn?: (
    target: string,
    fromTime: number,
    duration: number,
    timesteps: number,
  ) => TransformsResultType<T>,
): ObjectDataType<T>[] => {
  const allData = dataDb.map((data) => {
    return getObjectData<T>(data, requestTransform, objectType, nextTicksCn);
  });

  return allData.filter((item) => item !== null);
};

const getObjectData = <T extends Planet | Moon | System | Star>(
  data: T,
  requestTransform: RequestTransformType,
  objectType: ObjectTypeFrom<T>,
  nextTicksCn?: (
    target: string,
    fromTime: number,
    duration: number,
    timesteps: number,
  ) => TransformsResultType<T>,
): ObjectDataType<T> => {
  let transforms: TransformType[] | undefined;
  if (nextTicksCn) {
    const objectNextTicks = nextTicksCn(
      data.uuid as string,
      requestTransform.from_timestamp,
      requestTransform.duration_s,
      requestTransform.frequency,
    );

    transforms = objectNextTicks.transforms;
  }

  let parentId = "";
  if ("systemId" in data && data.systemId) {
    parentId = getSystem(data.systemId).uuid as string;
  } else if ("planetId" in data && data.planetId) {
    parentId = getPlanet(data.planetId).uuid as string;
  }

  let soi: number | undefined;
  if ("radiusGravityInfluenceM" in data) {
    soi = data.radiusGravityInfluenceM;
  }

  if ("systemId" in data && data.systemId) {
    parentId = getSystem(data.systemId).uuid as string;
  } else if ("planetId" in data && data.planetId) {
    parentId = getPlanet(data.planetId).uuid as string;
  }

  return {
    target: data,
    parentId,
    objectType,
    ...(transforms && { transforms }),
    ...(soi && { soi }),
  };
};

export const getUpdateObject = (
  requestTransformData: RequestTransformWsType["data"],
) => {
  const timer = createTimer();
  let objectData: ObjectDataType<Planet | Moon>;

  const target = requestTransformData.uuid;
  const planet = getPlanetByUuid(target);
  const moon = getMoonByUuid(target);

  try {
    if (planet) {
      objectData = getObjectData<Planet>(
        planet,
        requestTransformData,
        "planet",
        (target, fromTime, duration, timesteps) =>
          getPlanetUpdateTransforms(target, fromTime, duration, timesteps),
      );
    } else if (moon) {
      objectData = getObjectData<Moon>(
        moon,
        requestTransformData,
        "moon",
        (target, fromTime, duration, timesteps) =>
          getMoonUpdateTransforms(target, fromTime, duration, timesteps),
      );
    } else {
      logger.error({ target }, `Object not found: ${target}`);
      throw new Error(`Object not found not found: ${target}`);
    }

    if (!objectData) {
      logger.error({ target }, `Update object not found for target: ${target}`);
      throw new Error(`Update object not found for target: ${target}`);
    }

    logPerformance(logger, `Query for ${objectData.target.name}`, timer.end(), {
      ...(objectData.transforms && {
        transformCount: objectData.transforms.length,
      }),
      target: objectData.target,
    });

    return objectData;
  } catch (error) {
    logError(logger, error, {
      context: "getUpdateObject",
      target,
      fromTime: requestTransformData.from_timestamp,
    });
    throw error;
  }
};

const getSystem = (systemId: number): System => {
  const system = systems.find((system) => system.id === systemId);

  if (!system) {
    logger.error(`Star not found: ${systemId}`);
    throw new Error(`Star not found: ${systemId}`);
  }

  return system;
};

const getStar = (target: string, systemId: number): Star => {
  const star = stars.find((star) => star.systemId === systemId);

  if (!star) {
    logger.error(`Star not found: ${target}`);
    throw new Error(`Star not found: ${target}`);
  }

  return star;
};

const getPlanet = (planetId: number): Planet => {
  const planet = planets.find((planet) => planet.id === planetId);

  if (!planet) {
    logger.error(`Planet not found: ${planetId}`);
    throw new Error(`Star not found: ${planetId}`);
  }

  return planet;
};

const getPlanetByUuid = (uuid: string): Planet | undefined => {
  return planets.find((planet) => planet.uuid === uuid);
};

const getPlanetUpdateTransforms = (
  target: string,
  fromTime: number,
  duration: number,
  frequency: number,
): TransformsResultType<Planet> => {
  const planet = planets.find((planet) => planet.uuid === target);

  if (!planet || !planet.systemId || !systems) {
    logger.error(
      { context: "getPlanetNextTicks" },
      `Planet not found: ${target}`,
    );
    throw new Error(`Planet not found: ${target}`);
  }

  const star = getStar(target, planet.systemId);

  const orbitalCalculation = OrbitDataHelper.createPlanetParamsFromDB(
    planet,
    star,
    fromTime,
    duration,
    frequency,
  );

  logger.debug(
    {
      planet,
      target,
      fromTime,
      duration,
      orbitalInfo: OrbitDataHelper.getOrbitalInfo(
        orbitalCalculation.orbitalObject,
      ),
    },
    "Querying planet positions",
  );

  return {
    object: planet,
    transforms: keplerOrbitService.getTransforms(orbitalCalculation),
  };
};

const getMoonByUuid = (uuid: string): Moon | undefined => {
  return moons.find((moon) => moon.uuid === uuid);
};

const getMoonUpdateTransforms = (
  target: string,
  fromTime: number,
  duration: number,
  timesteps: number,
): TransformsResultType<Moon> => {
  const moon = moons.find((moon) => moon.uuid === target);

  if (!moon || !moon.planetId) {
    logger.error({ context: "getMoonNextTicks" }, `Moon not found: ${target}`);
    throw new Error(`Moon not found: ${target}`);
  }

  const moonPlanet = getPlanet(moon.planetId);

  if (!moonPlanet || !moonPlanet.systemId) {
    logger.error(
      { moon },
      `Planet for moon "${moon.name}" not found: ${target}`,
    );
    throw new Error(`Planet for moon "${moon.name}" not found: ${target}`);
  }

  const moonPlanetPositions = getPlanetUpdateTransforms(
    moonPlanet.uuid as string,
    fromTime,
    duration,
    timesteps,
  );

  if (!moonPlanetPositions) {
    logger.error({ moon }, `No positions for planet for moon ${moon.name}`);
    throw new Error(`No positions for planet for moon ${moon.name}`);
  }

  const orbitalCalculation = OrbitDataHelper.createMoonParamsFromDB(
    moon,
    moonPlanet,
    fromTime,
    duration,
    timesteps,
  );

  logger.debug(
    {
      moon,
      target,
      fromTime,
      duration,
      orbitalInfo: OrbitDataHelper.getOrbitalInfo(
        orbitalCalculation.orbitalObject,
      ),
    },
    "Querying moon transforms",
  );

  // Si besoin d'une position globale.
  //   const transforms = keplerOrbitService
  //   .getTransforms(orbitalCalculation)
  //   .map((transform, index) => {
  //     const newPosition = Vector3Math.add(
  //       moonPlanetPositions.transforms[index].position,
  //       transform.position,
  //     );

  //     return { ...transform, position: newPosition };
  //   });
  // return { object: moon, transforms };

  // Position relatif au parent
  return {
    object: moon,
    transforms: keplerOrbitService.getTransforms(orbitalCalculation),
  };
};

const loadData = async (systemInternalName?: string) => {
  if (!hasInitData && systemInternalName) {
    const systemsWithDetails =
      await getSystemWithDetailsByInternalName(systemInternalName);

    if (!systemsWithDetails) {
      logger.error(
        { systemInternalName },
        `System not found ${systemInternalName}`,
      );
      throw new Error(`System not found ${systemInternalName}`);
    }

    planets = systemsWithDetails.planets.map(
      ({ moons: _moons, ...planetWithoutMoons }) => planetWithoutMoons,
    );

    moons = systemsWithDetails.planets.flatMap((planet) => planet.moons);

    stars = systemsWithDetails.stars;

    systems = [
      (({ id, uuid, name, internalName }) => ({
        id,
        uuid,
        name,
        internalName,
      }))(systemsWithDetails),
    ];

    hasInitData = true;
  }
};
