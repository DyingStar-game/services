import { keplerOrbitLogger } from "@lib/logger";
import { Basis3D } from "@lib/math/basis3d";
import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { Vector3 } from "@lib/math/vector3";

export type PositionObjectType = {
  primaryMassKg: number;
  objectMassKg: number;
  periapsisAU: number;
  apoapsisAU: number;
  inclinationRad: number;
  longitudeOfAscendingNodeRad: number;
  argumentOfPeriapsisRad: number;
  meanAnomalyRad: number;
};

export class KeplerOrbit {
  private static readonly AU_M = 1.495978707e11;
  private static readonly G = 6.6743e-11;
  private static readonly TAU = 2.0 * Math.PI;
  private static readonly SOLAR_MASS_KG = 1.98892e30;

  private basis: Basis3D;
  private orbitCenter: Vector3Type;
  private semiMajorAxisM: number;
  private eccentricity: number;
  private meanMotion: number;
  private meanAnomaly: number;

  constructor(
    private elements: PositionObjectType,
    private referenceTimeS: number = 0,
  ) {
    this.orbitCenter = { x: 0, y: 0, z: 0 };

    if (elements.periapsisAU > 0 || elements.apoapsisAU > 0) {
      let rpAU =
        elements.periapsisAU > 0 ? elements.periapsisAU : elements.apoapsisAU;
      let raAU =
        elements.apoapsisAU > 0 ? elements.apoapsisAU : elements.periapsisAU;

      if (raAU < rpAU) {
        [rpAU, raAU] = [raAU, rpAU];
      }

      this.semiMajorAxisM = 0.5 * (rpAU + raAU) * KeplerOrbit.AU_M;
      this.eccentricity = Math.max(
        0,
        (raAU - rpAU) / Math.max(raAU + rpAU, 1e-12),
      );

      keplerOrbitLogger.debug(
        {
          "Semi-major axis": `${(this.semiMajorAxisM / 1e9).toFixed(2)} million km (${(this.semiMajorAxisM / KeplerOrbit.AU_M).toFixed(4)} AU)`,
          Eccentricity: this.eccentricity.toFixed(4),
          Periapsis: `${rpAU.toFixed(4)} AU`,
          Apoapsis: `${raAU.toFixed(4)} AU`,
        },
        "📐 Orbit parameters",
      );
    } else {
      throw new Error("periapsisAU and apoapsisAU must be provided");
    }

    if (elements.primaryMassKg === 0) {
      keplerOrbitLogger.warn(
        { defaultValue: KeplerOrbit.SOLAR_MASS_KG, orbitalObject: elements },
        "⚠️ Star mass missing. Use default value",
      );
    }

    const starMass = elements.primaryMassKg || KeplerOrbit.SOLAR_MASS_KG;
    const objectMass = elements.objectMassKg;

    // Create mean motion
    const mu = KeplerOrbit.G * (starMass + objectMass);
    this.meanMotion = Math.sqrt(mu / Math.pow(this.semiMajorAxisM, 3));

    // Create the orbital rotation base
    this.basis = this.createOrbitBasis(
      elements.argumentOfPeriapsisRad,
      elements.inclinationRad,
      elements.longitudeOfAscendingNodeRad,
    );

    this.meanAnomaly = this.normalizeAngle(
      elements.meanAnomalyRad + this.meanMotion * referenceTimeS,
    );

    // Check the calculated initial position
    const initialPos = this.getPosition();
    const initialDistance = Vector3.magnitude(initialPos);
    keplerOrbitLogger.debug(
      {
        elements,
        "🎯 Mean anomaly at epoch (T=0)": `${((elements.meanAnomalyRad * 180) / Math.PI).toFixed(2)}°`,
        "🕐 Reference time": `${referenceTimeS.toFixed(2)}s`,
        "📍 Mean anomaly": `${((this.meanAnomaly * 180) / Math.PI).toFixed(2)}°`,
        distance: `${(initialDistance / 1e9).toFixed(2)} million km`,
        coordinates: `(${(initialPos.x / 1e9).toFixed(2)}, ${(initialPos.y / 1e9).toFixed(2)}, ${(initialPos.z / 1e9).toFixed(2)}) million km`,
      },
      "✅ Initial position calculated",
    );

    // Check if object is on orbit
    const minR = this.semiMajorAxisM * (1 - this.eccentricity);
    const maxR = this.semiMajorAxisM * (1 + this.eccentricity);
    if (initialDistance < minR * 0.99 || initialDistance > maxR * 1.01) {
      keplerOrbitLogger.warn(
        {
          "Expected range": `[${(minR / 1e9).toFixed(2)}, ${(maxR / 1e9).toFixed(2)}] million km`,
          Actual: `${(initialDistance / 1e9).toFixed(2)} million km`,
        },
        "⚠️  Initial position may be off orbit!",
      );
    }
  }

  getOrbitalPeriod(): number {
    if (this.meanMotion === 0) return 0;
    return (2 * Math.PI) / this.meanMotion;
  }

  private createOrbitBasis(
    argPeriRad: number,
    incRad: number,
    nodeRad: number,
  ): Basis3D {
    let b = Basis3D.identity();
    // Rotation sequence: Rz(Ω) * Rx(i) * Rz(ω)
    b = b.rotated({ x: 0, y: 0, z: 1 }, nodeRad);
    b = b.rotated({ x: 1, y: 0, z: 0 }, incRad);
    b = b.rotated({ x: 0, y: 0, z: 1 }, argPeriRad);
    return b;
  }

  private normalizeAngle(angle: number): number {
    let x = angle % KeplerOrbit.TAU;
    if (x < 0) x += KeplerOrbit.TAU;
    return x;
  }

  private solveKeplerEquation(M: number, e: number): number {
    const Mm = this.normalizeAngle(M);
    let E = e > 0.8 ? Math.PI : Mm;

    for (let i = 0; i < 16; i++) {
      const f = E - e * Math.sin(E) - Mm;
      const fp = 1.0 - e * Math.cos(E);
      const step = -f / Math.max(fp, 1e-12);
      E += step;
      if (Math.abs(step) < 1e-12) break;
    }

    return this.normalizeAngle(E);
  }

  private keplerPositionPlane(a: number, e: number, M: number): Vector3Type {
    const E = this.solveKeplerEquation(M, e);
    const xp = a * (Math.cos(E) - e);
    const zp = a * Math.sqrt(Math.max(1.0 - e * e, 0)) * Math.sin(E);
    return { x: xp, y: 0, z: zp };
  }

  private calculatePosition(M: number): Vector3Type {
    const posPlane = this.keplerPositionPlane(
      this.semiMajorAxisM,
      this.eccentricity,
      M,
    );
    return Vector3.add(this.orbitCenter, this.basis.transform(posPlane));
  }

  advance(dt: number): Vector3Type {
    this.meanAnomaly = this.normalizeAngle(
      this.meanAnomaly + this.meanMotion * dt,
    );
    return this.calculatePosition(this.meanAnomaly);
  }

  getPosition(): Vector3Type {
    return this.calculatePosition(this.meanAnomaly);
  }

  getCurrentMeanAnomaly(): number {
    return this.meanAnomaly;
  }
}
