import { starSchema } from "@db/schema";
import { Faker } from "@faker-js/faker";

import { BuilderBase } from "./BuilderBase";

export class StarBuilder extends BuilderBase<typeof starSchema> {
  constructor(faker: Faker) {
    super(starSchema, {
      id: faker.number.int({ min: 1, max: 50000 }),
      uuid: faker.string.uuid(),
      systemId: faker.number.int({ min: 1, max: 50000 }),
      name: faker.word.sample(),
      internalName: faker.word.sample(),
      massKg: faker.number.float(),
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
}
