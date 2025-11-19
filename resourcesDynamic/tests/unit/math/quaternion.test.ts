// ou fakerFR, fakerEN, selon ton setup
import { aQuaternion } from "@builder/builders";
import { Quaternion } from "@lib/math/quaternion";
import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { describe, expect, it } from "vitest";

describe("Quaternion", () => {
  it("should create identity quaternion", () => {
    const q = Quaternion.identity();
    expect(q.x).toBe(0);
    expect(q.y).toBe(0);
    expect(q.z).toBe(0);
    expect(q.w).toBe(1);
  });

  it("should normalize correctly", () => {
    const q = aQuaternion().build();
    const norm = q.normalize();
    const len = Math.sqrt(
      norm.x ** 2 + norm.y ** 2 + norm.z ** 2 + norm.w ** 2,
    );
    expect(Math.abs(len - 1)).toBeLessThan(1e-5);
  });

  it("should calculate length", () => {
    const q = aQuaternion().build();
    const expected = Math.sqrt(q.x ** 2 + q.y ** 2 + q.z ** 2 + q.w ** 2);
    expect(q.length()).toBeCloseTo(expected, 5);
  });

  it("should compute conjugate", () => {
    const q = aQuaternion().build();
    const conj = q.conjugate();
    expect(conj.x).toBe(-q.x);
    expect(conj.y).toBe(-q.y);
    expect(conj.z).toBe(-q.z);
    expect(conj.w).toBe(q.w);
  });

  it("should compute inverse of a non-zero quaternion", () => {
    const q = aQuaternion().build();
    const inv = q.inverse();
    const lenSq = q.x ** 2 + q.y ** 2 + q.z ** 2 + q.w ** 2;
    expect(inv.x).toBeCloseTo(-q.x / lenSq);
    expect(inv.y).toBeCloseTo(-q.y / lenSq);
    expect(inv.z).toBeCloseTo(-q.z / lenSq);
    expect(inv.w).toBeCloseTo(q.w / lenSq);
  });

  it("should multiply quaternions", () => {
    const q1 = aQuaternion().build();
    const q2 = aQuaternion().build();
    const res = q1.mul(q2);

    expect(res.x).toBeCloseTo(
      q1.w * q2.x + q1.x * q2.w + q1.y * q2.z - q1.z * q2.y,
    );

    expect(res.y).toBeCloseTo(
      q1.w * q2.y - q1.x * q2.z + q1.y * q2.w + q1.z * q2.x,
    );

    expect(res.z).toBeCloseTo(
      q1.w * q2.z + q1.x * q2.y - q1.y * q2.x + q1.z * q2.w,
    );

    expect(res.w).toBeCloseTo(
      q1.w * q2.w - q1.x * q2.x - q1.y * q2.y - q1.z * q2.z,
    );
  });

  it("should rotate a vector with identity quaternion", () => {
    const v: Vector3Type = { x: 1, y: 0, z: 0 };
    const q = Quaternion.identity();
    const rotated = q.rotateVector(v);
    expect(rotated).toEqual(v);
  });

  it("should return correct dot product", () => {
    const q1 = aQuaternion().build();
    const q2 = aQuaternion().build();
    expect(q1.dot(q2)).toBe(
      q1.x * q2.x + q1.y * q2.y + q1.z * q2.z + q1.w * q2.w,
    );
  });

  it("should negate quaternion", () => {
    const q = aQuaternion().build();
    const neg = q.negate();
    expect(neg.x).toBe(-q.x);
    expect(neg.y).toBe(-q.y);
    expect(neg.z).toBe(-q.z);
    expect(neg.w).toBe(-q.w);
  });

  it("should clone quaternion", () => {
    const q = aQuaternion().build();
    const clone = q.clone();
    expect(clone).not.toBe(q); // pas la même instance
    expect(clone).toEqual(q); // mais mêmes valeurs
  });
});
