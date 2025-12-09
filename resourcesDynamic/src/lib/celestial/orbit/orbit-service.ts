import type {
  CacheCalculationParamsType,
  CacheStrategyType,
  PrefetchConfigType,
  TransformType,
} from "@lib/cache/cache-transform";
import { CacheTransform } from "@lib/cache/cache-transform";
import type { RotationObjectType } from "@lib/celestial/rotation-quaternion";
import { RotationQuaternion } from "@lib/celestial/rotation-quaternion";
import { keplerOrbitServiceLogger, logPerformance } from "@lib/logger";
import type { Vector3Type } from "@lib/math/schema/vector3.model";

import { KeplerOrbit, type PositionObjectType } from "./kepler-orbit";

export type OrbitalObjectType = PositionObjectType & RotationObjectType;

export type OrbitCalculationParamsType = CacheCalculationParamsType & {
  orbitalObject: OrbitalObjectType;
};

export class OrbitService {
  private cacheTransform: CacheTransform;

  constructor(
    cacheStrategy?: Partial<CacheStrategyType>,
    prefetchConfig?: Partial<PrefetchConfigType>,
  ) {
    this.cacheTransform = new CacheTransform(cacheStrategy, prefetchConfig);
  }

  private calculateInternal = (
    params: OrbitCalculationParamsType,
  ): TransformType[] => {
    const startTime = performance.now();

    const timeStep = 1 / params.frequency;
    const orbitalObject = params.orbitalObject;

    if (!orbitalObject) {
      return [];
    }

    keplerOrbitServiceLogger.debug(
      {
        objectId: params.objectId,
        startTime: params.startTime,
      },
      `🕐 Calculating orbit starting at T=${params.startTime}s`,
    );

    const orbit = new KeplerOrbit(orbitalObject, params.startTime);
    const orbitRotation = new RotationQuaternion(orbitalObject);
    const steps = Math.max(1, Math.ceil(params.duration / timeStep));
    const transforms: TransformType[] = new Array(steps) as TransformType[];

    let currentTime = params.startTime;
    let posPrev: Vector3Type | undefined;

    for (let i = 0; i < steps; i++) {
      const dt = i === 0 ? 0 : timeStep;

      const position = orbit.advance(dt);
      const rotation = orbitRotation.getRotation(
        currentTime,
        position,
        posPrev,
      );

      transforms[i] = {
        timeS: Math.round(currentTime * 1000) / 1000,
        position,
        rotation,
      };

      currentTime += timeStep;
      posPrev = position;
    }

    const duration = performance.now() - startTime;

    logPerformance(
      keplerOrbitServiceLogger,
      `Calculated ${steps} transforms (${((steps / duration) * 1000).toFixed(0)} transforms/sec)`,
      duration,
    );

    return transforms;
  };

  getCacheTransform(): CacheTransform {
    return this.cacheTransform;
  }

  /**
   * Get orbital transform
   */
  getTransforms(params: OrbitCalculationParamsType): TransformType[] {
    // keplerOrbitServiceLogger.debug(
    //   { cacheStats: this.cachePosition.getCacheStats() },
    //   "Stats for cache",
    // );
    return this.cacheTransform.getTransforms(params, this.calculateInternal);
  }
}

export const keplerOrbitService = new OrbitService();
