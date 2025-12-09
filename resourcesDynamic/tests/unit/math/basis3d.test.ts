import { aVector3 } from "@builder/builders";
import { Basis3D } from "@lib/math/basis3d";
import { describe, expect, it } from "vitest";

const vec = (x: number, y: number, z: number) => {
  return aVector3().withCoordinate({ x, y, z }).build();
};

describe("Basis3D", () => {
  it("should construct identity basis by default", () => {
    const basis = new Basis3D();
    expect(basis.x).toEqual(vec(1, 0, 0));
    expect(basis.y).toEqual(vec(0, 1, 0));
    expect(basis.z).toEqual(vec(0, 0, 1));
  });

  it("transform should apply basis change", () => {
    const basis = new Basis3D(vec(0, 1, 0), vec(1, 0, 0), vec(0, 0, 1));
    // In this basis, x and y swapped
    const v = vec(2, 3, 4);
    const transformed = basis.transform(v);
    expect(transformed).toEqual({
      x: 3, // basis.x: {0,1,0}
      y: 2, // basis.y: {1,0,0}
      z: 4, // basis.z: {0,0,1}
    });
  });

  it("identity() should return a default identity basis", () => {
    const identity = Basis3D.identity();
    expect(identity.x).toEqual(vec(1, 0, 0));
    expect(identity.y).toEqual(vec(0, 1, 0));
    expect(identity.z).toEqual(vec(0, 0, 1));
  });

  it("rotated with zero axis does nothing", () => {
    const basis = new Basis3D();
    const rotated = basis.rotated(vec(0, 0, 0), Math.PI / 4);
    expect(rotated.x).toEqual(basis.x);
    expect(rotated.y).toEqual(basis.y);
    expect(rotated.z).toEqual(basis.z);
  });

  it("rotated 90deg around z axis swaps x and y", () => {
    const basis = new Basis3D();
    const angleRad = Math.PI / 2;
    const rotated = basis.rotated(vec(0, 0, 1), angleRad);
    expect(rotated.x.x).toBeCloseTo(0, 6);
    expect(rotated.x.y).toBeCloseTo(1, 6);
    expect(rotated.x.z).toBeCloseTo(0, 6);
    expect(rotated.y.x).toBeCloseTo(-1, 6);
    expect(rotated.y.y).toBeCloseTo(0, 6);
    expect(rotated.y.z).toBeCloseTo(0, 6);
    expect(rotated.z).toEqual(vec(0, 0, 1));
  });

  it("inverse of identity is identity", () => {
    const basis = new Basis3D();
    const inv = basis.inverse();
    expect(inv.x).toEqual(basis.x);
    expect(inv.y).toEqual(basis.y);
    expect(inv.z).toEqual(basis.z);
  });

  it("inverse transpose the basis", () => {
    const basis = new Basis3D(vec(1, 2, 3), vec(4, 5, 6), vec(7, 8, 9));
    const inv = basis.inverse();
    expect(inv.x).toEqual(vec(1, 4, 7));
    expect(inv.y).toEqual(vec(2, 5, 8));
    expect(inv.z).toEqual(vec(3, 6, 9));
  });
});
