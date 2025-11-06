import argparse
import json
import os
import sys
import psycopg2
from psycopg2.extras import Json, RealDictCursor

#!/usr/bin/env python3
"""
Import a planetary system from JSON into PostgreSQL.

Usage:
    python import_system_from_json.py path/to/system.json

Postgres connection is read from environment variables:
    PGHOST, PGPORT, PGDATABASE, PGUSER, PGPASSWORD

This script expects the target DB schema (planet.sql) to provide tables
with at least the following columns (names are conventional; adjust SQL if your schema differs):

- systems(name text, attributes jsonb)
- stars(system_id int references systems, name text, attributes jsonb)
- planets(system_id int references systems, star_id int references stars, name text, attributes jsonb, mass numeric, radius numeric)

The JSON may be shaped like:
{
  "system": { "name": "Alpha", "attributes": {...} },
  "stars": [ { "name": "Alpha A", "attributes": {...} } ],
  "planets": [ { "name": "Alpha b", "mass": 1.0, "radius": 0.9, "star_index": 0, "attributes": {...} } ]
}

Fields are optional; planets may reference their parent star either with "star_index" (index into stars array)
or "star_name". If nothing references a star, star_id will be NULL.
"""

def get_conn():
    params = {
        "host": os.environ.get("PGHOST", "127.0.0.1"),
        "port": os.environ.get("PGPORT", 5432),
        "dbname": os.environ.get("PGDATABASE", "ds_planets"),
        "user": os.environ.get("PGUSER", "postgres"),
        "password": os.environ.get("PGPASSWORD", "localpass"),
    }
    return psycopg2.connect(**params)

def insert_system(cur, name):
    cur.execute(
        "INSERT INTO systems (name, internal_name) VALUES (%s, %s) RETURNING id",
        (name,name)
    )
    # Using RealDictCursor returns a mapping (dict-like). Access the returned
    # id by key instead of positional index. Also ensure the params argument
    # is a tuple (use (name,) for single-param) to avoid psycopg2 string-format
    # conversion errors: "not all arguments converted during string formatting".
    row = cur.fetchone()
    return row["id"] if row else None

def insert_star(cur, system_id, star_obj, name):
    mass_sun = star_obj.get("mass_Sun", 1.0) * 1.989e30
    attrs = star_obj.get("attributes") or star_obj.get("attrs") or {"mass_kg": 0.0}
    star_name = star_obj.get("name") or name
    cur.execute(
        "INSERT INTO stars (system_id, name, internal_name, mass_kg) VALUES (%s, %s, %s, %s) RETURNING id",
        (system_id, star_name, star_name, attrs.get("mass_kg", 0.0)),
    )
    row = cur.fetchone()
    return (row["id"], mass_sun) if row else None

def insert_planet(cur, system_id, name, planet_obj, mass_sun):
    if planet_obj.get("periapsis_AU", None) is None:
        return None

    # calculate gravity influence radius of the planet (SOI)
    planet_mass_kg = planet_obj.get("mass_Me", 0) * 5.972e24  # Earth mass in kg
    semi_major_axis_m = planet_obj.get("semi_major_AU", 0) * 1.496e11  # AU to meters
    if semi_major_axis_m > 0 and mass_sun > 0.0:
        radius_gravity_influence_m = semi_major_axis_m * (planet_mass_kg / mass_sun) ** (2/5)
    else:
        radius_gravity_influence_m = 0
    radius_gravity_influence_km = radius_gravity_influence_m / 1000  # convert to km
    # calculation is ended here

    cur.execute(
        """
    INSERT INTO planets (system_id, name, internal_name, mass_kg, periapsis_AU, apoapsis_AU, inc_deg, node_deg, arg_peri_deg, mean_anomaly_deg, radius_km, radius_gravity_influence_km)
    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
        RETURNING id
        """,
        (
            system_id,
            name,
            name,
            planet_obj.get("mass_Me"),
            planet_obj.get("periapsis_AU"),
            planet_obj.get("apoapsis_AU"),
            planet_obj.get("inclination_deg"),
            planet_obj.get("ascending_node_deg"),
            planet_obj.get("arg_peri_deg"),
            planet_obj.get("M0_deg"),
            planet_obj.get("radius_km"),
            radius_gravity_influence_km,
        ),
    )
    row = cur.fetchone()
    return row["id"] if row else None

def main():
    p = argparse.ArgumentParser(description="Import a system JSON into PostgreSQL")
    p.add_argument("json_file", help="Path to system JSON file")
    p.add_argument("sys_name", help="Name of the system")
    args = p.parse_args()

    try:
        with open(args.json_file, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception as e:
        print(f"Failed to read JSON: {e}", file=sys.stderr)
        sys.exit(2)

    # Normalize structure
    if isinstance(data, dict) and "structured" in data:
        structured = data.get("structured")
        # If 'structured' is a JSON string, try to parse it
        if isinstance(structured, str):
            try:
                structured = json.loads(structured)
            except Exception:
                pass
        if isinstance(structured, dict):
            data = structured
    system_obj = data.get("system") or data
    stars = data.get("stars") or (system_obj.get("stars") if isinstance(system_obj, dict) else None) or []
    planets = data.get("planets") or (system_obj.get("planets") if isinstance(system_obj, dict) else None) or []

    conn = get_conn()
    try:
        with conn:
            with conn.cursor(cursor_factory=RealDictCursor) as cur:
                system_id = insert_system(cur, args.sys_name)
                # insert stars and keep map of index/name -> id for planet references
                star_ids = []
                name_to_star_id = {}
                for i, s in enumerate(stars):
                    sid, mass_sun = insert_star(cur, system_id, s, args.sys_name)
                    star_ids.append(sid)
                    if s.get("name"):
                        name_to_star_id[s["name"]] = sid

                planet_ids = []
                for p_obj in planets:
                    # determine star_id for this planet
                    star_id = None
                    if p_obj.get("star_index") is not None:
                        idx = p_obj.get("star_index")
                        try:
                            star_id = star_ids[int(idx)]
                        except Exception:
                            star_id = None
                    elif p_obj.get("star_name"):
                        star_id = name_to_star_id.get(p_obj.get("star_name"))
                    elif p_obj.get("star_id"):
                        # allow user-provided existing star id
                        star_id = p_obj.get("star_id")
                    # insert planet
                    name = f"{args.sys_name}_{len(planet_ids) + 1}"
                    pid = insert_planet(cur, system_id, name, p_obj, mass_sun)
                    planet_ids.append(pid)

                # successful commit happens automatically on exiting context
                print(json.dumps({
                    "system_id": system_id,
                    "star_ids": star_ids,
                    "planet_ids": planet_ids
                }))
    except Exception as e:
        # psycopg will rollback on exception when using with conn
        print(f"Import failed: {e}", file=sys.stderr)
        sys.exit(3)
    finally:
        conn.close()

if __name__ == "__main__":
    main()