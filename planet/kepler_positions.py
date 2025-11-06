"""
kepler_positions.py

Calculate 3D planet positions (x, y, z) around a star at the origin (0,0,0)
using Keplerian orbital elements and write a JSON file containing samples
every 16.66667 ms (60 Hz equivalent time step of 1/60 s).

Top-level parameters (edit these values to change the simulated orbit):

  star_mass_kg      : float  - Mass of the central star in kilograms (default: Sun mass ~1.98847e30)
  planet_mass_kg    : float  - Mass of the planet in kilograms (default: Earth mass ~5.972e24)
  periapsis_AU      : float  - Periapsis distance in astronomical units (AU). If both periapsis and apoapsis are 0,
                                the semi-major axis is inferred from the initial position.
  apoapsis_AU       : float  - Apoapsis distance in AU.
  inc_deg           : float  - Inclination of the orbit in degrees (i)
  node_deg          : float  - Longitude of the ascending node in degrees (Ω)
  arg_peri_deg      : float  - Argument of periapsis in degrees (ω)
  mean_anomaly_deg  : float  - Mean anomaly at epoch in degrees (M0)
  time_scale        : float  - Multiplier mapping simulation seconds to orbital seconds (1.0 = real seconds)

Output JSON format (array of samples):
  [{"time_s": float, "x": float, "y": float, "z": float}, ...]

Notes:
  - Positions are in meters relative to the star at (0,0,0).
  - The sampling interval is 0.01666667 seconds (16.66667 ms).
  - Uses a Newton-Raphson solver for Kepler's equation; it's stable for e < 0.999.
  - The script is self-contained and intentionally small; it reimplements the minimal
    vector and orbital helpers required.
"""

from __future__ import annotations
import math
import json
import time
from typing import List, Dict, Optional
import argparse
import re
import os
import psycopg2
from psycopg2.extras import RealDictCursor

try:
    import numpy as np
    NUMPY_AVAILABLE = True
except ImportError:
    NUMPY_AVAILABLE = False
    print("Warning: NumPy not available, performance will be reduced")

try:
    import pandas as pd
    PANDAS_AVAILABLE = True
except ImportError:
    PANDAS_AVAILABLE = False
    print("Warning: Pandas not available, will use slower insertion method")

try:
    import duckdb
    DUCKDB_AVAILABLE = True
except ImportError:
    DUCKDB_AVAILABLE = False
    print("Warning: DuckDB not available, will only generate JSON")


class Vector3:
    def __init__(self, x: float = 0.0, y: float = 0.0, z: float = 0.0):
        self.x = float(x)
        self.y = float(y)
        self.z = float(z)

    def __add__(self, o: 'Vector3') -> 'Vector3':
        return Vector3(self.x + o.x, self.y + o.y, self.z + o.z)

    def __sub__(self, o: 'Vector3') -> 'Vector3':
        return Vector3(self.x - o.x, self.y - o.y, self.z - o.z)

    def __mul__(self, scalar: float) -> 'Vector3':
        return Vector3(self.x * scalar, self.y * scalar, self.z * scalar)

    def distance_to(self, o: 'Vector3') -> float:
        return math.sqrt((self.x - o.x)**2 + (self.y - o.y)**2 + (self.z - o.z)**2)


class Basis:
    """Simple rotation basis built from Euler rotations: Rz(node) * Rx(inc) * Rz(arg_peri)"""
    def __init__(self, x: Vector3 = None, y: Vector3 = None, z: Vector3 = None):
        self.x = x or Vector3(1, 0, 0)
        self.y = y or Vector3(0, 1, 0)
        self.z = z or Vector3(0, 0, 1)

    def __mul__(self, v: Vector3) -> Vector3:
        return Vector3(self.x.x * v.x + self.x.y * v.y + self.x.z * v.z,
                       self.y.x * v.x + self.y.y * v.y + self.y.z * v.z,
                       self.z.x * v.x + self.z.y * v.y + self.z.z * v.z)

    def rotated(self, axis: Vector3, angle: float) -> 'Basis':
        # Rodrigues rotation applied to basis vectors
        cos_a = math.cos(angle)
        sin_a = math.sin(angle)
        axis_len = math.sqrt(axis.x * axis.x + axis.y * axis.y + axis.z * axis.z)
        if axis_len == 0:
            return self
        ux, uy, uz = axis.x / axis_len, axis.y / axis_len, axis.z / axis_len

        m00 = cos_a + ux * ux * (1 - cos_a)
        m01 = ux * uy * (1 - cos_a) - uz * sin_a
        m02 = ux * uz * (1 - cos_a) + uy * sin_a

        m10 = uy * ux * (1 - cos_a) + uz * sin_a
        m11 = cos_a + uy * uy * (1 - cos_a)
        m12 = uy * uz * (1 - cos_a) - ux * sin_a

        m20 = uz * ux * (1 - cos_a) - uy * sin_a
        m21 = uz * uy * (1 - cos_a) + ux * sin_a
        m22 = cos_a + uz * uz * (1 - cos_a)

        def rot_vec(v: Vector3) -> Vector3:
            return Vector3(m00 * v.x + m01 * v.y + m02 * v.z,
                           m10 * v.x + m11 * v.y + m12 * v.z,
                           m20 * v.x + m21 * v.y + m22 * v.z)

        return Basis(rot_vec(self.x), rot_vec(self.y), rot_vec(self.z))

    def inverse(self) -> 'Basis':
        # For orthonormal basis, inverse is transpose
        return Basis(Vector3(self.x.x, self.y.x, self.z.x),
                     Vector3(self.x.y, self.y.y, self.z.y),
                     Vector3(self.x.z, self.y.z, self.z.z))


class OrbitKepler:
    AU_M = 1.495978707e11
    G = 6.67430e-11
    TAU = 2.0 * math.pi

    def __init__(self,
                 star_mass_kg: float = 1.98847e30,
                 planet_mass_kg: float = 5.972e24,
                 periapsis_AU: float = 0.0,
                 apoapsis_AU: float = 0.0,
                 inc_deg: float = 0.0,
                 node_deg: float = 0.0,
                 arg_peri_deg: float = 0.0,
                 mean_anomaly_deg: float = 0.0,
                 initial_position_m: Vector3 = None):

        self.star_mass_kg = star_mass_kg
        self.planet_mass_kg = planet_mass_kg
        self.periapsis_AU = periapsis_AU
        self.apoapsis_AU = apoapsis_AU
        self.inc_deg = inc_deg
        self.node_deg = node_deg
        self.arg_peri_deg = arg_peri_deg
        self.mean_anomaly_deg = mean_anomaly_deg

        self._basis = self._orbit_basis(arg_peri_deg, inc_deg, node_deg)
        self._orbit_center = Vector3(0.0, 0.0, 0.0)

        # compute a and e
        if self.periapsis_AU > 0.0 or self.apoapsis_AU > 0.0:
            rp_AU = self.periapsis_AU if self.periapsis_AU > 0.0 else self.apoapsis_AU
            ra_AU = self.apoapsis_AU if self.apoapsis_AU > 0.0 else self.periapsis_AU
            if ra_AU < rp_AU:
                rp_AU, ra_AU = ra_AU, rp_AU
            self._a_m = 0.5 * (rp_AU + ra_AU) * self.AU_M
            self._e = max(0.0, (ra_AU - rp_AU) / max(ra_AU + rp_AU, 1e-12))
        else:
            # infer from initial position if provided
            if initial_position_m is None:
                raise ValueError("Provide initial_position_m if periapsis and apoapsis are zero")
            d = initial_position_m.distance_to(self._orbit_center)
            self._a_m = max(d, 1.0)
            self._e = 0.0

        mu = self.G * max(self.star_mass_kg + self.planet_mass_kg, 1.0)
        self._n = math.sqrt(mu / (self._a_m ** 3.0))
        self._M = math.radians(self.mean_anomaly_deg)

        # initial position in meters
        self._pos = initial_position_m or Vector3(self._a_m, 0.0, 0.0)

    def _orbit_basis(self, arg_peri_deg: float, inc_deg: float, node_deg: float) -> Basis:
        b = Basis()
        b = b.rotated(Vector3(0, 1, 0), math.radians(node_deg))
        b = b.rotated(Vector3(1, 0, 0), math.radians(inc_deg))
        b = b.rotated(Vector3(0, 1, 0), math.radians(arg_peri_deg))
        return b

    def _norm(self, a: float) -> float:
        x = math.fmod(a, self.TAU)
        if x < 0:
            x += self.TAU
        return x

    def _kepler_E_from_M(self, M: float, e: float) -> float:
        Mm = self._norm(M)
        E = math.pi if e > 0.8 else Mm
        for _ in range(16):
            f = E - e * math.sin(E) - Mm
            fp = 1.0 - e * math.cos(E)
            step = -f / max(fp, 1e-12)
            E += step
            if abs(step) < 1e-12:
                break
        return self._norm(E)

    def _kepler_position_plane(self, a_m: float, e: float, M: float) -> Vector3:
        E = self._kepler_E_from_M(M, e)
        xp = a_m * (math.cos(E) - e)
        zp = a_m * (math.sqrt(max(1.0 - e * e, 0.0)) * math.sin(E))
        return Vector3(xp, 0.0, zp)

    def advance(self, dt: float) -> Vector3:
        """Advance the mean anomaly by dt seconds and return current world-space position"""
        self._M = self._norm(self._M + self._n * dt)
        pos_plane = self._kepler_position_plane(self._a_m, self._e, self._M)
        world = self._orbit_center + (self._basis * pos_plane)
        self._pos = world
        return world


def generate_positions(output_filename: str,
                                  duration_seconds: float,
                                  sample_dt: float = 0.01666667,
                                  params: Dict = None,
                                  use_db: bool = True,
                                  use_json: bool = True,
                                  verbose: bool = True) -> Optional[List[Dict]]:
    """
    Generate positions.
    
    Args:
        output_filename: JSON output file path
        duration_seconds: Simulation duration
        sample_dt: Time step between samples
        params: Orbital parameters dictionary
        use_db: Whether to write to DuckDB
        use_json: Whether to write JSON file
        verbose: Whether to print progress
    
    Returns:
        List of sample dictionaries if use_json is True, otherwise None
    """
    start_time = time.time()
    params = params or {}

    # Pre-calculate constants (avoid repeated params.get() calls)
    object_type = params.get('object_type', 'planet')
    object_id = int(params.get('object_id', 0))
    star_mass_kg = params.get('star_mass_kg', 1.98847e30)
    planet_mass_kg = params.get('planet_mass_kg', 5.972e24)
    periapsis_AU = float(params.get('periapsis_AU', 0.98))
    apoapsis_AU = float(params.get('apoapsis_AU', 1.02))
    inc_deg = float(params.get('inc_deg', 0.0))
    node_deg = float(params.get('node_deg', 0.0))
    arg_peri_deg = float(params.get('arg_peri_deg', 0.0))
    mean_anomaly_deg = float(params.get('mean_anomaly_deg', 0.0))
    initial_position_m = params.get('initial_position_m', Vector3(1.0 * OrbitKepler.AU_M, 0.0, 0.0))

    # Initialize orbit
    orbit = OrbitKepler(star_mass_kg=star_mass_kg,
                        planet_mass_kg=planet_mass_kg,
                        periapsis_AU=periapsis_AU,
                        apoapsis_AU=apoapsis_AU,
                        inc_deg=inc_deg,
                        node_deg=node_deg,
                        arg_peri_deg=arg_peri_deg,
                        mean_anomaly_deg=mean_anomaly_deg,
                        initial_position_m=initial_position_m)

    steps = max(1, int(math.ceil(duration_seconds / sample_dt)))
    
    if verbose:
        print(f"Calculating {steps:,} positions over {duration_seconds}s...")

    # Use NumPy arrays for efficiency if available
    if NUMPY_AVAILABLE:
        times = np.arange(steps, dtype=np.float64) * sample_dt
        times = np.round(times, 3)
        positions = np.zeros((steps, 3), dtype=np.float64)
        
        # Calculate all positions
        calc_start = time.time()
        for i in range(steps):
            pos = orbit.advance(sample_dt if i > 0 else 0.0)
            positions[i] = [pos.x, pos.y, pos.z]
            
            if verbose and (i + 1) % 10000 == 0:
                print(f"  Calculated {i + 1:,}/{steps:,} positions...")
        
        calc_time = time.time() - calc_start
        if verbose:
            print(f"Position calculation completed in {calc_time:.2f}s ({steps/calc_time:.0f} positions/s)")
    else:
        # Fallback to Python lists
        times = [round(i * sample_dt, 3) for i in range(steps)]
        positions = []
        
        calc_start = time.time()
        for i in range(steps):
            pos = orbit.advance(sample_dt if i > 0 else 0.0)
            positions.append([pos.x, pos.y, pos.z])
            
            if verbose and (i + 1) % 10000 == 0:
                print(f"  Calculated {i + 1:,}/{steps:,} positions...")
        
        calc_time = time.time() - calc_start
        if verbose:
            print(f"Position calculation completed in {calc_time:.2f}s")

    # Write to DuckDB using pandas (much faster than executemany)
    if use_db and DUCKDB_AVAILABLE:
        db_start = time.time()
        
        if PANDAS_AVAILABLE and NUMPY_AVAILABLE:
            # Fast path: use pandas DataFrame
            df = pd.DataFrame({
                'type': object_type,
                'type_id': object_id,
                'time_s': times,
                'x': positions[:, 0],
                'y': positions[:, 1],
                'z': positions[:, 2]
            })
            
            con = duckdb.connect(database='database/my-db.duckdb', read_only=False)
            con.execute("""
                CREATE TABLE IF NOT EXISTS planet_positions (
                    type TEXT NOT NULL,
                    type_id INTEGER NOT NULL,
                    time_s FLOAT NOT NULL,
                    x DOUBLE NOT NULL,
                    y DOUBLE NOT NULL,
                    z DOUBLE NOT NULL
                )
            """)
            
            # Bulk insert (extremely fast)
            con.execute("INSERT INTO planet_positions SELECT * FROM df")
            con.close()
            
            db_time = time.time() - db_start
            if verbose:
                print(f"Database insertion completed in {db_time:.2f}s ({steps/db_time:.0f} rows/s)")
        else:
            # Fallback: batch insert
            con = duckdb.connect(database='database/my-db.duckdb', read_only=False)
            con.execute("""
                CREATE TABLE IF NOT EXISTS planet_positions (
                    type TEXT NOT NULL,
                    type_id INTEGER NOT NULL,
                    time_s FLOAT NOT NULL,
                    x DOUBLE NOT NULL,
                    y DOUBLE NOT NULL,
                    z DOUBLE NOT NULL
                )
            """)
            
            batch_size = 10000
            for i in range(0, steps, batch_size):
                end_idx = min(i + batch_size, steps)
                batch = [
                    (object_type, object_id, times[j], positions[j][0], positions[j][1], positions[j][2])
                    for j in range(i, end_idx)
                ]
                con.executemany(
                    "INSERT INTO planet_positions (type, type_id, time_s, x, y, z) VALUES (?, ?, ?, ?, ?, ?)",
                    batch
                )
                if verbose and end_idx % 50000 == 0:
                    print(f"  Inserted {end_idx:,}/{steps:,} rows...")
            
            con.close()
            db_time = time.time() - db_start
            if verbose:
                print(f"Database insertion completed in {db_time:.2f}s")

    # Write JSON file
    samples = None
    if use_json:
        json_start = time.time()
        
        if NUMPY_AVAILABLE:
            # Convert NumPy arrays to list of dicts
            samples = [
                {
                    'type': object_type,
                    'type_id': object_id,
                    'time_s': float(times[i]),
                    'x': float(positions[i, 0]),
                    'y': float(positions[i, 1]),
                    'z': float(positions[i, 2])
                }
                for i in range(steps)
            ]
        else:
            samples = [
                {
                    'type': object_type,
                    'type_id': object_id,
                    'time_s': times[i],
                    'x': positions[i][0],
                    'y': positions[i][1],
                    'z': positions[i][2]
                }
                for i in range(steps)
            ]
        
        with open(output_filename, 'w') as f:
            json.dump(samples, f, indent=2)
        
        json_time = time.time() - json_start
        if verbose:
            print(f"JSON file written in {json_time:.2f}s")

    total_time = time.time() - start_time
    if verbose:
        print(f"Total time: {total_time:.2f}s for {steps:,} samples")
        print(f"Average: {steps/total_time:.0f} samples/second")

    return samples


if __name__ == '__main__':

    parser = argparse.ArgumentParser(description="Generate Kepler orbit samples (60 Hz default).")
    parser.add_argument('--type', '-t', choices=['planet', 'moon'], default='planet',
                        help="Object type: 'planet' or 'moon'")
    parser.add_argument('--id', '-i', required=True, help="Database identifier")
    parser.add_argument('--out', help="Output JSON filename (overrides default naming)")
    parser.add_argument('--duration', '-d', type=float, default=60.0, help="Duration in seconds")
    parser.add_argument('--dt', type=float, default=0.01666667, help="Sample timestep in seconds")
    parser.add_argument('--no-db', action='store_true', help="Skip database insertion")
    parser.add_argument('--no-json', action='store_true', help="Skip JSON file generation")
    parser.add_argument('--quiet', '-q', action='store_true', help="Suppress progress output")
    args = parser.parse_args()

    # Sanitize id for filesystem use
    safe_id = re.sub(r'[^A-Za-z0-9._-]', '_', args.id)
    out = args.out or f'database/planet_positions_{args.type}_{safe_id}_60hz.json'

    # PostgreSQL connection parameters
    PGHOST = os.getenv('PGHOST', 'localhost')
    PGPORT = int(os.getenv('PGPORT', '5432'))
    PGUSER = os.getenv('PGUSER', 'postgres')
    PGPASSWORD = os.getenv('PGPASSWORD', 'localpass')
    DBNAME = 'resources_dynamic'

    table = 'planets' if args.type == 'planet' else 'planet_moons'

    # Fetch parameters from PostgreSQL
    params = None
    row = None
    try:
        conn = psycopg2.connect(host=PGHOST, port=PGPORT, user=PGUSER, password=PGPASSWORD, dbname=DBNAME)
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute(f"SELECT * FROM {table} WHERE id = %s LIMIT 1", (args.id,))
        row = cur.fetchone()
        cur.close()
        conn.close()
    except Exception as e:
        print(f"Warning: could not query PostgreSQL ({e}), will use defaults.")

    if row:
        def _gv(keys, default=None):
            for k in keys:
                if k in row and row[k] is not None:
                    return row[k]
            return default

        params = {
            'star_mass_kg': 1.98847e30 * float(_gv(['star_mass_kg'], 0.758581416228569)),
            'planet_mass_kg': float(_gv(['mass_kg'], 5.972e24)),
            'periapsis_AU': float(_gv(['periapsis_AU'], 0.0)),
            'apoapsis_AU': float(_gv(['apoapsis_AU'], 0.0)),
            'inc_deg': float(_gv(['inc_deg'], 0.0)),
            'node_deg': float(_gv(['node_deg'], 0.0)),
            'arg_peri_deg': float(_gv(['arg_peri_deg'], 0.0)),
            'mean_anomaly_deg': float(_gv(['mean_anomaly_deg'], 0.0)),
            'initial_position_m': Vector3(1.0 * OrbitKepler.AU_M, 0.0, 0.0),
            'object_type': args.type,
            'object_id': args.id,
        }
    else:
        print(f"No database row found for id={args.id}, using defaults")
        params = {
            'star_mass_kg': 1.98847e30 * 0.758581416228569,
            'planet_mass_kg': 5.972e24,
            'periapsis_AU': 0.98,
            'apoapsis_AU': 1.02,
            'inc_deg': 0.0,
            'node_deg': 0.0,
            'arg_peri_deg': 0.0,
            'mean_anomaly_deg': 0.0,
            'initial_position_m': Vector3(1.0 * OrbitKepler.AU_M, 0.0, 0.0),
            'object_type': args.type,
            'object_id': args.id,
        }

    print(f"\n{'='*60}")
    print(f"Kepler Position Generator")
    print(f"{'='*60}")
    print(f"Object: {args.type} id={args.id}")
    print(f"Duration: {args.duration}s at dt={args.dt}s")
    print(f"Output: {out}")
    print(f"{'='*60}\n")

    samples = generate_positions(
        out,
        args.duration,
        sample_dt=args.dt,
        params=params,
        use_db=not args.no_db,
        use_json=not args.no_json,
        verbose=not args.quiet
    )

    if samples and not args.quiet:
        print(f"\nFirst sample:")
        print(json.dumps(samples[0], indent=2))
        print(f"\nLast sample:")
        print(json.dumps(samples[-1], indent=2))