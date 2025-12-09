import { aPlanet, aStar } from "@builder/builders";
import { PlanetWithMoon, Star, systemWithDetailsSchema } from "@db/schema";
import { Faker } from "@faker-js/faker";

import { BuilderBase } from "./BuilderBase";

export class SystemBuilder extends BuilderBase<typeof systemWithDetailsSchema> {
  constructor(faker: Faker) {
    const systemId = faker.number.int({ min: 1, max: 50000 });
    const star = aStar(faker).withSystemId(systemId).build();
    const countPlanet = faker.number.int({ min: 5, max: 20 });
    const planets = Array.from({ length: countPlanet }, () =>
      aPlanet(faker).withSystemId(systemId).build(),
    );
    super(systemWithDetailsSchema, {
      id: systemId,
      uuid: faker.string.uuid(),
      name: faker.word.sample(),
      internalName: faker.word.sample(),
      planets,
      stars: [star],
    });
  }

  withId(id: number): this {
    this.data.id = id;
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

  withPlanets(planets: PlanetWithMoon[]): this {
    planets.map((planet) => {
      planet.systemId = this.data.id as number;
    });
    this.data.planets = planets;
    return this;
  }

  withStar(star: Star): this {
    star.systemId = this.data.id as number;
    this.data.stars = [star];
    return this;
  }
}
