import { Faker } from "@faker-js/faker";

import { FakerGeneratorFactory } from "./FakerGeneratorFactory";
import { MoonBuilder } from "./builders/MoonBuilder";
import { PlanetBuilder } from "./builders/PlanetBuilder";
import { QuaternionBuilder } from "./builders/QuaternionBuilder";
import { StarBuilder } from "./builders/StarBuilder";
import { SystemBuilder } from "./builders/SystemBuilder";
import { Vector3Builder } from "./builders/Vector3Builder";

export const aVector3 = (generator?: Faker): Vector3Builder => {
  const faker = generator ?? FakerGeneratorFactory.getInstance();

  return new Vector3Builder(faker);
};

export const aQuaternion = (generator?: Faker): QuaternionBuilder => {
  const faker = generator ?? FakerGeneratorFactory.getInstance();

  return new QuaternionBuilder(faker);
};

export const aPlanet = (generator?: Faker): PlanetBuilder => {
  const faker = generator ?? FakerGeneratorFactory.getInstance();

  return new PlanetBuilder(faker);
};

export const aMoon = (generator?: Faker): MoonBuilder => {
  const faker = generator ?? FakerGeneratorFactory.getInstance();

  return new MoonBuilder(faker);
};

export const aStar = (generator?: Faker): StarBuilder => {
  const faker = generator ?? FakerGeneratorFactory.getInstance();

  return new StarBuilder(faker);
};

export const aSystem = (generator?: Faker): SystemBuilder => {
  const faker = generator ?? FakerGeneratorFactory.getInstance();

  return new SystemBuilder(faker);
};
