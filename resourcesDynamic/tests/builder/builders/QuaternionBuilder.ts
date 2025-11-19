import { Faker } from "@faker-js/faker";
import { Quaternion } from "@lib/math/quaternion";
import { QuaternionType } from "@lib/math/schema/quaternion.model";

export class QuaternionBuilder {
  protected data: Partial<Quaternion>;
  constructor(faker: Faker) {
    this.data = {
      x: faker.number.int(),
      y: faker.number.int(),
      z: faker.number.int(),
      w: faker.number.int(),
    };
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

  withW(w: number): this {
    this.data.w = w;
    return this;
  }

  withCoordinate(coord: QuaternionType): this {
    this.data = coord;
    return this;
  }

  build(): Quaternion {
    return new Quaternion(this.data.x, this.data.y, this.data.z, this.data.w);
  }
}
