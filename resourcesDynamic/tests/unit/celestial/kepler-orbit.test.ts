import { KeplerOrbit, type PositionObjectType } from "@lib/celestial/orbit/kepler-orbit";
import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { describe, expect, it } from "vitest";

// The conventions every orbit (planet, moon, station) follows, shared with the game's kepler_orbit.gd:
// reference plane XZ, north +Y, orbits counter-clockwise seen from +Y (the sense a body spins in),
// Ry(node)·Rx(inc)·Ry(argPeri). The closed-form case below is the SAME, number for number, as the
// game's test_kepler_orbit.gd: a divergence between the two solvers would raise no error anywhere, it
// would put a planet somewhere else for the server than for the clients.

const AU_M = 1.495978707e11;
const SUN_KG = 1.98892e30;
const SANDBOX_STAR_KG = 1.5088e30;
const EARTH_KG = 5.972e24;
const SECONDS_PER_DAY = 86400;
const deg = (d: number) => (d * Math.PI) / 180;

// Rotations written out by hand, so the expected values owe nothing to Basis3D.
const ry = (v: Vector3Type, a: number): Vector3Type => ({
  x: v.x * Math.cos(a) + v.z * Math.sin(a),
  y: v.y,
  z: -v.x * Math.sin(a) + v.z * Math.cos(a),
});
const rx = (v: Vector3Type, a: number): Vector3Type => ({
  x: v.x,
  y: v.y * Math.cos(a) - v.z * Math.sin(a),
  z: v.y * Math.sin(a) + v.z * Math.cos(a),
});
const distance = (a: Vector3Type, b: Vector3Type) =>
  Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
const cross = (a: Vector3Type, b: Vector3Type): Vector3Type => ({
  x: a.y * b.z - a.z * b.y,
  y: a.z * b.x - a.x * b.z,
  z: a.x * b.y - a.y * b.x,
});
const angleDeg = (a: Vector3Type, b: Vector3Type) => {
  const cosine =
    (a.x * b.x + a.y * b.y + a.z * b.z) /
    (Math.hypot(a.x, a.y, a.z) * Math.hypot(b.x, b.y, b.z));
  return (Math.acos(Math.max(-1, Math.min(1, cosine))) * 180) / Math.PI;
};

// Where an inclined circular orbit must be: Ry(Ω)·Rx(i)·Ry(ω)·(a·cos M, 0, −a·sin M).
const closedForm = (aM: number, inc: number, node: number, argPeri: number, meanAnomaly: number) =>
  ry(rx(ry({ x: aM * Math.cos(meanAnomaly), y: 0, z: -aM * Math.sin(meanAnomaly) }, argPeri), inc), node);

const elementsOf = (
  periapsisAU: number,
  apoapsisAU: number,
  incDeg: number,
  nodeDeg: number,
  argPeriDeg: number,
  meanAnomalyDeg: number,
  primaryMassKg: number,
  objectMassKg = 0,
): PositionObjectType => ({
  primaryMassKg,
  objectMassKg,
  periapsisAU,
  apoapsisAU,
  inclinationRad: deg(incDeg),
  longitudeOfAscendingNodeRad: deg(nodeDeg),
  argumentOfPeriapsisRad: deg(argPeriDeg),
  meanAnomalyRad: deg(meanAnomalyDeg),
});

// Position at absolute time t: a fresh solver anchored at t, the way the service answers any instant.
const positionAt = (elements: PositionObjectType, t: number) => new KeplerOrbit(elements, t).getPosition();
const periodOf = (elements: PositionObjectType) => new KeplerOrbit(elements).getOrbitalPeriod();
const UP: Vector3Type = { x: 0, y: 1, z: 0 };

describe("KeplerOrbit conventions (twin of the game's test_kepler_orbit.gd)", () => {
  it("an inclined circular orbit matches the closed form shared with the game", () => {
    // 1 AU circular, inclined 30°, node at 40°, periapsis at 50°: the order of the three rotations
    // matters only because the inclination is not zero, which is the point.
    const elements = elementsOf(1, 1, 30, 40, 50, 0, SUN_KG);
    const period = periodOf(elements);
    const n = (2 * Math.PI) / period;
    for (const fraction of [0, 0.25, 1 / 3, 0.7]) {
      const t = fraction * period;
      const expected = closedForm(AU_M, deg(30), deg(40), deg(50), n * t);
      expect(distance(positionAt(elements, t), expected)).toBeLessThan(1);
    }
  });

  it("a flat orbit stays in the XZ plane whatever its node and periapsis", () => {
    const elements = elementsOf(0.5335, 0.5665, 0, 110, 45, 20, SANDBOX_STAR_KG);
    const period = periodOf(elements);
    for (let i = 0; i < 8; i++) {
      expect(Math.abs(positionAt(elements, (period * i) / 8).y)).toBeLessThan(1);
    }
  });

  it("runs counter-clockwise seen from north (+Y), the sense a body spins in", () => {
    const elements = elementsOf(0.5335, 0.5665, 1.5, 110, 45, 0, SANDBOX_STAR_KG);
    const p0 = positionAt(elements, 0);
    const p1 = positionAt(elements, 600);
    const momentum = cross(p0, { x: p1.x - p0.x, y: p1.y - p0.y, z: p1.z - p0.z });
    expect(momentum.y).toBeGreaterThan(0);
  });

  it("measures the inclination from north", () => {
    const elements = elementsOf(1, 1, 30, 40, 0, 0, SUN_KG);
    const quarter = periodOf(elements) / 4;
    const normal = cross(positionAt(elements, 0), positionAt(elements, quarter));
    expect(angleDeg(normal, UP)).toBeCloseTo(30, 2);
  });

  it("rises through the plane at the ascending node, at the node's longitude about +Y", () => {
    const elements = elementsOf(1, 1, 30, 40, 0, 0, SUN_KG);
    const expected = ry({ x: AU_M, y: 0, z: 0 }, deg(40));
    expect(distance(positionAt(elements, 0), expected)).toBeLessThan(1);
    expect(positionAt(elements, 600).y).toBeGreaterThan(0);
  });

  it("keeps SandBox's period", () => {
    const elements = elementsOf(0.5335, 0.5665, 1.5, 110, 45, 20, SANDBOX_STAR_KG, 0.788 * EARTH_KG);
    expect(periodOf(elements) / SECONDS_PER_DAY).toBeCloseTo(171.05, 0);
  });

  it("leans a planet's spin axis on its orbit by its tilt, not by its node and periapsis", () => {
    // SandBox: tilt 15° in the data must be ~15° between its spin axis and its orbit's normal — not
    // the 40° the old Z-turned basis produced (and 110° to 169°, retrograde, for six other planets).
    const elements = elementsOf(0.5335, 0.5665, 1.5, 110, 45, 20, SANDBOX_STAR_KG);
    const quarter = periodOf(elements) / 4;
    const normal = cross(positionAt(elements, 0), positionAt(elements, quarter));
    const tilt = deg(15);
    const spinAxis = { x: -Math.sin(tilt), y: Math.cos(tilt), z: 0 }; // Rz(tilt) · +Y
    expect(Math.abs(angleDeg(normal, spinAxis) - 15)).toBeLessThan(1.5);
  });
});
