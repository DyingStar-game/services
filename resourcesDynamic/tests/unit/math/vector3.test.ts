import { aVector3 } from "@builder/builders";
import { Vector3 } from "@lib/math/vector3";
import { describe, expect, it } from "vitest";

const vec = (x: number, y: number, z: number) => {
  return aVector3().withCoordinate({ x, y, z }).build();
};

describe("Vector3", () => {
  it("create should return correct vector", () => {
    expect(Vector3.create()).toEqual(vec(0, 0, 0));
    expect(Vector3.create(1, -2, 3)).toEqual(vec(1, -2, 3));
  });

  it("add should return componentwise sum", () => {
    expect(Vector3.add(vec(1, 2, 3), vec(4, 5, 6))).toEqual(vec(5, 7, 9));
  });

  it("subtract should return componentwise difference", () => {
    expect(Vector3.subtract(vec(7, 8, 9), vec(1, 2, 3))).toEqual(vec(6, 6, 6));
  });

  it("multiply should multiply all components by scalar", () => {
    expect(Vector3.multiply(vec(2, 4, 6), 0.5)).toEqual(vec(1, 2, 3));
    expect(Vector3.multiply(vec(2, 4, 6), 0)).toEqual(vec(0, 0, 0));
  });

  it("distanceTo should compute Euclidean distance", () => {
    expect(Vector3.distanceTo(vec(0, 0, 0), vec(3, 4, 0))).toBe(5);
    expect(Vector3.distanceTo(vec(1, 2, 3), vec(1, 2, 3))).toBe(0);
  });

  it("magnitude should compute vector length", () => {
    expect(Vector3.magnitude(vec(3, 4, 0))).toBe(5);
    expect(Vector3.magnitude(vec(0, 0, 0))).toBe(0);
  });

  it("normalize should return unit vector", () => {
    expect(Vector3.normalize(vec(0, 0, 0))).toEqual(vec(0, 0, 0));
    const norm = Vector3.normalize(vec(3, 0, 4));
    expect(norm.x).toBeCloseTo(0.6, 6);
    expect(norm.y).toBeCloseTo(0, 6);
    expect(norm.z).toBeCloseTo(0.8, 6);
  });

  it("dot should return dot product", () => {
    expect(Vector3.dot(vec(1, 2, 3), vec(4, -5, 6))).toBe(12); // 1*4 + 2*-5 + 3*6
    expect(Vector3.dot(vec(0, 0, 0), vec(1, 2, 3))).toBe(0);
  });

  it("cross should return correct cross product", () => {
    expect(Vector3.cross(vec(1, 0, 0), vec(0, 1, 0))).toEqual(vec(0, 0, 1));
    expect(Vector3.cross(vec(0, 1, 0), vec(0, 0, 1))).toEqual(vec(1, 0, 0));
    expect(Vector3.cross(vec(1, 2, 3), vec(4, 5, 6))).toEqual(vec(-3, 6, -3));
  });
});
