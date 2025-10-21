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
from typing import List, Dict
import duckdb


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
                       params: Dict = None) -> List[Dict]:
    """Generate positions and write to JSON file. Returns list of samples."""
    params = params or {}

    # Default parameters
    star_mass_kg = params.get('star_mass_kg', 1.98847e30)
    planet_mass_kg = params.get('planet_mass_kg', 5.972e24)
    periapsis_AU = params.get('periapsis_AU', 0.98)
    apoapsis_AU = params.get('apoapsis_AU', 1.02)
    inc_deg = params.get('inc_deg', 0.0)
    node_deg = params.get('node_deg', 0.0)
    arg_peri_deg = params.get('arg_peri_deg', 0.0)
    mean_anomaly_deg = params.get('mean_anomaly_deg', 0.0)

    # connection to the database
    con = duckdb.connect(database='database/my-db.duckdb', read_only=False)

    # create tables if they don't exist
    con.execute("CREATE TABLE IF NOT EXISTS planet_positions (time_s FLOAT NOT NULL,x DOUBLE NOT NULL,y DOUBLE NOT NULL,z DOUBLE NOT NULL)")


    # initial position (1 AU on x axis) unless specified
    initial_position_m = params.get('initial_position_m', Vector3(1.0 * OrbitKepler.AU_M, 0.0, 0.0))

    orbit = OrbitKepler(star_mass_kg=star_mass_kg,
                        planet_mass_kg=planet_mass_kg,
                        periapsis_AU=periapsis_AU,
                        apoapsis_AU=apoapsis_AU,
                        inc_deg=inc_deg,
                        node_deg=node_deg,
                        arg_peri_deg=arg_peri_deg,
                        mean_anomaly_deg=mean_anomaly_deg,
                        initial_position_m=initial_position_m)

    samples = []
    t = 0.0
    steps = max(1, int(math.ceil(duration_seconds / sample_dt)))

    values = []
    nbValues = 0
    for i in range(steps):
        pos = orbit.advance(sample_dt if i > 0 else 0.0)  # first sample at t=0
        samples.append({
            'time_s': round(t, 3),
            'x': pos.x,
            'y': pos.y,
            'z': pos.z
        })
        nbValues += 1
        values.append((round(t, 3), pos.x, pos.y, pos.z))
        if nbValues >= 10000:
            con.executemany("INSERT INTO planet_positions (time_s, x, y, z) VALUES (?, ?, ?, ?)", values)
            values = []
            nbValues = 0
            # print length of samples
            print(f"Inserted {len(samples)} samples so far...")
        t += sample_dt

    if nbValues > 0:
        con.executemany("INSERT INTO planet_positions (time_s, x, y, z) VALUES (?, ?, ?, ?)", values)
    con.close()
    # Write minimal JSON array to file
    with open(output_filename, 'w') as f:
        json.dump(samples, f, indent=2)

    return samples


if __name__ == '__main__':
    # Example run: generate 1 second of data at 16.66667 ms steps
    out = 'database/planet_positions_60hz.json'
    duration = 60.0  # 24.0 * 60.0 * 60.0  # seconds (short smoke test)
    dt = 0.01666667

    params = {
        'star_mass_kg': 1.98847e30 * 0.758581416228569,
        'planet_mass_kg': 5.972e24 * 0.0547453576852204,
        'periapsis_AU': 0.769595856230391,
        'apoapsis_AU': 0.809703378964002,
        'inc_deg': 0.691390066011356,
        'node_deg': 142.257975171927,
        'arg_peri_deg': 149.260586550769,
        'mean_anomaly_deg': 0.691390066011356,
        'initial_position_m': Vector3(1.0 * OrbitKepler.AU_M, 0.0, 0.0)
    }

    print(f"Generating {duration}s of samples at dt={dt}s to '{out}'...")
    samples = generate_positions(out, duration, sample_dt=dt, params=params)
    print(f"Wrote {len(samples)} samples. First sample:")
    print(json.dumps(samples[0], indent=2))
