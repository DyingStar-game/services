import { aMoon } from "@builder/builders";
import { Moon, planetWithMoonsSchema } from "@db/schema";
import { Faker } from "@faker-js/faker";

import { BuilderBase } from "./BuilderBase";

export class PlanetBuilder extends BuilderBase<typeof planetWithMoonsSchema> {
  constructor(faker: Faker) {
    const planetId = faker.number.int({ min: 1, max: 50000 });
    const countMoon = faker.number.int({ min: 0, max: 3 });
    const moons = Array.from({ length: countMoon }, () =>
      aMoon(faker).withPlanetId(planetId).build(),
    );
    super(planetWithMoonsSchema, {
      id: planetId,
      uuid: faker.string.uuid(),
      systemId: faker.number.int({ min: 1, max: 50000 }),
      name: faker.word.sample(),
      internalName: faker.word.sample(),
      massKg: faker.number.float(),
      periapsisAu: faker.number.float({ min: 0, max: 1 }),
      apoapsisAu: faker.number.float({ min: 0, max: 1 }),
      incRad: faker.number.float({ min: 0, max: 2 * Math.PI }),
      nodeRad: faker.number.float({ min: 0, max: 2 * Math.PI }),
      argPeriRad: faker.number.float({ min: 0, max: 2 * Math.PI }),
      meanAnomalyRad: faker.number.float({ min: 0, max: 2 * Math.PI }),
      radiusM: faker.number.float(),
      radiusGravityInfluenceM: faker.number.float(),
      rotationH: faker.number.int({ min: 1, max: 360 }),
      tidalLocked: faker.datatype.boolean(),
      tiltRad: faker.number.float({ min: 0, max: 2 * Math.PI }),
      moons,
    });
  }

  withId(id: number): this {
    this.data.id = id;
    return this;
  }

  withSystemId(systemId: number): this {
    this.data.systemId = systemId;
    return this;
  }

  withUuid(uuid: string): this {
    this.data.uuid = uuid;
    return this;
  }

  withName(name: string): this {
    this.data.name = name;
    return this;
  }

  withoutMoons(): this {
    this.data.moons = [];
    return this;
  }

  withMoons(moons: Moon[]): this {
    moons.map((moon) => {
      moon.planetId = this.data.id as number;
    });
    this.data.moons = moons;
    return this;
  }
}
