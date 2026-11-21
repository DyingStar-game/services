#!/usr/bin/env python3
"""Propose the farm villages of a planet (ADR services/0010).

Farm villages are sealed greenhouses on the high plateaus. The POIs do not exist yet; this
script proposes where they go, to be imported into QGIS as a `farm village` POI layer:

  - above `farms.min_altitude_m` (the planet heightmap of the DyingStar export),
  - on the `farms.biome` biome (first matching zone of biomes.dsmpart, as the game reads it),
  - within `farms.max_road_m` of a road,
  - between `farms.hub_distance_m` of a factory, which becomes its hub (and is its nearest
    factory, as universesimulator's import picks it),
  - at least `farms.min_spacing_m` from another farm.

The number of farms is the one the food budget asks for (`--count`, default: computed by
scripts/village_gen.py from the diets and the farm archetype). Factories are served in order
of how many mining villages they already serve, one farm each, then again.

Writes config/world/<planet>/farm_villages.json (the POI format of poi.json, plus details) and
farm_villages.geojson (points for QGIS).

  python3 scripts/village_farms.py [--dyingstar ../../DyingStar] [--count N]
  python3 scripts/village_farms.py --check   # fail if the committed files are stale

Needs numpy and GDAL (osgeo).
"""

import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np
import yaml

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))
import village_geology  # noqa: E402  (same directory)


def haversine(lon1, lat1, lon2, lat2, radius):
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dlat, dlon = p2 - p1, math.radians(lon2 - lon1)
    a = math.sin(dlat / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dlon / 2) ** 2
    return 2 * radius * math.asin(min(1.0, math.sqrt(a)))


def road_distance(lon, lat, roads, radius):
    """Distance to the nearest road centerline (local equirectangular projection)."""
    k = math.cos(math.radians(lat))
    best = math.inf
    for line in roads:
        for (x1, y1), (x2, y2) in zip(line, line[1:]):
            ax, ay = (x1 - lon) * k, y1 - lat
            bx, by = (x2 - lon) * k, y2 - lat
            dx, dy = bx - ax, by - ay
            t = 0.0 if dx == dy == 0 else max(0.0, min(1.0, -(ax * dx + ay * dy) / (dx * dx + dy * dy)))
            best = min(best, math.hypot(ax + t * dx, ay + t * dy))
    return math.radians(best) * radius


def build(args, count):
    sc = yaml.safe_load((ROOT / "config/village/supply_chain.yaml").read_text())
    fc = sc["farms"]
    planet = sc["planet"]
    from osgeo import gdal

    gdal.UseExceptions()
    raster = gdal.Open(str(args.dyingstar / f"assets/qgis/export/{planet}_heightmap.tif"))
    heights = raster.GetRasterBand(1).ReadAsArray().astype(np.float32)
    raster = None
    h, w = heights.shape
    poi_file = json.loads((ROOT / f"config/world/{planet}/poi.json").read_text())
    radius = poi_file["planet_radius"]
    pois = poi_file["pois"]
    roads = [f["properties"]["centerline"] for f in json.loads((ROOT / f"config/world/{planet}/roads.json").read_text())["features"]]
    dsmp, geom = village_geology.load_dyingstar(args.dyingstar)
    export = args.dyingstar / "assets/qgis/export" / f"{planet}_chunks/parts"
    part = dsmp.read_container(str(export / "biomes.dsmpart"))
    strings = json.loads((export / "strings.json").read_text())

    factories = [p for p in pois if p["poi_type"] == "factory village"]
    mines = [p for p in pois if p["poi_type"] == "mining village"]
    served = {f["name"]: 0 for f in factories}
    for m in mines:
        nearest = min(factories, key=lambda f: haversine(m["lon"], m["lat"], f["lon"], f["lat"], radius))
        served[nearest["name"]] += 1

    lo, hi = fc["hub_distance_m"]
    ys, xs = np.nonzero(heights > fc["min_altitude_m"])
    lats = 90.0 - (ys + 0.5) * 180.0 / h
    lons = -180.0 + (xs + 0.5) * 360.0 / w
    candidates = {}
    for f in factories:
        near = np.abs(lats - f["lat"]) < math.degrees(hi / radius) + 0.1
        options = []
        for lon, lat, y, x in zip(lons[near], lats[near], ys[near], xs[near]):
            d = haversine(f["lon"], f["lat"], lon, lat, radius)
            if not lo <= d <= hi:
                continue
            # The import makes the nearest factory the hub: it must be this one.
            if any(haversine(o["lon"], o["lat"], lon, lat, radius) < d for o in factories if o is not f):
                continue
            road = road_distance(lon, lat, roads, radius)
            if road > fc["max_road_m"]:
                continue
            zone = village_geology.rock_at(dsmp, geom, part, strings, float(lon), float(lat))
            if not zone or zone["biome"] != fc["biome"]:
                continue
            options.append({"lon": float(lon), "lat": float(lat), "elevation_m": float(heights[y, x]), "road_m": road, "hub_m": d, **zone})
        # Closest to the road first, then closest to the factory.
        candidates[f["name"]] = sorted(options, key=lambda o: (round(o["road_m"] / 500), o["hub_m"]))

    order = sorted((f for f in factories if candidates[f["name"]]), key=lambda f: (-served[f["name"]], f["name"]))
    chosen = []
    while len(chosen) < count and order:
        progressed = False
        for f in order:
            if len(chosen) >= count:
                break
            pick = next((o for o in candidates[f["name"]]
                         if all(haversine(o["lon"], o["lat"], c["lon"], c["lat"], radius) >= fc["min_spacing_m"] for c in chosen)), None)
            if pick:
                chosen.append({**pick, "hub": f["name"]})
                candidates[f["name"]].remove(pick)
                progressed = True
        if not progressed:
            break
    farms = []
    for i, c in enumerate(sorted(chosen, key=lambda c: (c["hub"], c["lon"]))):
        farms.append({
            "id": f"farm_{i + 1:04d}", "name": f"{fc['name_prefix']}{i + 1:04d}", "poi_type": "farm village", "radius": 2000,
            "lon": round(c["lon"], 6), "lat": round(c["lat"], 6), "elevation_m": round(c["elevation_m"]),
            "road_m": round(c["road_m"]), "hub": c["hub"], "hub_m": round(c["hub_m"]),
            "biome": c["biome"], "rock_type": c["rock_type"], "clarity": c["clarity"],
        })
    out = {"_note": "GENERATED by npc/scripts/village_farms.py (ADR services/0010) — proposed farm villages, to import into QGIS.",
           "planet_name": planet, "planet_radius": radius, "criteria": fc, "pois": farms}
    geojson = {"type": "FeatureCollection", "name": f"{planet}_farm_villages",
               "crs": {"type": "name", "properties": {"name": "urn:ogc:def:crs:OGC:1.3:CRS84"}},
               "features": [{"type": "Feature", "properties": {k: v for k, v in f.items() if k not in ("lon", "lat")},
                             "geometry": {"type": "Point", "coordinates": [f["lon"], f["lat"]]}} for f in farms]}
    return out, geojson, len(chosen) < count


def required_count():
    """Farms the food budget asks for (scripts/village_gen.py)."""
    import village_gen
    return village_gen.farms_needed()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dyingstar", type=Path, default=ROOT.parent.parent / "DyingStar")
    ap.add_argument("--count", type=int, help="number of farms (default: what the food budget needs)")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    count = args.count or required_count()
    out, geojson, short = build(args, count)
    planet = out["planet_name"]
    paths = (ROOT / f"config/world/{planet}/farm_villages.json", ROOT / f"config/world/{planet}/farm_villages.geojson")
    texts = (json.dumps(out, indent=2, ensure_ascii=False) + "\n", json.dumps(geojson, indent=2, ensure_ascii=False) + "\n")
    for f in out["pois"]:
        print(f"{f['name']}  {f['lat']:9.4f} {f['lon']:10.4f}  {f['elevation_m']:5d} m  road {f['road_m']:5d} m  hub {f['hub']} {f['hub_m'] / 1000:5.1f} km  {f['rock_type']}")
    if short:
        print(f"only {len(out['pois'])} of {count} farms could be placed with these criteria", file=sys.stderr)
    if args.check:
        stale = any(not p.exists() or p.read_text() != t for p, t in zip(paths, texts))
        if stale:
            print("farm_villages.json/.geojson are stale: rerun without --check", file=sys.stderr)
        return 1 if stale or short else 0
    for p, t in zip(paths, texts):
        p.write_text(t)
        print(f"wrote {p.relative_to(ROOT)}")
    return 1 if short else 0


if __name__ == "__main__":
    sys.exit(main())
