// import type { Planet, Star } from "@db/schema";
// import type { OrbitCalculationParams } from "@lib/kepler-orbit/kepler-orbit-service";
// import { keplerOrbitService } from "@lib/kepler-orbit/kepler-orbit-service";
// import { OrbitDataHelper } from "@lib/kepler-orbit/orbit-data-helper";
// import type { Vector3Type } from "@lib/math/schema/vector3.model";
import { decode, encode } from "@msgpack/msgpack";
import type { RequestInitWsType } from "@websocket/schema/Request/init.ws.model";
import type { RequestTransformWsType } from "@websocket/schema/Request/transform.ws.model";
// import { NextTicksMessageType } from "@websocket/schema/Response/nextTick.model";
import type { ResponseWsType } from "@websocket/schema/Response/response.ws.model";
// import type { Vector3Type } from "@websocket/schema/vector3.model";
import WebSocket from "ws";

// const planetData: Planet = {
//   id: 3,
//   uuid: "5514fdf5-a411-42ea-aee5-0c2d6343accc",
//   systemId: 1,
//   name: "Tarsis_1",
//   internalName: "Tarsis_1",
//   massKg: 0.318718034317024,
//   periapsisAu: 0.0518740964529183,
//   apoapsisAu: 0.0561969378239948,
//   incDeg: 0.43879826599353,
//   nodeDeg: 3.73398360904092,
//   argPeriDeg: 89.2849996583068,
//   meanAnomalyDeg: 81.9,
//   radiusKm: 4678.72368420455,
//   radiusGravityInfluenceKm: 35315.60801443042,
// };

// // Données du système stellaire (à récupérer depuis DB)
// const starData: Star = {
//   id: 1,
//   uuid: "5514fdf5-a411-42ea-aee5-0c2d6343a000",
//   name: "plop",
//   internalName: "plop",
//   systemId: 1,
//   massKg: 0.758581416228569, // Multiplicateur (75.8% de la masse solaire)
// };

// // Conversion
// const orbitalElements = OrbitDataHelper.planetDBToOrbitalElements(
//   planetData,
//   starData,
// );

// console.log("🌟 Orbital Elements:");
// console.log(`  Star mass: ${orbitalElements.starMassKg.toExponential(2)} kg`);
// console.log(`  Planet mass: ${orbitalElements.planetMassKg} kg`);
// console.log(
//   `  Semi-major axis: ${((((orbitalElements.periapsisAU + orbitalElements.apoapsisAU) / 2) * 1.496e11) / 1e9).toFixed(2)} million km`,
// );
// console.log(`  Mean anomaly: ${orbitalElements.meanAnomalyDeg}°`);

// console.log("=== Exemple 1: Prefetch automatique ===\n");

// const params1: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: 0,
//   durationS: 60, // Demande 60 secondes
//   timestepS: 0.01666667, // 60 Hz
//   orbitalElements,
// };

// // Premier appel: calculera 600s (10x) automatiquement
// console.log("📞 Première demande: 60 secondes");
// const samples1 = keplerOrbitService.getPositions(params1);
// console.log(`✅ Reçu: ${samples1.length} échantillons\n`);

// // console.log(samples1);

// console.log("\n📊 Position Analysis:");
// samples1.slice(0, 10).forEach((sample, i) => {
//   const dist = Math.sqrt(
//     sample.position.x ** 2 + sample.position.y ** 2 + sample.position.z ** 2,
//   );
//   console.log(
//     `  T=${sample.timeS.toFixed(3)}s: ${(dist / 1e9).toFixed(6)} million km`,
//   );
// });

// const params2: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: 60,
//   durationS: 60, // Demande 60 secondes
//   timestepS: 0.01666667, // 60 Hz
//   orbitalElements,
// };
// const samples2 = keplerOrbitService.getPositions(params2);

// console.log("\n📊 Position Analysis:");
// samples2.slice(0, 10).forEach((sample, i) => {
//   const dist = Math.sqrt(
//     sample.position.x ** 2 + sample.position.y ** 2 + sample.position.z ** 2,
//   );
//   console.log(
//     `  T=${sample.timeS.toFixed(3)}s: ${(dist / 1e9).toFixed(6)} million km`,
//   );
// });

// const params3: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: 580,
//   durationS: 60, // Demande 60 secondes
//   timestepS: 0.01666667, // 60 Hz
//   orbitalElements,
// };
// const samples3 = keplerOrbitService.getPositions(params3);

// console.log("\n📊 Position Analysis:");
// samples3.slice(0, 10).forEach((sample, i) => {
//   const dist = Math.sqrt(
//     sample.position.x ** 2 + sample.position.y ** 2 + sample.position.z ** 2,
//   );
//   console.log(
//     `  T=${sample.timeS.toFixed(3)}s: ${(dist / 1e9).toFixed(6)} million km`,
//   );
// });

// // Vérifier les limites de l'orbite
// const info = OrbitDataHelper.getOrbitalInfo(orbitalElements);
// const minDistance = info.semiMajorAxisKm * (1 - info.eccentricity) * 1000; // en mètres
// const maxDistance = info.semiMajorAxisKm * (1 + info.eccentricity) * 1000; // en mètres

// console.log("\n🎯 Orbital Boundaries:");
// console.log(
//   `  Min distance (periapsis): ${(minDistance / 1e9).toFixed(4)} million km`,
// );
// console.log(
//   `  Max distance (apoapsis):  ${(maxDistance / 1e9).toFixed(4)} million km`,
// );

// // Vérifier que toutes les positions sont dans ces limites
// const allValid = samples1.every((sample) => {
//   const dist = Math.sqrt(
//     sample.position.x ** 2 + sample.position.y ** 2 + sample.position.z ** 2,
//   );
//   return dist >= minDistance * 0.99 && dist <= maxDistance * 1.01;
// });

// console.log(
//   `\n✅ All positions within orbital bounds: ${allValid ? "YES" : "NO"}`,
// );

// // Analyser les éléments orbitaux
// // const info = OrbitDataHelper.getOrbitalInfo(orbitalElements);

// console.log("📊 Orbital Analysis:");
// console.log(`  Semi-major axis: ${info.semiMajorAxisAU.toFixed(4)} AU`);
// console.log(`  Semi-major axis: ${info.semiMajorAxisKm.toFixed(0)} km`);
// console.log(`  Eccentricity: ${info.eccentricity.toFixed(4)}`);
// console.log(`  Orbital period: ${info.periodDays.toFixed(2)} days`);
// console.log(`  Orbital period: ${info.periodYears.toFixed(4)} years`);

// // Validation
// const validation = OrbitDataHelper.validateOrbitalElements(orbitalElements);
// console.log("\n✅ Validation:", validation.valid ? "OK" : "ERRORS");
// if (validation.warnings.length > 0) {
//   console.log("⚠️  Warnings:");
//   validation.warnings.forEach((w) => console.log(`  - ${w}`));
// }

// // Position initiale attendue
// const expectedDistance = info.semiMajorAxisKm * 1000; // en mètres
// console.log(
//   `\n📍 Expected initial distance: ${(expectedDistance / 1e9).toFixed(2)} million km`,
// );

// // Position Python
// const pythonPos = {
//   x: -1.490099765857017e11,
//   y: 1.7556926374876687e8,
//   z: 1.3248357307084667e10,
// };
// const pythonDistance = Math.sqrt(
//   pythonPos.x ** 2 + pythonPos.y ** 2 + pythonPos.z ** 2,
// );
// console.log(
//   `📍 Python position distance: ${(pythonDistance / 1e9).toFixed(2)} million km`,
// );

// console.log(
//   `\n❌ INCONSISTENCY: ${((pythonDistance - expectedDistance) / 1e9).toFixed(2)} million km difference!`,
// );

// ===== TEST 1: Position à T=0 =====
// console.log(`\n${"=".repeat(60)}`);
// console.log("TEST 1: startTimeS = 0");
// console.log("=".repeat(60));

// const params4: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: 0, // ⬅️ Départ à T=0
//   durationS: 0.1,
//   timestepS: 0.01666667,
//   orbitalElements,
// };

// const samples4 = keplerOrbitService.getPositions(params4);
// console.log(`\n📍 Position at T=0.000s:`);
// const pos1 = samples4[0].position;
// const dist1 = Math.sqrt(pos1.x ** 2 + pos1.y ** 2 + pos1.z ** 2);
// console.log(`  Distance: ${(dist1 / 1e9).toFixed(6)} million km`);
// console.log(
//   `  Coords: (${(pos1.x / 1e9).toFixed(4)}, ${(pos1.y / 1e9).toFixed(4)}, ${(pos1.z / 1e9).toFixed(4)})`,
// );

// // ===== TEST 2: Position à T=172800 (2 jours) =====
// console.log(`\n${"=".repeat(60)}`);
// console.log("TEST 2: startTimeS = 172800 (2 days)");
// console.log("=".repeat(60));

// const params2: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: 172800, // ⬅️ Départ après 2 jours
//   durationS: 0.1,
//   timestepS: 0.01666667,
//   orbitalElements,
// };

// const samples2 = keplerOrbitService.getPositions(params2);
// console.log(`\n📍 Position at T=172800.000s:`);
// const pos2 = samples2[0].position;
// const dist2 = Math.sqrt(pos2.x ** 2 + pos2.y ** 2 + pos2.z ** 2);
// console.log(`  Distance: ${(dist2 / 1e9).toFixed(6)} million km`);
// console.log(
//   `  Coords: (${(pos2.x / 1e9).toFixed(4)}, ${(pos2.y / 1e9).toFixed(4)}, ${(pos2.z / 1e9).toFixed(4)})`,
// );

// // ===== TEST 3: Position à T = période orbitale / 2 =====
// const halfPeriod = info.periodSeconds / 2;
// console.log(`\n${"=".repeat(60)}`);
// console.log(`TEST 3: startTimeS = ${halfPeriod.toFixed(0)} (half period)`);
// console.log("=".repeat(60));

// const params3: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: halfPeriod, // ⬅️ Mi-orbite
//   durationS: 0.1,
//   timestepS: 0.01666667,
//   orbitalElements,
// };

// const samples3 = keplerOrbitService.getPositions(params3);
// console.log(`\n📍 Position at T=${halfPeriod.toFixed(0)}s (opposite side):`);
// const pos3 = samples3[0].position;
// const dist3 = Math.sqrt(pos3.x ** 2 + pos3.y ** 2 + pos3.z ** 2);
// console.log(`  Distance: ${(dist3 / 1e9).toFixed(6)} million km`);
// console.log(
//   `  Coords: (${(pos3.x / 1e9).toFixed(4)}, ${(pos3.y / 1e9).toFixed(4)}, ${(pos3.z / 1e9).toFixed(4)})`,
// );

// // ===== COMPARAISON =====
// console.log(`\n${"=".repeat(60)}`);
// console.log("📊 COMPARISON");
// console.log("=".repeat(60));

// OrbitDataHelper.comparePositions(pos1, pos2, "T=0", "T=2 days");
// OrbitDataHelper.comparePositions(pos1, pos3, "T=0", "T=half period");

// // Vérifier que les positions sont différentes
// const diff12 = Math.sqrt(
//   (pos1.x - pos2.x) ** 2 + (pos1.y - pos2.y) ** 2 + (pos1.z - pos2.z) ** 2,
// );

// console.log(
//   `\n✅ Positions are different: ${diff12 > 1e6 ? "YES (> 1000 km)" : "NO (BUG!)"}`,
// );
// console.log(`   Difference: ${(diff12 / 1e6).toFixed(2)} thousand km`);

// console.log(`\n${"=".repeat(60)}`);
// console.log("🎉 FINAL VALIDATION");
// console.log("=".repeat(60));

// const info = OrbitDataHelper.getOrbitalInfo(orbitalElements);

// // Test sur une période complète
// const fullPeriod = info.periodSeconds;
// console.log(
//   `\n⏱️  Full orbital period: ${fullPeriod.toFixed(0)}s (${info.periodDays.toFixed(2)} days)`,
// );

// const testTimes = [
//   0, // Start
//   fullPeriod * 0.25, // Quarter
//   fullPeriod * 0.5, // Half
//   fullPeriod * 0.75, // Three quarters
//   fullPeriod, // Full period (should return to start!)
// ];

// const positions: { time: number; pos: Vector3Type; dist: number }[] = [];

// testTimes.forEach((timeS) => {
//   const params: OrbitCalculationParams = {
//     objectId: planetData.uuid as string,
//     objectType: "planet",
//     startTimeS: timeS,
//     durationS: 60,
//     timestepS: 0.01666667,
//     orbitalObject: orbitalElements,
//   };

//   const samples = keplerOrbitService.getPositions(params);
//   const pos = samples[0].position;
//   const dist = Math.sqrt(pos.x ** 2 + pos.y ** 2 + pos.z ** 2);

//   positions.push({ time: timeS, pos, dist });

//   console.log(
//     `\nT=${(timeS / 86400).toFixed(2)} days (${((timeS / fullPeriod) * 360).toFixed(0)}°):`,
//   );
//   console.log(`  Distance: ${(dist / 1e9).toFixed(4)} million km`);
//   console.log(
//     `  Position: (${(pos.x / 1e9).toFixed(2)}, ${(pos.y / 1e9).toFixed(2)}, ${(pos.z / 1e9).toFixed(2)})`,
//   );
// });

// // Vérifier que position(T=0) ≈ position(T=full_period)
// const pos0 = positions[0].pos;
// const posFull = positions[4].pos;
// const diffFull = Math.sqrt(
//   (pos0.x - posFull.x) ** 2 +
//     (pos0.y - posFull.y) ** 2 +
//     (pos0.z - posFull.z) ** 2,
// );

// console.log(`\n${"=".repeat(60)}`);
// console.log("🔄 PERIODICITY CHECK");
// console.log("=".repeat(60));
// console.log(
//   `Position at T=0:           (${(pos0.x / 1e9).toFixed(4)}, ${(pos0.y / 1e9).toFixed(4)}, ${(pos0.z / 1e9).toFixed(4)})`,
// );
// console.log(
//   `Position at T=full_period: (${(posFull.x / 1e9).toFixed(4)}, ${(posFull.y / 1e9).toFixed(4)}, ${(posFull.z / 1e9).toFixed(4)})`,
// );
// console.log(`Difference: ${(diffFull / 1e6).toFixed(2)} thousand km`);
// console.log(`Periodic: ${diffFull < 1e6 ? "✅ YES (< 1000 km)" : "❌ NO"}`);

// // Vérifier les distances min/max
// const distances = positions.map((p) => p.dist);
// const minDist = Math.min(...distances);
// const maxDist = Math.max(...distances);

// console.log(`\n${"=".repeat(60)}`);
// console.log("📏 DISTANCE RANGE");
// console.log("=".repeat(60));
// console.log(`Min distance observed: ${(minDist / 1e9).toFixed(4)} million km`);
// console.log(`Max distance observed: ${(maxDist / 1e9).toFixed(4)} million km`);
// console.log(
//   `Expected periapsis:    ${((info.semiMajorAxisKm * (1 - info.eccentricity)) / 1e6).toFixed(4)} million km`,
// );
// console.log(
//   `Expected apoapsis:     ${((info.semiMajorAxisKm * (1 + info.eccentricity)) / 1e6).toFixed(4)} million km`,
// );

// const periError = Math.abs(
//   minDist / 1e9 - (info.semiMajorAxisKm * (1 - info.eccentricity)) / 1e6,
// );
// const apoError = Math.abs(
//   maxDist / 1e9 - (info.semiMajorAxisKm * (1 + info.eccentricity)) / 1e6,
// );

// console.log(
//   `\nPeriapsis error: ${periError.toFixed(4)} million km ${periError < 0.1 ? "✅" : "⚠️"}`,
// );
// console.log(
//   `Apoapsis error:  ${apoError.toFixed(4)} million km ${apoError < 0.1 ? "✅" : "⚠️"}`,
// );

// console.log(`\n${"=".repeat(60)}`);
// console.log("✅ ALL TESTS PASSED - ORBIT CALCULATION IS CORRECT!");
// console.log("=".repeat(60));

// console.log("=".repeat(60));
// console.log("🔍 CACHE DEBUG TEST");
// console.log("=".repeat(60));

// const info = OrbitDataHelper.getOrbitalInfo(orbitalElements);

// // Vérifier la période orbitale
// console.log(`\n⏱️  Orbital info:`);
// console.log(
//   `  Period: ${info.periodSeconds.toFixed(0)}s (${info.periodDays.toFixed(2)} days)`,
// );
// console.log(`  Semi-major axis: ${info.semiMajorAxisKm.toFixed(0)} km`);

// console.log(`\n${"=".repeat(60)}`);
// console.log("📞 TEST 1: Small request");
// console.log("=".repeat(60));

// const params1: OrbitCalculationParams = {
//   objectId: planetData.uuid,
//   objectType: "planet",
//   startTimeS: 0,
//   durationS: 100, // 100 secondes
//   timestepS: 0.01666667,
//   orbitalElements,
// };

// console.log(`\n📊 Request:`);
// console.log(`  startTimeS: ${params1.startTimeS}`);
// console.log(`  durationS: ${params1.durationS}s`);
// console.log(`  timestepS: ${params1.timestepS}s`);
// console.log(
//   `  Expected samples: ${Math.ceil(params1.durationS / params1.timestepS)}`,
// );

// try {
//   const samples1 = keplerOrbitService.getPositions(params1);
//   console.log(`\n✅ SUCCESS: Received ${samples1.length} samples`);

//   if (samples1.length > 0) {
//     console.log(`   First sample: T=${samples1[0].timeS}s`);
//     console.log(`   Last sample:  T=${samples1[samples1.length - 1].timeS}s`);
//     console.log(
//       `   Position at T=0: (${(samples1[0].position.x / 1e9).toFixed(4)}, ${(samples1[0].position.y / 1e9).toFixed(4)}, ${(samples1[0].position.z / 1e9).toFixed(4)}) million km`,
//     );
//   }
// } catch (error) {
//   console.error(`\n❌ ERROR:`, error);
// }

// console.log(`\n${"=".repeat(60)}`);
// console.log("📞 TEST 2: Same request (should hit cache)");
// console.log("=".repeat(60));

// try {
//   const samples2 = keplerOrbitService.getPositions(params1);
//   console.log(`\n✅ SUCCESS: Received ${samples2.length} samples`);
// } catch (error) {
//   console.error(`\n❌ ERROR:`, error);
// }

// console.log(`\n${"=".repeat(60)}`);
// console.log("📊 CACHE STATS");
// console.log("=".repeat(60));

// const stats = keplerOrbitService.getCacheStats();
// console.log(`\nCache size: ${stats.size}`);
// stats.entries.forEach((entry, i) => {
//   console.log(`\nEntry ${i + 1}:`);
//   console.log(`  Key: ${entry.key}`);
//   console.log(`  Samples: ${entry.sampleCount.toLocaleString()}`);
//   console.log(`  Time range: ${entry.timeRange}`);
//   console.log(`  Access count: ${entry.accessCount}`);
// });

// console.log("🧪 Testing auto-prefetch...\n");

// // const keplerService = new KeplerOrbitService(
// //   { maxCacheSize: 20, cacheExpirationMs: 600000, evictionPolicy: "lru" },
// //   {
// //     enabled: true,
// //     multiplier: 10,
// //     maxDurationS: 600,
// //     minDurationS: 10,
// //     autoThreshold: 0.8, // Prefetch à 80%
// //   },
// // );

// const keplerService = keplerOrbitService;

// let currentTimeS = 0;
// const deltaTimeS = 0.01666667;
// const totalFrames = 20000;

// // ✅ Collecter les événements importants
// const events: Array<{
//   frame: number;
//   timeS: number;
//   event: string;
//   calcTimeMs: number;
//   cacheSize: number;
//   prefetching: number;
// }> = [];

// console.log(`\n⏱️  Simulating ${totalFrames} frames...`);
// console.log(`⏳ Running...\n`);

// const simStart = performance.now();

// // Simuler la game loop
// for (let frame = 0; frame < totalFrames; frame++) {
//   const params: OrbitCalculationParams = {
//     objectId: planetData.uuid as string,
//     objectType: "planet" as const,
//     startTimeS: currentTimeS,
//     durationS: 60,
//     timestepS: deltaTimeS,
//     orbitalObject: orbitalElements,
//   };

//   const cachePosition = keplerService.getCachePosition();
//   const frameStart = performance.now();
//   const samples = keplerService.getPositions(params);
//   const frameTime = performance.now() - frameStart;

//   const stats = cachePosition.getCacheStats();

//   // ✅ Enregistrer les frames importantes (tous les 1000 frames OU si calcul > 1ms)
//   if (frame % 1000 === 0 || frameTime > 1) {
//     events.push({
//       frame,
//       timeS: currentTimeS,
//       event: frameTime > 1 ? "CACHE MISS" : "CACHE HIT",
//       calcTimeMs: frameTime,
//       cacheSize: stats.size,
//       prefetching: stats.activePrefetches,
//     });
//   }

//   currentTimeS += deltaTimeS;
// }

// const simTime = performance.now() - simStart;

// console.log(`✅ Simulation complete in ${simTime.toFixed(0)}ms\n`);

// // ✅ AFFICHER LE RAPPORT
// console.log("=".repeat(60));
// console.log("📊 PERFORMANCE REPORT");
// console.log("=".repeat(60));

// console.log("\n🎯 IMPORTANT FRAMES:");
// console.log("─".repeat(60));
// console.log("Frame      Time(s)    Calc(ms)  Event         Cache  Prefetch");
// console.log("─".repeat(60));

// events.forEach((e) => {
//   console.log(
//     `${e.frame.toString().padStart(10)} ` +
//       `${e.timeS.toFixed(1).padStart(10)} ` +
//       `${e.calcTimeMs.toFixed(2).padStart(10)} ` +
//       ` ${e.event.padEnd(12)} ` +
//       `${e.cacheSize.toString().padStart(5)} ` +
//       `${e.prefetching.toString().padStart(8)}`,
//   );
// });

// // Stats finales
// const finalStats = keplerService.getCachePosition().getCacheStats();
// const hitFrames = events.filter((e) => e.event === "CACHE HIT").length;
// const missFrames = events.filter((e) => e.event === "CACHE MISS").length;
// const avgHitTime =
//   events
//     .filter((e) => e.event === "CACHE HIT")
//     .reduce((sum, e) => sum + e.calcTimeMs, 0) / hitFrames || 0;
// const avgMissTime =
//   events
//     .filter((e) => e.event === "CACHE MISS")
//     .reduce((sum, e) => sum + e.calcTimeMs, 0) / missFrames || 0;

// console.log(`\n${"=".repeat(60)}`);
// console.log("📈 STATISTICS");
// console.log("=".repeat(60));
// console.log(`Total frames simulated: ${totalFrames.toLocaleString()}`);
// console.log(`Total simulation time: ${simTime.toFixed(0)}ms`);
// console.log(
//   `Cache hits: ~${(totalFrames - missFrames).toLocaleString()} (~${(((totalFrames - missFrames) / totalFrames) * 100).toFixed(2)}%)`,
// );
// console.log(`Cache misses: ~${missFrames}`);
// console.log(`Avg time (cache hit): ${avgHitTime.toFixed(3)}ms`);
// console.log(`Avg time (cache miss): ${avgMissTime.toFixed(3)}ms`);
// console.log(
//   `Speedup with cache: ${(avgMissTime / avgHitTime).toFixed(0)}× faster`,
// );

// console.log(`\n${"=".repeat(60)}`);
// console.log("💾 FINAL CACHE STATE");
// console.log("=".repeat(60));
// console.log(`Cache entries: ${finalStats.size}`);
// console.log(`Active prefetches: ${finalStats.activePrefetches}`);

// finalStats.entries.forEach((entry, i) => {
//   console.log(`\nCache ${i + 1}:`);
//   console.log(`  Range: ${entry.timeRange}`);
//   console.log(`  Samples: ${entry.sampleCount.toLocaleString()}`);
//   console.log(`  Memory: ${entry.memoryMB.toFixed(1)} MB`);
//   console.log(`  Hits: ${entry.accessCount.toLocaleString()}`);
//   console.log(`  Age: ${(entry.ageMs / 1000).toFixed(1)}s`);
// });

// console.log(`\n${"=".repeat(60)}`);
// console.log("✅ TEST COMPLETE");
// console.log("=".repeat(60));

const ws = new WebSocket("ws://localhost:9200");

const DURATION = 5;
const FREQUENCY = 60;

ws.on("open", () => {
  console.log("✅ Connected to WebSocket server");

  const requestInit: RequestInitWsType = {
    event_type: "init",
    data: {
      duration_s: DURATION,
      frequency: FREQUENCY,
      from_timestamp: 0,
      system_internal_name: "tarsis",
    },
  };

  ws.send(encode(requestInit));
  // ws.send(JSON.stringify(requestInit));

  let fromTime = 0;
  setInterval(() => {
    const nextTicksRequest: RequestTransformWsType = {
      event_type: "transform",
      data: {
        duration_s: DURATION,
        frequency: FREQUENCY,
        from_timestamp: fromTime,
        uuid: "88f3a0af-28c7-42f6-8228-551a98fc55cd", // Lune
        // uuid: "844221a5-e2be-432c-94c2-947462c1c310", // Planet Tarsis 1
      },
    };
    // ws.send(encode(nextTicksRequest));
    ws.send(JSON.stringify(nextTicksRequest));
    fromTime += DURATION;
  }, 1000);
});

ws.on("message", (data) => {
  const decoded = decode(data as Buffer) as ResponseWsType;
  // const decoded = JSON.parse((data as Buffer).toString()) as ResponseWsType;

  if ("data" in decoded) {
    console.log(decoded.data);
    // console.log("🎯 Received:", decoded.data[2].object_data.rotations);
    if ("object_data" in decoded.data) {
      // const positions = decoded.data.object_data.positions;
      // const timeS = decoded.data.object_data.from_timestamp;
      // const rotations = decoded.data.object_data.rotations as Vector3Type[];
      // const positions = decoded.data.object_data.positions as Vector3Type[];
      //       if (positions) {
      //   positions.map((position) => console.log(position));
      // }
      // if (rotations) {
      //   rotations.map((rotation) => console.log(rotation));
      // }
      // console.log(timeS);
    }
  } else {
    console.log("🎯 Received:", decoded);
  }
});
