import { Quaternion } from "@lib/math/quaternion";
import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { Vector3 } from "@lib/math/vector3";

export type RotationObjectType = {
  tidalLocked: boolean;
  rotationPeriodH: number;
  tiltRad: number;
  spinLongitudeRad: number;
};

export class RotationQuaternion {
  private static readonly TAU = 2 * Math.PI;

  private tiltRad: number;
  private spinLongRad: number;
  private rotationRateRadS: number;

  constructor(private elements: RotationObjectType) {
    const { tiltRad, spinLongitudeRad, rotationPeriodH } = elements;
    this.tiltRad = tiltRad;
    this.spinLongRad = spinLongitudeRad;
    this.rotationRateRadS = RotationQuaternion.TAU / (rotationPeriodH * 3600);
  }

  // Crée un quaternion qui aligne l'axe X local avec "right" et Y avec "up"
  lookTarget(right: Vector3Type, up: Vector3Type): Quaternion {
    right = Vector3.normalize(right);
    up = Vector3.normalize(up);

    // Base orthonormée : X = right, Y = up, Z = forward
    const forward = Vector3.normalize(Vector3.cross(right, up));
    const correctedUp = Vector3.normalize(Vector3.cross(forward, right));

    return Quaternion.fromBasis(right, correctedUp, forward);
  }

  getRotation(
    currentTime: number,
    orbitPos?: Vector3Type,
    orbitPrevPos?: Vector3Type,
  ): Quaternion {
    const { tidalLocked } = this.elements;

    if (tidalLocked && orbitPos) {
      // Direction vers le parent (ce sera l'axe X local)
      const towardParent = Vector3.normalize({
        x: -orbitPos.x,
        y: -orbitPos.y,
        z: -orbitPos.z,
      });

      const globalUp = Vector3.cross(
        orbitPos,
        orbitPrevPos ?? { x: 0, y: 0, z: 0 },
      );

      let targetRotation = this.lookTarget(towardParent, globalUp);

      targetRotation = targetRotation.mul(
        Quaternion.fromAxisAngle({ x: 0, y: 0, z: 1 }, this.tiltRad),
      );
      targetRotation = targetRotation.mul(
        Quaternion.fromAxisAngle({ x: 0, y: 1, z: 0 }, this.spinLongRad),
      );

      return targetRotation;
    }

    // ---- CASE 2 : FREE ROTATION ----
    const spinAngle = this.rotationRateRadS * currentTime;

    let q = Quaternion.identity();
    q = q.mul(Quaternion.fromAxisAngle({ x: 0, y: 0, z: 1 }, this.tiltRad));
    q = q.mul(Quaternion.fromAxisAngle({ x: 0, y: 1, z: 0 }, this.spinLongRad));
    q = q.mul(Quaternion.fromAxisAngle({ x: 0, y: 1, z: 0 }, spinAngle));

    return q;
  }
}
