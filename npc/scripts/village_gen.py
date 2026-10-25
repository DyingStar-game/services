#!/usr/bin/env python3
"""Generate the villages of a planet: rosters, budgets and the universesimulator world file
(ADR services/0002-0004, 0007-0010).

Reads, under config/:
  village/<archetype>.yaml     the archetypes (mining_village, farm_village): roles, buildings,
                               housing, coefficients
  schedules.yaml               the rotas
  food/consumables.yaml        consumables, their composition, the diets
  village/supply_chain.yaml    the planet's chain: villages, refining, food, trade
  world/tarsis_3/village_geology.json  the rock under every village (scripts/village_geology.py)
  world/tarsis_3/farm_villages.json    the farm villages (scripts/village_farms.py)

Writes:
  npcs.<archetype>.yaml                          the roster of the archetype, one NpcSpec each
  village/budget.<archetype>.generated.md        staffing, building load, resources
  village/supply_chain.generated.md              ore, food and trade for the planet
  village/usim_profile.generated.json            universesimulator world-file sections
  (and, with --write-seed, universesimulator/seeds/tarsis_3-ares.json itself)

  python3 scripts/village_gen.py [--village-uuid UUID] [--planet tarsis_3] [--write-seed]
  python3 scripts/village_gen.py --check      # validate only, write nothing

Paths are relative to the npc/ directory. Needs PyYAML.
"""

import argparse
import json
import math
import random
import sys
import uuid
from collections import Counter, defaultdict
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
VILLAGES = ROOT / "config/village"
SCHEDULES = ROOT / "config/schedules.yaml"
SAMPLE = ROOT / "config/npcs.yaml.sample"
FOOD = ROOT / "config/food/consumables.yaml"
CHAIN = VILLAGES / "supply_chain.yaml"
SEED = ROOT.parent / "universesimulator/seeds/tarsis_3-ares.json"
OUT_USIM = VILLAGES / "usim_profile.generated.json"
OUT_CHAIN = VILLAGES / "supply_chain.generated.md"

# Namespace of the archetypes' NPC uuids: uuid5(NS, "<archetype>/<role>/<rank>").
NS = uuid.UUID("6f1d2a7e-3b0c-4c5e-9a43-8d2f1e0b7c11")
PLACEHOLDER_VILLAGE = "00000000-0000-0000-0000-000000000000"
HOURS = 25
DAYS = 5
WORK = set("Tt")
# Ingredient origins brought in through the spaceports: moon 2 (water, salts) and off-system.
BROUGHT_IN = ("moon", "import")
SPACING_M = 1.5


class CheckError(Exception):
    pass


def world_file(cfg, name):
    return ROOT / f"config/world/{cfg['planet']}/{name}"


def load():
    cfg = yaml.safe_load(CHAIN.read_text())
    archetypes = {a: yaml.safe_load((VILLAGES / f"{a}.yaml").read_text()) for a in cfg["archetypes"]}
    sched = yaml.safe_load(SCHEDULES.read_text())["schedules"]
    sample = yaml.safe_load(SAMPLE.read_text())["npcs"]
    food = yaml.safe_load(FOOD.read_text())
    geology = json.loads(world_file(cfg, "village_geology.json").read_text())
    farms_path = world_file(cfg, "farm_villages.json")
    farms = json.loads(farms_path.read_text())["pois"] if farms_path.exists() else []
    return cfg, archetypes, sched, sample, food, geology, farms


def cells(pattern):
    """The 125 hourly letters of a 5-day pattern."""
    return "".join(pattern["days"])


def shifts(c):
    """Number of work blocks in a cyclic pattern: maximal runs of non-`.`/`l`
    hours that contain at least one `T`."""
    n, in_block, has_t = len(c), False, False
    # Start right after an off hour so a block wrapping day 5 -> day 1 counts once.
    start = next((i + 1 for i in range(n) if c[i] in ".l"), 0)
    count = 0
    for k in range(n):
        ch = c[(start + k) % n]
        if ch in ".l":
            if in_block and has_t:
                count += 1
            in_block, has_t = False, False
        else:
            in_block = True
            has_t = has_t or ch == "T"
    if in_block and has_t:
        count += 1
    return count


def spot(building, layout, index, slots):
    """Position of the index-th of `slots` spots spread over a building's footprint."""
    mpp = layout["metres_per_px"]
    ox, oy = layout["origin_px"]
    cx, cy = building["center_px"]
    w, h = (s * mpp for s in building["size_px"])
    cols = max(1, int(w // SPACING_M))
    rows = max(1, int(h // SPACING_M))
    if slots > cols * rows:  # dense building: shrink the grid to fit everyone
        cols = max(1, int((slots * w / h) ** 0.5))
        rows = -(-slots // cols)
    dx = (index % cols + 0.5) * (w / cols) - w / 2
    dz = ((index // cols) % rows + 0.5) * (h / rows) - h / 2
    return {"x": round((cx - ox) * mpp + dx, 2), "y": 0.0, "z": round((cy - oy) * mpp + dz, 2)}


def build(arch, sched, sample, village_uuid, planet):
    """One NPC per inhabitant of an archetype, with its rota, workplace, home and places."""
    buildings, layout, rotas = arch["buildings"], arch["layout"], arch["rotas"]
    storekeepers = {s["schedule"]: s for s in sample if str(s.get("schedule", "")).startswith("magasinier_")}

    # Housing units: (building, unit index) pools, players' units taken out first.
    player = arch["housing"]["player_units"]
    free = {b: list(range(d.get("units", 0))) for b, d in buildings.items() if d.get("units")}
    reserved = free[player["building"]][-player["units"]:]
    del free[player["building"]][-player["units"]:]

    def take_home(kind):
        order = {"family": arch["housing"]["family_order"], "dorm": arch["housing"]["dorm_order"],
                 "refugee": ["refugee_housing"]}[kind]
        for b in order:
            if free.get(b):
                return b, free[b].pop(0)
        raise CheckError(f"no free dwelling left for a `{kind}` inhabitant")

    names = [f"{f} {l}" for l in arch["names"]["last"] for f in arch["names"]["first"]]
    random.Random(arch["archetype"]).shuffle(names)  # deterministic, surnames mixed
    used_names = {s["name"] for s in storekeepers.values()}
    name_iter = (n for n in names if n not in used_names)
    seen = set()

    def next_name():
        for n in name_iter:
            if n not in seen:
                seen.add(n)
                return n
        raise CheckError("name pool exhausted")

    place = lambda b, i, slots: {"uuid": village_uuid, "planet": planet, "position": spot(buildings[b], layout, i, slots)}
    load = Counter()  # how many NPCs use a building as a spot, to spread them
    totals = Counter()
    for r in arch["roles"]:
        for b in (r.get("work"), r.get("mandatory"), arch["meal_building"]):
            if b:
                totals[b] += r["count"]

    npcs = []
    for role in arch["roles"]:
        rota = role["rota"]
        for rank in range(role["count"]):
            if role.get("patterns"):
                pattern = role["patterns"][rank % len(role["patterns"])]
            else:
                pattern = rotas[rota][rank % len(rotas[rota])]
            home_b, unit = take_home(role["housing"])
            home = place(home_b, unit, buildings[home_b]["units"])
            if role.get("from_sample"):
                spec = dict(storekeepers[pattern])
                spec["home"] = home
                npcs.append({"role": role, "pattern": pattern, "home": (home_b, unit), "spec": spec})
                continue
            actions = list(dict.fromkeys(arch["base_actions"] + role.get("actions", [])))
            if not role.get("work"):
                actions.remove("going_to_work")
            spec = {
                "name": next_name(),
                "uuid": str(uuid.uuid5(NS, f"{arch['archetype']}/{role['id']}/{rank}")),
                "schedule": pattern,
                "action_profile": actions,
            }
            if role.get("work"):
                spec["place_where_work"] = place(role["work"], load[role["work"]], totals[role["work"]])
                load[role["work"]] += 1
            spec["home"] = home
            meal = arch["meal_building"]
            spec["meal_place"] = place(meal, load[meal], totals[meal])
            load[meal] += 1
            mand = role["mandatory"]
            spec["mandatory_place"] = place(mand, load[mand], totals[mand])
            load[mand] += 1
            npcs.append({"role": role, "pattern": pattern, "home": (home_b, unit), "spec": spec})
    return npcs, reserved, free


def check(arch, sched, npcs, reserved):
    errors = []
    for name, p in sched.items():
        if len(p["days"]) != DAYS or any(len(d) != HOURS for d in p["days"]):
            errors.append(f"schedule {name}: needs {DAYS} rows of {HOURS} letters")
    if len(npcs) != arch["population"]:
        errors.append(f"population {len(npcs)} != {arch['population']}")
    for r in arch["roles"]:
        if r["rota"] in ("crew", "magasinier") and r["count"] % 5:
            errors.append(f"role {r['id']}: crew headcount {r['count']} is not a multiple of 5")
        for p in r.get("patterns", []):
            if p not in sched:
                errors.append(f"role {r['id']}: unknown pattern {p}")
    for n in npcs:
        if n["pattern"] not in sched:
            errors.append(f"{n['spec']['name']}: unknown pattern {n['pattern']}")
    homes = Counter(n["home"] for n in npcs)
    errors += [f"dwelling {h} shared by {c} NPCs" for h, c in homes.items() if c > 1]
    errors += [f"dwelling {arch['housing']['player_units']['building']}#{u} reserved for players is taken"
               for u in reserved if (arch["housing"]["player_units"]["building"], u) in homes]
    uuids = Counter(n["spec"]["uuid"] for n in npcs)
    errors += [f"uuid {u} used {c} times" for u, c in uuids.items() if c > 1]
    names = Counter(n["spec"]["name"] for n in npcs)
    errors += [f"name {u} used {c} times" for u, c in names.items() if c > 1]
    # Every 24/7 role has someone on duty at every hour of the cycle.
    for r in arch["roles"]:
        if r["rota"] not in ("crew", "magasinier"):
            continue
        members = [cells(sched[n["pattern"]]) for n in npcs if n["role"] is r]
        for h in range(DAYS * HOURS):
            if not any(c[h] in WORK for c in members):
                errors.append(f"role {r['id']}: nobody on duty on day {h // HOURS + 1} hour {h % HOURS}")
                break
    staffed = {b for r in arch["roles"] for b in (r.get("work"), r.get("mandatory")) if b}
    staffed |= {arch["meal_building"]} | {b for b, d in arch["buildings"].items() if d.get("units")}
    for b in arch["buildings"]:
        if b not in staffed and b not in arch["unstaffed"]:
            errors.append(f"building {b}: nobody works or lives there and it is not listed as unstaffed")
    if errors:
        raise CheckError("\n".join(errors))


# ── Food (ADR services/0009) ─────────────────────────────────────────────────

def check_food(food, archetypes):
    errors = []
    for code, c in food["consumables"].items():
        total = sum(x["pct"] for x in c["composition"])
        if abs(total - 100) > 0.01:
            errors.append(f"consumable {code}: composition sums to {total} %")
        for x in c["composition"]:
            if x["ingredient"] not in food["ingredients"]:
                errors.append(f"consumable {code}: unknown ingredient {x['ingredient']}")
    for diet, menu in food["diets"].items():
        errors += [f"diet {diet}: unknown consumable {c}" for c in menu if c not in food["consumables"]]
    for a, arch in archetypes.items():
        errors += [f"{a}: role {r['id']} has unknown diet {r['diet']}" for r in arch["roles"] if r.get("diet", "worker") not in food["diets"]]
    if errors:
        raise CheckError("\n".join(errors))


def person_units(food, diet="worker", adult=True):
    """Units of each consumable one person eats per day."""
    out = Counter(food["diets"][diet])
    if adult:
        out.update(food["extras"])
    return out


def ingredients_of(food, code):
    """kg of each ingredient per kg of a consumable."""
    out = Counter()
    for x in food["consumables"][code]["composition"]:
        out[x["ingredient"]] += x["pct"] / 100
    return out


def batch_recipe(food, code, min_input_kg):
    """A batch of the consumable: (units per batch, {ingredient: whole kg per batch}). The batch
    is the smallest round number of units (1, 2, 5 × 10^k) whose smallest input is at least
    `min_input_kg`; inputs are rounded to whole kg."""
    c = food["consumables"][code]
    fractions = ingredients_of(food, code)
    smallest = min(v for v in fractions.values() if v > 0) * c["unit_kg"]
    need = min_input_kg / smallest
    k = 0
    while True:
        for m in (1, 2, 5):
            n = m * 10 ** k
            if n >= need:
                return n, {i: max(1, round(v * c["unit_kg"] * n)) for i, v in fractions.items() if v > 0}
        k += 1


# ── Budget of one archetype village ──────────────────────────────────────────

def budget(arch, sched, npcs, food):
    """Staffing, consumption and production of one archetype village, from its rotas."""
    b = arch["budget"]
    pop = len(npcs)
    hourly = defaultdict(lambda: [0] * (DAYS * HOURS))  # role -> on duty per hour
    occupancy = defaultdict(lambda: [0] * (DAYS * HOURS))  # building -> people per hour
    ore = b.get("ore")
    miner_t_hours = 0
    showers = 0
    driver_hours = 0
    production = Counter()  # resource -> kg per cycle
    units = Counter()  # consumable -> units per day
    for n in npcs:
        role, c = n["role"], cells(sched[n["pattern"]])
        for h, ch in enumerate(c):
            if ch in WORK:
                hourly[role["id"]][h] += 1
            where = {"T": role.get("work"), "t": role.get("work"), "a": role.get("mandatory"),
                     "r": arch["meal_building"], "l": "(leisure: bar, shop)"}.get(ch) or n["home"][0]
            occupancy[where][h] += 1
        if ore and role["id"] == ore["producer_role"]:
            miner_t_hours += c.count("T")
        for res, kg in role.get("produces", {}).items():
            production[res] += kg * c.count("T")
        if role.get("mine_side"):
            showers += shifts(c)
        if role["id"] == b["transport"]["driver_role"]:
            driver_hours += sum(ch in WORK for ch in c)
        units.update(person_units(food, role.get("diet", "worker"), adult=role["id"] != "child"))

    per_day = lambda cycle_total: cycle_total / DAYS
    production = {r: per_day(v) for r, v in production.items()}
    raw_day = per_day(miner_t_hours) * ore["raw_kg_per_worker_hour"] if ore else 0.0
    conc_day = raw_day * ore["pretreatment_yield"] if ore else 0.0
    food_kg_day = sum(u * food["consumables"][c]["unit_kg"] for c, u in units.items())
    w = b["water"]
    water_pop_day = pop * w["l_per_person_day"]
    water_ind_day = (per_day(showers) * w.get("shower_l_per_mine_shift", 0) + raw_day * w.get("process_l_per_raw_kg", 0)
                     + sum(production.values()) * w.get("irrigation_l_per_kg", 0))
    water_day = water_pop_day + water_ind_day

    t = b["transport"]
    out_day = conc_day + sum(production.values())
    out_loads = out_day / t["truck_payload_kg"]
    import_loads = (water_day + food_kg_day) / t["truck_payload_kg"]
    trips = max(out_loads, import_loads)  # imports ride back on the outbound trucks
    hours_needed = trips * (t["road_round_trip_h"] + t["handling_h"])
    hours_available = per_day(driver_hours)

    sd = b["stock_days"]
    levels = lambda per_day: {k: per_day * sd[k] for k in ("min_threshold", "target_level", "capacity")}
    stocks = {"water": levels(water_day), **{c: levels(u) for c, u in sorted(units.items())}}
    if conc_day:
        stocks["ore"] = {"capacity": conc_day * sd["ore_capacity"]}
    for r, v in production.items():
        stocks[r] = {"capacity": v * sd["ore_capacity"]}
    return {
        "pop": pop, "hourly": hourly, "occupancy": occupancy, "units": dict(units), "food_kg_day": food_kg_day,
        "raw_day": raw_day, "conc_day": conc_day, "production": production, "water_pop_day": water_pop_day,
        "water_ind_day": water_ind_day, "water_day": water_day, "showers_day": per_day(showers),
        "miner_hours_day": per_day(miner_t_hours), "out_loads": out_loads, "import_loads": import_loads, "trips": trips,
        "hours_needed": hours_needed, "hours_available": hours_available, "stocks": stocks,
    }


# ── Planet supply chain (ADR services/0004, 0007-0010) ───────────────────────

def rock_yields(rock, cfg):
    """kg of each refined element per kg of the rock's ore: the host crystal's aluminium
    (Al2O3, less the impurities) and the mid-range ppm of each impurity, times the refining
    yield (ADR services/0008)."""
    y = cfg["refining_yield"]
    impurities = {i["element"]: (i["ppm_min"] + i["ppm_max"]) / 2e6 for i in rock["impurities"]}
    host = rock["purity"] if rock.get("purity") else 1.0 - sum(impurities.values())
    out = {cfg["host_element"]["element"]: host * cfg["host_element"]["fraction"] * y}
    for element, frac in impurities.items():
        out[element] = out.get(element, 0.0) + frac * y
    return {e: v for e, v in out.items() if v > 0}


def demand(cfg, buds, food, n_farm):
    """Units of each consumable the planet eats per day, and what they are made of (kg/day)."""
    n, ref = cfg["villages"], cfg["reference_population"]
    worker = person_units(food)
    per_village = {
        "mining": Counter(buds["mining_village"]["units"]),
        "farm": Counter(buds["farm_village"]["units"]),
        **{role: Counter({c: u * ref[role] for c, u in worker.items()}) for role in ("factory", "railway", "spaceport")},
    }
    counts = {"mining": n["mining"], "farm": n_farm, "factory": n["factory"], "railway": n["railway"], "spaceport": n["spaceport"]}
    planet = Counter()
    for role, units in per_village.items():
        for c, u in units.items():
            planet[c] += u * counts[role]
    ingredients = Counter()
    for c, u in planet.items():
        for i, frac in ingredients_of(food, c).items():
            ingredients[i] += u * food["consumables"][c]["unit_kg"] * frac
    return per_village, counts, planet, ingredients


def farms_for(cfg, buds, food, ingredients):
    """Farm villages needed so the farms grow every raw ingredient the planet eats."""
    produced = buds["farm_village"]["production"]
    need = {i: math.ceil(v / produced[i]) for i, v in ingredients.items() if food["ingredients"][i]["origin"] == "farm" and v > 0}
    return max(need.values()), need


def chain(cfg, buds, geology, farms, food):
    """universesimulator world-file sections for the whole planet, and a summary."""
    mining_bud, farm_bud = buds["mining_village"], buds["farm_village"]
    sd = {"min_threshold": 1.5, "target_level": 3, "capacity": 5, "ore_capacity": 2}
    rnd = lambda x, d=1: round(x, d)
    whole = lambda x, d=0: int(round(x, d))  # the seed wants integers
    ph = lambda per_day: per_day / HOURS
    metals, price = cfg["metals"], lambda value: cfg["value_prices"][value]
    lot = cfg["push_lot_kg"]
    n, ref = cfg["villages"], cfg["reference_population"]
    n_farm = len(farms)

    # ── Ore (ADR services/0008) ──
    mines = {name: v for name, v in geology["villages"].items() if v["poi_type"] == "mining village"}
    rocks = sorted({v["rock_type"] for v in mines.values()})
    yields = {r: rock_yields(geology["rocks"][r], cfg) for r in rocks}
    elements = sorted({e for y in yields.values() for e in y}, key=lambda e: list(metals).index(e))
    ore_code = lambda r: f"ore_{r}"
    ore_price = {r: rnd(sum(v * price(metals[e]["value"]) for e, v in yields[r].items()) * cfg["ore_price_share"], 2) for r in rocks}
    ore_day = mining_bud["conc_day"]
    mined_by_rock = {r: sum(1 for v in mines.values() if v["rock_type"] == r) * ore_day for r in rocks}
    metal_day = {e: sum(mined_by_rock[r] * yields[r].get(e, 0.0) for r in rocks) for e in elements}
    served = n["mining"] / n["factory"]
    factory_ore_h = ph(ore_day) * served

    # ── Food (ADR services/0009, 0010) ──
    per_village, counts, planet_units, ingredients = demand(cfg, buds, food, n_farm)
    needed, need_by = farms_for(cfg, buds, food, ingredients)
    consumables = food["consumables"]
    raw = [i for i, d in food["ingredients"].items() if d["origin"] == "farm"]
    hubs = sorted({f["hub"] for f in farms})
    n_hub = max(1, len(hubs))
    recipes = {c: batch_recipe(food, c, cfg["food_batch_min_input"]) for c in consumables}
    made_at = lambda where: [c for c in consumables if consumables[c]["made_by"] == where]
    # Per day: what a hub factory and a farm make, and the ingredients they use.
    def use(codes, share):
        out = Counter()
        for c in codes:
            batches = planet_units[c] / recipes[c][0] / share
            for i, kg in recipes[c][1].items():
                out[i] += batches * kg
        return out
    hub_use = use(made_at("factory"), n_hub)
    farm_use = use(made_at("farm"), max(1, n_farm))
    rate = lambda c, share: rnd(ph(planet_units[c] / recipes[c][0] / share) * cfg["food_capacity_factor"], 2)
    process = lambda c, share, prefix: {
        "code": f"{prefix}_{c}", "ratePerHour": rate(c, share), "label": f"{consumables[c]['name']} ({recipes[c][0]} {consumables[c]['unit']}s)",
        "inputs": [{"resource": i, "quantity": kg} for i, kg in recipes[c][1].items()],
        "outputs": [{"resource": c, "quantity": float(recipes[c][0])}],
    }
    # Shares of the food mass by origin (recipe water aside: it is water, not food).
    by_origin = Counter()
    for i, v in ingredients.items():
        if i != "water":
            by_origin[food["ingredients"][i]["origin"]] += v
    local_kg = by_origin["farm"]
    food_kg = sum(by_origin.values())

    # ── Resources ──
    raw_price = lambda value: {"bon marché": 1.0, "normal": 3.0}.get(value, 3.0)
    resources = [
        {"code": "water", "name": "Water", "category": "water", "unit": "l", "inventoryGoodType": "water", "massPerUnitKg": 1.0, "basePrice": 2.0},
        *[{"code": c, "name": d["name"], "category": "food", "unit": d["unit"], "inventoryGoodType": c, "massPerUnitKg": d["unit_kg"], "basePrice": float(d["price"])}
          for c, d in consumables.items()],
        *[{"code": i, "name": d["name"], "category": "food" if d["origin"] == "farm" else "material", "unit": "kg", "inventoryGoodType": i,
           "massPerUnitKg": 1.0, "basePrice": raw_price(d["value"])} for i, d in food["ingredients"].items() if i != "water"],
        *[{"code": ore_code(r), "name": f"{geology['rocks'][r]['label']} ore", "category": "ore", "unit": "kg", "inventoryGoodType": ore_code(r),
           "massPerUnitKg": 1.0, "basePrice": ore_price[r]} for r in rocks],
        *[{"code": metals[e]["code"], "name": metals[e]["name"], "category": "material", "unit": "kg", "inventoryGoodType": metals[e]["code"],
           "massPerUnitKg": 1.0, "basePrice": float(price(metals[e]["value"]))} for e in elements],
    ]

    # ── Helpers for profiles ──
    level = lambda per_day, key: whole(per_day * sd[key])
    pull = lambda res, per_day, per_thousand=False: {"resource": res, "minThreshold": level(per_day, "min_threshold"), "targetLevel": level(per_day, "target_level"),
                                                    **({"perThousand": True} if per_thousand else {})}
    capacity = lambda res, per_day, days="capacity", initial=True, d=0: {"resource": res, "capacity": max(1, whole(per_day * sd[days], d)),
                                                                        "initialStock": whole(per_day * sd["target_level"], d) if initial else 0}
    def food_profile(units, pop):
        """Consumption of a village's consumables, per thousand inhabitants."""
        k = 1000 / pop
        return (
            [pull(c, u * k, True) for c, u in sorted(units.items())],
            [{"resource": c, "ratePerHour": rnd(-ph(u) * k, 3), "perThousand": True, "source": "population", "label": "Inhabitants"} for c, u in sorted(units.items())],
            [capacity(c, u) for c, u in sorted(units.items())],
        )
    def water_flows(bud, k):
        return [{"resource": "water", "ratePerHour": rnd(-ph(bud["water_pop_day"]) * k), "perThousand": True, "source": "population", "label": "Inhabitants"},
                {"resource": "water", "ratePerHour": rnd(-ph(bud["water_ind_day"]) * k), "perThousand": True, "source": "industry", "label": "Industry"}]

    # Defaults: every village eats the worker diet (factories, railway cities, spaceports).
    worker = person_units(food)
    d_pol, d_flows, d_caps = food_profile(Counter({c: u * 1000 for c, u in worker.items()}), 1000)
    defaults = {"policies": d_pol, "flows": d_flows, "capacities": d_caps}

    # Mining villages.
    k = 1000 / mining_bud["pop"]
    m_pol, m_flows, m_caps = food_profile(mining_bud["units"], mining_bud["pop"])
    mining = {
        "policies": [pull("water", mining_bud["water_day"] * k, True), *m_pol],
        "flows": [*water_flows(mining_bud, k), *m_flows],
        "capacities": [capacity("water", mining_bud["water_day"], d=-2), *m_caps],
    }
    overrides = {}
    for name, v in sorted(mines.items()):
        ore = ore_code(v["rock_type"])
        overrides[name] = {
            "policies": [{"resource": ore, "minThreshold": 0, "targetLevel": lot, "pushTo": "hub"}],
            "flows": [{"resource": ore, "ratePerHour": rnd(ph(ore_day) * k), "perThousand": True, "source": "industry", "label": "Mine crews"}],
            "capacities": [{"resource": ore, "capacity": whole(mining_bud["stocks"]["ore"]["capacity"], -2), "initialStock": 0}],
            "mineralSources": [{"resource": ore, "remainingUnits": 50_000_000, "maxUnitsPerMission": lot}],
        }

    # Farm villages: grow the raw food, push it to their factory; make the civilian products.
    kf = 1000 / farm_bud["pop"]
    f_pol, f_flows, f_caps = food_profile(farm_bud["units"], farm_bud["pop"])
    farm_water_day = farm_bud["water_day"] + farm_use.get("water", 0)
    civil_push = lambda c: max(1, min(round(lot / consumables[c]["unit_kg"]), whole(planet_units[c] / max(1, n_farm) * 5)))
    farm = {
        "processes": [process(c, max(1, n_farm), "make") for c in made_at("farm")],
        "policies": [
            pull("water", farm_water_day),
            *f_pol,
            *[{"resource": r, "minThreshold": 0, "targetLevel": lot, "pushTo": "hub"} for r in raw],
            *[{"resource": c, "minThreshold": 0, "targetLevel": civil_push(c), "pushTo": "hub"} for c in made_at("farm")],
        ],
        "flows": [
            *water_flows(farm_bud, kf), *f_flows,
            *[{"resource": r, "ratePerHour": rnd(ph(v) * kf), "perThousand": True, "source": "industry", "label": "Greenhouses"} for r, v in sorted(farm_bud["production"].items())],
        ],
        "capacities": [
            capacity("water", farm_water_day, d=-2), *f_caps,
            *[capacity(r, v, "ore_capacity", initial=False, d=-2) for r, v in sorted(farm_bud["production"].items())],
            *[capacity(c, planet_units[c] / max(1, n_farm), "capacity", initial=False) for c in made_at("farm")],
        ],
    }

    # Factories: refine ore, push metals to the spaceport, stock water and food for the villages
    # they serve. Hubs of farms also make the consumables (overrides below).
    served_farms = n_farm / n["factory"]
    local = Counter({c: u * ref["factory"] for c, u in worker.items()})
    for c, u in mining_bud["units"].items():
        local[c] += u * served
    for c, u in farm_bud["units"].items():
        local[c] += u * served_farms
    own_water = cfg["default_water_per_thousand_h"] * HOURS / 1000
    factory_water_day = own_water * ref["factory"] + served * mining_bud["water_day"] + served_farms * farm_water_day
    factory = {
        "processes": [
            {"code": f"refine_{ore_code(r)}", "ratePerHour": whole(factory_ore_h * cfg["refining_capacity_factor"], -1), "label": f"Refinery ({geology['rocks'][r]['label']})",
             "inputs": [{"resource": ore_code(r), "quantity": 1}],
             "outputs": [{"resource": metals[e]["code"], "quantity": round(v, 6)} for e, v in yields[r].items()]}
            for r in rocks
        ],
        "policies": [
            pull("water", factory_water_day), *[pull(c, u) for c, u in sorted(local.items())],
            # Ship from one truckload, or from 5 days of output for the rare metals.
            *[{"resource": metals[e]["code"], "minThreshold": 0, "targetLevel": max(1, min(lot, whole(metal_day[e] / n["factory"] * 5))), "pushTo": "spaceport"}
              for e in elements],
        ],
        "capacities": [
            capacity("water", factory_water_day, d=-3), *[capacity(c, u) for c, u in sorted(local.items())],
            *[{"resource": ore_code(r), "capacity": whole(factory_ore_h * HOURS * sd["ore_capacity"], -3), "initialStock": 0} for r in rocks],
            *[{"resource": metals[e]["code"], "capacity": max(whole(metal_day[e] / n["factory"] * sd["ore_capacity"] * 2, -2), lot * 2), "initialStock": 0} for e in elements],
        ],
    }
    for hub in hubs:
        overrides[hub] = {
            "processes": [process(c, n_hub, "make") for c in made_at("factory")],
            "policies": [pull("water", factory_water_day + hub_use.get("water", 0)),
                         *[pull(i, v) for i, v in sorted(hub_use.items()) if i != "water" and food["ingredients"][i]["origin"] in BROUGHT_IN]],
            "capacities": [
                capacity("water", factory_water_day + hub_use.get("water", 0), d=-3),
                *[capacity(i, v, "ore_capacity" if food["ingredients"][i]["origin"] == "farm" else "capacity",
                           initial=food["ingredients"][i]["origin"] in BROUGHT_IN, d=-2) for i, v in sorted(hub_use.items()) if i != "water"],
                *[capacity(c, max(local[c], planet_units[c] / n_hub), "capacity", initial=False) for c in made_at("factory")],
            ],
        }

    # Railway cities: transit, water and additives for their factories.
    per_railway = n["factory"] / n["railway"]
    railway_water_day = own_water * ref["railway"] + per_railway * factory_water_day
    additives = {i: v for i, v in ingredients.items() if i != "water" and food["ingredients"][i]["origin"] in BROUGHT_IN}
    railway = {
        "policies": [pull("water", railway_water_day), *[pull(i, v / n["railway"]) for i, v in additives.items()]],
        "capacities": [
            capacity("water", railway_water_day, d=-3), *[capacity(i, v / n["railway"], d=-2) for i, v in additives.items()],
            *[{"resource": metals[e]["code"], "capacity": None, "initialStock": 0} for e in elements],
        ],
    }

    # Spaceports: import water and additives, export metals.
    planet_water_day = (n["mining"] * mining_bud["water_day"] + n_farm * farm_water_day
                        + own_water * sum(ref[r] * n[r] for r in ("factory", "railway", "spaceport")) + ingredients.get("water", 0))
    trade = cfg["trade_capacity_factor"]
    sp_water_day = planet_water_day / n["spaceport"]
    moon = cfg["moon"]
    moon_label = f"From moon 2 ({moon['name']})"
    salts_kg = sum(v for i, v in additives.items() if food["ingredients"][i]["origin"] == "moon")
    regolith_day = planet_water_day / moon["water_fraction"]
    salts_available = regolith_day * sum(moon["regolith_pct"][k] for k in moon["salts_from"]) / 100
    spaceport = {
        "flows": [
            {"resource": "water", "ratePerHour": rnd(ph(sp_water_day) * trade), "source": "import", "label": moon_label},
            *[{"resource": i, "ratePerHour": rnd(ph(v / n["spaceport"]) * trade, 2), "source": "import",
               "label": moon_label if food["ingredients"][i]["origin"] == "moon" else "Off-system imports"} for i, v in additives.items()],
            *[{"resource": metals[e]["code"], "ratePerHour": rnd(-ph(metal_day[e] / n["spaceport"]) * trade, 3), "source": "export", "label": "Off-planet exports"} for e in elements],
        ],
        "capacities": [
            capacity("water", sp_water_day, "target_level", d=-3),
            *[capacity(i, v / n["spaceport"], "target_level", d=-2) for i, v in additives.items()],
            *[{"resource": metals[e]["code"], "capacity": None, "initialStock": 0} for e in elements],
        ],
    }

    villages = {
        "defaults": defaults,
        "byType": {"mining": mining, "farm": farm, "factory": factory, "railway": railway, "spaceport": spaceport},
        "typeByNamePrefix": cfg["poi_types"],
        "hubs": {"mining": "factory", "farm": "factory"},
        "overrides": overrides,
    }
    routes = {
        "links": [
            {"a": "mining", "b": "factory", "nearestNeighbours": 1},
            {"a": "farm", "b": "factory", "nearestNeighbours": 1},
            {"a": "factory", "b": "factory", "nearestNeighbours": 2, "maxDistanceM": 60000},
            {"a": "factory", "b": "railway", "nearestNeighbours": 1},
            {"a": "railway", "b": "railway", "mode": "rail", "nearestNeighbours": 2, "maxDistanceM": 150000},
            {"a": "railway", "b": "spaceport", "mode": "rail", "nearestNeighbours": 1},
        ],
    }
    summary = {
        "rocks": {r: {"villages": sum(1 for v in mines.values() if v["rock_type"] == r), "yields": yields[r], "ore_price": ore_price[r]} for r in rocks},
        "metal_day": metal_day, "planet_ore_day": n["mining"] * ore_day, "planet_water_day": planet_water_day, "served": served,
        "factory_ore_h": factory_ore_h, "factory_water_day": factory_water_day, "railway_water_day": railway_water_day, "mines": mines,
        "planet_units": planet_units, "ingredients": ingredients, "farms_needed": needed, "need_by": need_by, "n_farm": n_farm, "hubs": hubs,
        "local_share": local_kg / food_kg if food_kg else 0, "by_origin": by_origin, "food_ingredients_kg": food_kg,
        "regolith_day": regolith_day, "salts_kg": salts_kg, "salts_available": salts_available, "recipes": recipes, "counts": counts,
        "per_village": per_village, "farms": farms,
    }
    world = {"resources": resources, "villages": villages, "routes": routes,
             "federation": {"managedCategories": ["ore", "food", "water", "material"], "railSpeedKmh": cfg["rail_speed_kmh"]}}
    return world, summary


def fmt_place(p):
    pos = p["position"]
    return (f'{{ uuid: "{p["uuid"]}", planet: "{p["planet"]}", '
            f'position: {{x: {pos["x"]}, y: {pos["y"]}, z: {pos["z"]}}} }}')



# ── Writers ──────────────────────────────────────────────────────────────────

fmt = lambda x: f"{x:,.0f}".replace(",", " ")


def write_roster(name, npcs, args, path):
    lines = [
        f"# GENERATED by scripts/village_gen.py from config/village/{name}.yaml — do not edit.",
        f"# {name} archetype: one NPC per inhabitant (ADR services/0002, 0003, 0010).",
        f"# Village frame: {args.village_uuid}" + (" (PLACEHOLDER: pass --village-uuid)" if args.village_uuid == PLACEHOLDER_VILLAGE else ""),
        "# Positions are placeholders until Godot exports markers for the village buildings.",
        f"# Use with ROSTER_PATH={path.relative_to(ROOT)} (and NPC_ACTIVE_COUNT to cap live NPCs).",
        "",
        "npcs:",
    ]
    role = None
    for n in npcs:
        if n["role"] is not role:
            role = n["role"]
            lines += ["", f"  # --- {role['label']} ({role['id']}, {role['count']}) ---"]
        s = n["spec"]
        lines.append(f'  - name: {json.dumps(s["name"], ensure_ascii=False)}')
        lines.append(f'    uuid: "{s["uuid"]}"')
        lines.append(f'    schedule: {s["schedule"]}')
        lines.append("    action_profile: [" + ", ".join(s["action_profile"]) + "]")
        for key in ("place_where_work", "home", "meal_place", "mandatory_place"):
            if key in s:
                lines.append(f"    {key}: {fmt_place(s[key])}")
        for key in ("depot_store_reception_to_temp_area", "depot_store_item"):
            if key in s:
                lines.append(f"    {key}: " + json.dumps(s[key]))
    path.write_text("\n".join(lines) + "\n")


def write_budget(name, arch, npcs, bud, reserved, free, food, path):
    b = arch["budget"]
    out = [f"# {name} budget (generated)", "",
           f"Generated by `scripts/village_gen.py` from `config/village/{name}.yaml`, `config/schedules.yaml` and",
           "`config/food/consumables.yaml` — do not edit. Decisions: ADR `services/docs/adr/0002`–`0010`.", ""]
    out += ["## Population", "", "| Role | Count | Rota | Works at | `a` at | Diet |", "|---|---:|---|---|---|---|"]
    for r in arch["roles"]:
        rota = ", ".join(r["patterns"]) if r.get("patterns") else r["rota"]
        out.append(f"| {r['label']} | {r['count']} | {rota} | {r.get('work') or '—'} | {r['mandatory']} | {r.get('diet', 'worker')} |")
    active = sum(r["count"] for r in arch["roles"] if r.get("active", True))
    out += [f"| **Total** | **{len(npcs)}** ({active} working) | | | | |", ""]

    homes = Counter(n["home"][0] for n in npcs)
    out += ["## Housing (one dwelling per NPC)", "", "| Building | Units | NPCs | Players | Free |", "|---|---:|---:|---:|---:|"]
    pb = arch["housing"]["player_units"]["building"]
    for bname, d in arch["buildings"].items():
        if d.get("units"):
            players = len(reserved) if bname == pb else 0
            out.append(f"| {d['name']} | {d['units']} | {homes[bname]} | {players} | {len(free.get(bname, []))} |")
    out.append("")

    out += ["## On duty per role (`T`/`t`), by planetary hour", "",
            "Min / mean / max over the 5-day cycle, then the 25 hours of day 1 (identical every day for crew rotas).", "",
            "| Role | Min | Mean | Max | Day 1, h0 → h24 |", "|---|---:|---:|---:|---|"]
    for r in arch["roles"]:
        h = bud["hourly"][r["id"]]
        out.append(f"| {r['id']} | {min(h)} | {sum(h) / len(h):.1f} | {max(h)} | `{' '.join(str(x) for x in h[:HOURS])}` |")
    out.append("")

    out += ["## Building load (people present), peak over the cycle", "", "| Place | Peak | At (day, hour) | Mean |", "|---|---:|---|---:|"]
    for place, h in sorted(bud["occupancy"].items(), key=lambda kv: -max(kv[1])):
        peak = max(h)
        at = h.index(peak)
        out.append(f"| {arch['buildings'].get(place, {}).get('name', place)} | {peak} | day {at // HOURS + 1}, h{at % HOURS} | {sum(h) / len(h):.1f} |")
    out.append("")

    w = b["water"]
    out += ["## Resources per planetary day (25 h)", "", "| Item | Per day | Per hour | Basis |", "|---|---:|---:|---|"]
    out.append(f"| Water, inhabitants (l) | {fmt(bud['water_pop_day'])} | {bud['water_pop_day'] / HOURS:.0f} | {w['l_per_person_day']} l per inhabitant |")
    out.append(f"| Water, industry (l) | {fmt(bud['water_ind_day'])} | {bud['water_ind_day'] / HOURS:.0f} | showers, pre-treatment, irrigation |")
    out.append(f"| **Water, total (l)** | **{fmt(bud['water_day'])}** | **{bud['water_day'] / HOURS:.0f}** | from the hub |")
    out.append(f"| **Food (kg)** | **{fmt(bud['food_kg_day'])}** | | the consumables below |")
    if bud["conc_day"]:
        o = b["ore"]
        out.append(f"| Miner hours (`T`) | {bud['miner_hours_day']:.0f} | {bud['miner_hours_day'] / HOURS:.1f} | |")
        out.append(f"| Raw ore (kg) | {fmt(bud['raw_day'])} | {bud['raw_day'] / HOURS:.0f} | {o['raw_kg_per_worker_hour']} kg per miner hour |")
        out.append(f"| Ore to the factory (kg) | {fmt(bud['conc_day'])} | {bud['conc_day'] / HOURS:.0f} | yield {o['pretreatment_yield']:.0%} at pre-treatment |")
    for r, v in sorted(bud["production"].items()):
        out.append(f"| {food['ingredients'][r]['name']} to the factory (kg) | {fmt(v)} | {v / HOURS:.0f} | greenhouses (`produces`) |")
    out += ["", "### Consumables eaten", "", "| Consumable | Units per day | kg per day |", "|---|---:|---:|"]
    for c, u in sorted(bud["units"].items()):
        out.append(f"| {food['consumables'][c]['name']} | {u:.1f} | {u * food['consumables'][c]['unit_kg']:.1f} |")
    out.append("")

    t = b["transport"]
    util = bud["hours_needed"] / bud["hours_available"] if bud["hours_available"] else 0
    out += ["## Transport to the factory", "", "| Item | Per day |", "|---|---:|",
            f"| Loads out ({t['truck_payload_kg']} kg) | {bud['out_loads']:.1f} |",
            f"| Loads back (water + food) | {bud['import_loads']:.1f} |",
            f"| Round trips | {bud['trips']:.1f} |",
            f"| Driver hours needed ({t['road_round_trip_h'] + t['handling_h']} h per trip) | {bud['hours_needed']:.0f} |",
            f"| Driver hours on duty | {bud['hours_available']:.0f} |",
            f"| NPC driver utilisation if players carry nothing | {util:.0%} |", ""]

    sd = b["stock_days"]
    out += ["## Village stock policy", "",
            f"Minimum {sd['min_threshold']} days, target {sd['target_level']} days, capacity {sd['capacity']} days of consumption; "
            f"outbound goods {sd['ore_capacity']} days of production.", "",
            "| Resource | Minimum | Target | Capacity |", "|---|---:|---:|---:|"]
    for r, s in bud["stocks"].items():
        out.append(f"| {r} | {fmt(s['min_threshold']) if 'min_threshold' in s else '—'} | {fmt(s['target_level']) if 'target_level' in s else '—'} | {fmt(s['capacity'])} |")
    path.write_text("\n".join(out) + "\n")


def write_chain(cfg, s, food, path):
    metals, n = cfg["metals"], cfg["villages"]
    out = ["# Supply chain of the planet (generated)", "",
           "Generated by `scripts/village_gen.py` from `config/village/supply_chain.yaml`, the archetypes, the food catalogue",
           "and the geology — do not edit. Decisions: ADR `services/docs/adr/0007`–`0010`.", ""]
    out += ["## Villages", "", "| Role | Villages | Population each |", "|---|---:|---:|"]
    pops = {"mining": s["per_village"] and None}
    for role, c in s["counts"].items():
        pop = {"mining": 250, "farm": 200}.get(role, cfg["reference_population"].get(role))
        out.append(f"| {role} | {c} | {pop} |")
    out.append("")

    out += ["## Ore by rock (outcrop geology, ADR services/0008)", "",
            f"Each mining village ships its ore to its factory, which refines it (yield {cfg['refining_yield']:.0%}). Per kg of ore:", "",
            "| Rock | Villages | " + " | ".join(f"{e} (kg)" for e in metals) + " | Ore price |", "|---|---:|" + "---:|" * len(metals) + "---:|"]
    for r, d in s["rocks"].items():
        out.append(f"| {r} | {d['villages']} | " + " | ".join(f"{d['yields'][e]:.6f}" if e in d["yields"] else "—" for e in metals) + f" | {d['ore_price']} |")
    out += ["", "| Village | Rock |", "|---|---|", *[f"| {k} | {v['rock_type']} |" for k, v in sorted(s["mines"].items())], ""]

    c = food["consumables"]
    out += ["## Food (ADR services/0009)", "",
            "| Consumable | Made by | Units per day (planet) | kg per day | Batch | Inputs per batch (kg) |", "|---|---|---:|---:|---:|---|"]
    for code, u in sorted(s["planet_units"].items(), key=lambda kv: -kv[1] * c[kv[0]]["unit_kg"]):
        units, inputs = s["recipes"][code]
        out.append(f"| {c[code]['name']} | {c[code]['made_by']} | {fmt(u)} | {fmt(u * c[code]['unit_kg'])} | {units} {c[code]['unit']}s | "
                   + ", ".join(f"{i} {q}" for i, q in inputs.items()) + " |")
    out += ["", "| Ingredient | Origin | kg per day (planet) | Farms needed |", "|---|---|---:|---:|"]
    for i, v in sorted(s["ingredients"].items(), key=lambda kv: -kv[1]):
        out.append(f"| {food['ingredients'][i]['name']} | {food['ingredients'][i]['origin']} | {fmt(v)} | {s['need_by'].get(i, '—')} |")
    share = lambda o: s["by_origin"][o] / s["food_ingredients_kg"] if s["food_ingredients_kg"] else 0
    out += ["", f"**Grown on Sandbox: {s['local_share']:.0%} of the food mass** (target {cfg['local_food_target']:.0%}); "
            f"mineral salts from moon 2: {share('moon'):.0%}; additives from outside the system: {share('import'):.0%}.", "",
            f"**Farm villages: {s['farms_needed']} needed, {s['n_farm']} placed** "
            f"(config/world/{cfg['planet']}/farm_villages.json); {len(s['hubs'])} factories make the consumables: {', '.join(s['hubs']) or '—'}.", ""]
    if s["farms"]:
        out += ["| Farm | Lat | Lon | Altitude | Road | Hub | Distance |", "|---|---:|---:|---:|---:|---|---:|"]
        out += [f"| {f['name']} | {f['lat']:.4f} | {f['lon']:.4f} | {f['elevation_m']} m | {f['road_m']} m | {f['hub']} | {f['hub_m'] / 1000:.1f} km |" for f in s["farms"]]
        out.append("")

    out += ["## Planet flows (per day)", "", "| Flow | Per day |", "|---|---:|", f"| Ore mined and refined | {fmt(s['planet_ore_day'])} kg |"]
    out += [f"| {metals[e]['name']} exported | {fmt(v)} kg |" for e, v in s["metal_day"].items()]
    moon = cfg["moon"]
    out += [f"| Water from moon 2 ({moon['name']}) | {fmt(s['planet_water_day'])} l |",
            f"| Regolith processed on moon 2 ({moon['water_fraction']:.0%} water) | {fmt(s['regolith_day'])} kg |",
            f"| Mineral salts from moon 2 (needed / by-product available) | {fmt(s['salts_kg'])} / {fmt(s['salts_available'])} kg |",
            f"| Food eaten | {fmt(sum(u * c[k]['unit_kg'] for k, u in s['planet_units'].items()))} kg |",
            *[f"| {food['ingredients'][i]['name']} from outside the system | {fmt(v)} kg |" for i, v in s["ingredients"].items() if food["ingredients"][i]["origin"] == "import"],
            f"| Water through one factory (own + its villages) | {fmt(s['factory_water_day'])} l |",
            f"| Ore into one factory | {fmt(s['factory_ore_h'] * HOURS)} kg |", ""]
    path.write_text("\n".join(out))


# ── Main ─────────────────────────────────────────────────────────────────────

def plan(args=None):
    """Builds every archetype and the chain. Returns everything the writers need."""
    cfg, archetypes, sched, sample, food, geology, farms = load()
    check_food(food, archetypes)
    village_uuid = args.village_uuid if args else PLACEHOLDER_VILLAGE
    planet = args.planet if args else cfg["planet"]
    villages = {}
    for name, arch in archetypes.items():
        npcs, reserved, free = build(arch, sched, sample, village_uuid, planet)
        check(arch, sched, npcs, reserved)
        villages[name] = (arch, npcs, reserved, free, budget(arch, sched, npcs, food))
    buds = {name: v[4] for name, v in villages.items()}
    world, summary = chain(cfg, buds, geology, farms, food)
    return cfg, food, villages, world, summary


def farms_needed():
    """Farm villages the food budget asks for (used by scripts/village_farms.py)."""
    return plan()[4]["farms_needed"]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--village-uuid", default=PLACEHOLDER_VILLAGE, help="frame of every place (the village object)")
    ap.add_argument("--planet", default="tarsis_3", help="planet key of every place")
    ap.add_argument("--check", action="store_true", help="validate only, write nothing")
    ap.add_argument("--write-seed", action="store_true", help=f"also apply the values to {SEED.relative_to(ROOT.parent)}")
    args = ap.parse_args()
    try:
        cfg, food, villages, world, summary = plan(args)
    except CheckError as e:
        print(f"check failed:\n{e}", file=sys.stderr)
        return 1
    for name, (_, npcs, _, _, bud) in villages.items():
        print(f"ok: {name}: {len(npcs)} inhabitants, {len(set(n['home'] for n in npcs))} dwellings, water {bud['water_day']:.0f} l/day, "
              f"food {bud['food_kg_day']:.0f} kg/day, out {bud['conc_day'] + sum(bud['production'].values()):.0f} kg/day")
    print(f"ok: planet: {summary['farms_needed']} farms needed, {summary['n_farm']} placed, local food {summary['local_share']:.0%}")
    if summary["n_farm"] < summary["farms_needed"]:
        print(f"warning: only {summary['n_farm']} farm villages for {summary['farms_needed']} needed: rerun scripts/village_farms.py", file=sys.stderr)
    if args.check:
        return 0
    written = []
    for name, (arch, npcs, reserved, free, bud) in villages.items():
        roster, report = ROOT / f"config/npcs.{name}.yaml", VILLAGES / f"budget.{name}.generated.md"
        write_roster(name, npcs, args, roster)
        write_budget(name, arch, npcs, bud, reserved, free, food, report)
        written += [roster, report]
    write_chain(cfg, summary, food, OUT_CHAIN)
    note = {"_note": "GENERATED by npc/scripts/village_gen.py (ADR services/0004, 0007-0010): world-file sections for universesimulator."}
    OUT_USIM.write_text(json.dumps({**note, **world}, indent=2, ensure_ascii=False) + "\n")
    written += [OUT_CHAIN, OUT_USIM]
    if args.write_seed:
        seed = json.loads(SEED.read_text())
        seed["federation"].update(world["federation"])
        seed["resources"] = world["resources"]
        # Defaults keep the water of every village; the food comes from the generator.
        codes = {r["code"] for r in world["resources"]}
        kept = {k: [x for x in v if x["resource"] == "water" and x["resource"] in codes] for k, v in seed["villages"]["defaults"].items()}
        defaults = {k: kept.get(k, []) + world["villages"]["defaults"].get(k, []) for k in ("policies", "flows", "capacities")}
        seed["villages"] = {**world["villages"], "defaults": defaults}
        seed["villages"] = {k: seed["villages"][k] for k in ("defaults", "byType", "typeByNamePrefix", "hubs", "overrides")}
        seed["routes"] = world["routes"]
        SEED.write_text(dump_seed(seed))
        written.append(SEED)
    for p in written:
        print(f"wrote {p.relative_to(ROOT.parent)}")
    return 0


def dump_seed(value, indent=0):
    """JSON in the seed's style: containers on several lines, an object or list of scalars on
    one line."""
    pad, inner = "  " * indent, "  " * (indent + 1)
    flat = lambda v: not isinstance(v, (dict, list)) or (isinstance(v, list) and all(not isinstance(x, (dict, list)) for x in v))
    one = lambda v: json.dumps(v, ensure_ascii=False, separators=(", ", ": ")).replace("{", "{ ").replace("}", " }") if isinstance(v, dict) else json.dumps(v, ensure_ascii=False, separators=(", ", ": "))
    if isinstance(value, dict):
        if value and all(flat(v) for v in value.values()) and len(one(value)) <= 240 and indent > 1:
            return one(value)
        items = [f'{inner}{json.dumps(k, ensure_ascii=False)}: {dump_seed(v, indent + 1)}' for k, v in value.items()]
        return "{\n" + ",\n".join(items) + f"\n{pad}}}" + ("\n" if indent == 0 else "")
    if isinstance(value, list):
        if all(flat(v) for v in value):
            return one(value)
        return "[\n" + ",\n".join(f"{inner}{dump_seed(v, indent + 1)}" for v in value) + f"\n{pad}]"
    return json.dumps(value, ensure_ascii=False)


if __name__ == "__main__":
    sys.exit(main())
