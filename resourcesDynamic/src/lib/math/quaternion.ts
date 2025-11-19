import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { Vector3 } from "@lib/math/vector3";

export class Quaternion {
  constructor(
    public x = 0,
    public y = 0,
    public z = 0,
    public w = 1,
  ) {}

  static identity() {
    return new Quaternion(0, 0, 0, 1);
  }

  static fromAxisAngle(axis: Vector3Type, angleRad: number) {
    const half = angleRad / 2;
    const s = Math.sin(half);
    const a = Vector3.normalize(axis);
    return new Quaternion(a.x * s, a.y * s, a.z * s, Math.cos(half));
  }

  mul(q: Quaternion): Quaternion {
    return new Quaternion(
      this.w * q.x + this.x * q.w + this.y * q.z - this.z * q.y,
      this.w * q.y - this.x * q.z + this.y * q.w + this.z * q.x,
      this.w * q.z + this.x * q.y - this.y * q.x + this.z * q.w,
      this.w * q.w - this.x * q.x - this.y * q.y - this.z * q.z,
    );
  }

  rotateVector(v: Vector3Type): Vector3Type {
    const qv = new Quaternion(v.x, v.y, v.z, 0);
    const qi = new Quaternion(-this.x, -this.y, -this.z, this.w);
    const r = this.mul(qv).mul(qi);
    return { x: r.x, y: r.y, z: r.z };
  }

  // Calcule la norme du quaternion
  length(): number {
    return Math.sqrt(
      this.x * this.x + this.y * this.y + this.z * this.z + this.w * this.w,
    );
  }

  // Normalise le quaternion (retourne un nouveau quaternion)
  normalize(): Quaternion {
    const len = this.length();
    if (len < 0.000001) {
      return Quaternion.identity();
    }
    const invLen = 1 / len;
    return new Quaternion(
      this.x * invLen,
      this.y * invLen,
      this.z * invLen,
      this.w * invLen,
    );
  }

  // Conjugué du quaternion (inverse pour quaternions unitaires)
  conjugate(): Quaternion {
    return new Quaternion(-this.x, -this.y, -this.z, this.w);
  }

  // Inverse du quaternion
  inverse(): Quaternion {
    const lenSq =
      this.x * this.x + this.y * this.y + this.z * this.z + this.w * this.w;
    if (lenSq < 0.000001) {
      return Quaternion.identity();
    }
    const invLenSq = 1 / lenSq;
    return new Quaternion(
      -this.x * invLenSq,
      -this.y * invLenSq,
      -this.z * invLenSq,
      this.w * invLenSq,
    );
  }

  // Produit scalaire entre quaternions
  dot(q: Quaternion): number {
    return this.x * q.x + this.y * q.y + this.z * q.z + this.w * q.w;
  }

  clone(): Quaternion {
    return new Quaternion(this.x, this.y, this.z, this.w);
  }

  negate(): Quaternion {
    return new Quaternion(-this.x, -this.y, -this.z, -this.w);
  }

  static fromBasis(
    right: Vector3Type,
    up: Vector3Type,
    forward: Vector3Type,
  ): Quaternion {
    const m00 = right.x,
      m01 = up.x,
      m02 = forward.x;
    const m10 = right.y,
      m11 = up.y,
      m12 = forward.y;
    const m20 = right.z,
      m21 = up.z,
      m22 = forward.z;

    const trace = m00 + m11 + m22;
    let x, y, z, w;

    if (trace > 0) {
      const s = 0.5 / Math.sqrt(trace + 1.0);
      w = 0.25 / s;
      x = (m21 - m12) * s;
      y = (m02 - m20) * s;
      z = (m10 - m01) * s;
    } else if (m00 > m11 && m00 > m22) {
      const s = 2.0 * Math.sqrt(1.0 + m00 - m11 - m22);
      w = (m21 - m12) / s;
      x = 0.25 * s;
      y = (m01 + m10) / s;
      z = (m02 + m20) / s;
    } else if (m11 > m22) {
      const s = 2.0 * Math.sqrt(1.0 + m11 - m00 - m22);
      w = (m02 - m20) / s;
      x = (m01 + m10) / s;
      y = 0.25 * s;
      z = (m12 + m21) / s;
    } else {
      const s = 2.0 * Math.sqrt(1.0 + m22 - m00 - m11);
      w = (m10 - m01) / s;
      x = (m02 + m20) / s;
      y = (m12 + m21) / s;
      z = 0.25 * s;
    }

    return new Quaternion(x, y, z, w).normalize();
  }
}
