import { Faker } from "@faker-js/faker";
import { Vector3Type, vector3Schema } from "@lib/math/schema/vector3.model";

import { BuilderBase } from "./BuilderBase";

export class Vector3Builder extends BuilderBase<typeof vector3Schema> {
  constructor(faker: Faker) {
    super(vector3Schema, {
      x: faker.number.int(),
      y: faker.number.int(),
      z: faker.number.int(),
    });
  }

  withX(x: number): this {
    this.data.x = x;
    return this;
  }

  withY(y: number): this {
    this.data.y = y;
    return this;
  }

  withZ(z: number): this {
    this.data.z = z;
    return this;
  }

  withCoordinate(coord: Vector3Type): this {
    this.data = coord;
    return this;
  }
}
