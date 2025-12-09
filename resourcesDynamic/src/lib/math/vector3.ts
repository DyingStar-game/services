import type { Vector3Type } from "@lib/math/schema/vector3.model";

export class Vector3 {
  static create(x: number = 0, y: number = 0, z: number = 0): Vector3Type {
    return { x, y, z };
  }

  static add(a: Vector3Type, b: Vector3Type): Vector3Type {
    return { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z };
  }

  static subtract(a: Vector3Type, b: Vector3Type): Vector3Type {
    return { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z };
  }

  static multiply(v: Vector3Type, scalar: number): Vector3Type {
    return { x: v.x * scalar, y: v.y * scalar, z: v.z * scalar };
  }

  static distanceTo(a: Vector3Type, b: Vector3Type): number {
    const dx = a.x - b.x;
    const dy = a.y - b.y;
    const dz = a.z - b.z;
    return Math.sqrt(dx * dx + dy * dy + dz * dz);
  }

  static magnitude(v: Vector3Type): number {
    return Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
  }

  static normalize(v: Vector3Type): Vector3Type {
    const mag = this.magnitude(v);
    if (mag === 0) return { x: 0, y: 0, z: 0 };
    return { x: v.x / mag, y: v.y / mag, z: v.z / mag };
  }

  static dot(a: Vector3Type, b: Vector3Type): number {
    return a.x * b.x + a.y * b.y + a.z * b.z;
  }

  static cross(a: Vector3Type, b: Vector3Type): Vector3Type {
    return {
      x: a.y * b.z - a.z * b.y,
      y: a.z * b.x - a.x * b.z,
      z: a.x * b.y - a.y * b.x,
    };
  }
}
