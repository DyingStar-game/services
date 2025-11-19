import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { Vector3 } from "@lib/math/vector3";

export class Basis3D {
  constructor(
    public x: Vector3Type = { x: 1, y: 0, z: 0 },
    public y: Vector3Type = { x: 0, y: 1, z: 0 },
    public z: Vector3Type = { x: 0, y: 0, z: 1 },
  ) {}

  transform(v: Vector3Type): Vector3Type {
    return {
      x: this.x.x * v.x + this.y.x * v.y + this.z.x * v.z,
      y: this.x.y * v.x + this.y.y * v.y + this.z.y * v.z,
      z: this.x.z * v.x + this.y.z * v.y + this.z.z * v.z,
    };
  }

  rotated(axis: Vector3Type, angleRad: number): Basis3D {
    const magnitude = Vector3.magnitude(axis);
    if (magnitude === 0) {
      return this;
    }

    const axisNorm = Vector3.normalize(axis);
    const cosA = Math.cos(angleRad);
    const sinA = Math.sin(angleRad);
    const { x: ux, y: uy, z: uz } = axisNorm;
    const oneMinusCos = 1 - cosA;

    // Matrice de rotation de Rodrigues
    const m00 = cosA + ux * ux * oneMinusCos;
    const m01 = ux * uy * oneMinusCos - uz * sinA;
    const m02 = ux * uz * oneMinusCos + uy * sinA;

    const m10 = uy * ux * oneMinusCos + uz * sinA;
    const m11 = cosA + uy * uy * oneMinusCos;
    const m12 = uy * uz * oneMinusCos - ux * sinA;

    const m20 = uz * ux * oneMinusCos - uy * sinA;
    const m21 = uz * uy * oneMinusCos + ux * sinA;
    const m22 = cosA + uz * uz * oneMinusCos;

    const rotVec = (v: Vector3Type): Vector3Type => ({
      x: m00 * v.x + m01 * v.y + m02 * v.z,
      y: m10 * v.x + m11 * v.y + m12 * v.z,
      z: m20 * v.x + m21 * v.y + m22 * v.z,
    });

    return new Basis3D(rotVec(this.x), rotVec(this.y), rotVec(this.z));
  }

  inverse(): Basis3D {
    return new Basis3D(
      { x: this.x.x, y: this.y.x, z: this.z.x },
      { x: this.x.y, y: this.y.y, z: this.z.y },
      { x: this.x.z, y: this.y.z, z: this.z.z },
    );
  }

  static identity(): Basis3D {
    return new Basis3D();
  }
}
