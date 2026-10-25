//! Planet geometry shared by the road graph and the driving controller.
//!
//! The QGIS exports (POIs, road centerlines) are in lon/lat (EPSG:4326); the
//! game places everything in the planet's local frame. The mapping is
//! `HEALPix.lonlat2vec` (`DyingStar/scenes/planet/healpix.gd`), reproduced
//! exactly here: Y is the polar axis, and a point at lon/lat on the surface is
//! `lonlat_to_unit(lon, lat) × (radius + elevation)` in the planet frame.

use crate::horizon::messages::Vec3;

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }
}

pub fn scale(a: Vec3, k: f64) -> Vec3 {
    Vec3 { x: a.x * k, y: a.y * k, z: a.z * k }
}

pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// `a` scaled to unit length; the zero vector is returned unchanged.
pub fn normalized(a: Vec3) -> Vec3 {
    let l = length(a);
    if l == 0.0 {
        a
    } else {
        scale(a, 1.0 / l)
    }
}

/// Unit direction of a lon/lat point (degrees), planet frame — identical to
/// `HEALPix.lonlat2vec`: `(cos φ·cos λ, sin φ, −cos φ·sin λ)`.
pub fn lonlat_to_unit(lon: f64, lat: f64) -> Vec3 {
    let (lon_r, lat_r) = (lon.to_radians(), lat.to_radians());
    let cl = lat_r.cos();
    Vec3 { x: cl * lon_r.cos(), y: lat_r.sin(), z: -cl * lon_r.sin() }
}

/// Inverse of [`lonlat_to_unit`] for any non-zero vector: `(lon, lat)` in
/// degrees, lon in (-180, 180].
pub fn unit_to_lonlat(v: Vec3) -> (f64, f64) {
    let n = normalized(v);
    let lat = n.y.clamp(-1.0, 1.0).asin().to_degrees();
    let lon = (-n.z).atan2(n.x).to_degrees();
    (lon, lat)
}

/// Great-circle distance in metres between two directions on a sphere of
/// `radius` metres. Inputs need not be normalized.
pub fn arc_m(a: Vec3, b: Vec3, radius: f64) -> f64 {
    let (a, b) = (normalized(a), normalized(b));
    // atan2 form: accurate for the tiny angles of a few metres, where acos of
    // a dot product loses everything to rounding.
    length(cross(a, b)).atan2(dot(a, b)) * radius
}

/// Shortest signed difference `b − a` between two longitudes, in degrees,
/// within [-180, 180] — so a road crossing the antimeridian is interpolated
/// the short way round.
pub fn lon_delta(a: f64, b: f64) -> f64 {
    let mut d = b - a;
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    d
}

/// Linear interpolation in lon/lat — how the game's road exporter places the
/// ribbon between two centerline vertices (`modifier_geom.with_cumulative` /
/// `partition_polyline` work in degrees). Following the same curve keeps a
/// densified route on the asphalt the client actually renders.
pub fn lerp_lonlat(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    let mut lon = a.0 + lon_delta(a.0, b.0) * t;
    if lon > 180.0 {
        lon -= 360.0;
    } else if lon <= -180.0 {
        lon += 360.0;
    }
    (lon, a.1 + (b.1 - a.1) * t)
}

/// Local horizontal frame at a point of the planet: `up` is the outward
/// normal, `east` and `north` span the tangent plane. Used to turn planet-frame
/// positions into flat 2D metres for the driving controller (altitude is left
/// to the vehicle's physics, which follows the terrain on its own).
#[derive(Debug, Clone, Copy)]
pub struct Tangent {
    pub origin: Vec3,
    pub up: Vec3,
    pub east: Vec3,
    pub north: Vec3,
}

impl Tangent {
    /// Frame at `origin` (planet frame, metres from the planet centre).
    pub fn at(origin: Vec3) -> Self {
        let up = normalized(origin);
        // Polar axis is +Y. At the exact poles any east will do.
        let mut east = cross(Vec3 { x: 0.0, y: 1.0, z: 0.0 }, up);
        if length(east) < 1e-9 {
            east = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
        }
        let east = normalized(east);
        let north = cross(up, east);
        Self { origin, up, east, north }
    }

    /// `p` (planet frame) as `(east, north)` metres from the origin, after
    /// projecting it onto the tangent plane.
    pub fn project(&self, p: Vec3) -> (f64, f64) {
        let d = sub(p, self.origin);
        (dot(d, self.east), dot(d, self.north))
    }

    /// Same as [`project`](Self::project) for a direction (no origin offset).
    pub fn project_dir(&self, d: Vec3) -> (f64, f64) {
        (dot(d, self.east), dot(d, self.north))
    }

    /// Point of the tangent plane at `(east, north)` metres, back in the
    /// planet frame.
    pub fn unproject(&self, e: f64, n: f64) -> Vec3 {
        add(self.origin, add(scale(self.east, e), scale(self.north, n)))
    }
}

/// Rotate `v` by the Godot Euler angles `rot` (radians, Godot's default
/// `EULER_ORDER_YXZ`: the basis is `Ry · Rx · Rz`, applied to `v`).
pub fn rotate_euler_yxz(v: Vec3, rot: Vec3) -> Vec3 {
    // Rz
    let (sz, cz) = rot.z.sin_cos();
    let v = Vec3 { x: v.x * cz - v.y * sz, y: v.x * sz + v.y * cz, z: v.z };
    // Rx
    let (sx, cx) = rot.x.sin_cos();
    let v = Vec3 { x: v.x, y: v.y * cx - v.z * sx, z: v.y * sx + v.z * cx };
    // Ry
    let (sy, cy) = rot.y.sin_cos();
    Vec3 { x: v.x * cy + v.z * sy, y: v.y, z: -v.x * sy + v.z * cy }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn lonlat_matches_the_healpix_convention() {
        // lon 0 / lat 0 → +X ; lon 90 → −Z ; lat 90 → +Y.
        let v = lonlat_to_unit(0.0, 0.0);
        assert!(close(v.x, 1.0, 1e-12) && close(v.y, 0.0, 1e-12) && close(v.z, 0.0, 1e-12));
        let v = lonlat_to_unit(90.0, 0.0);
        assert!(close(v.x, 0.0, 1e-12) && close(v.z, -1.0, 1e-12));
        let v = lonlat_to_unit(0.0, 90.0);
        assert!(close(v.y, 1.0, 1e-12));
    }

    #[test]
    fn lonlat_round_trips() {
        for (lon, lat) in [(-39.535, 24.69), (179.9, -60.0), (-179.9, 10.0), (12.0, 0.0)] {
            let (lo, la) = unit_to_lonlat(scale(lonlat_to_unit(lon, lat), 6_356_000.0));
            assert!(close(lo, lon, 1e-9), "lon {lon} -> {lo}");
            assert!(close(la, lat, 1e-9), "lat {lat} -> {la}");
        }
    }

    #[test]
    fn the_seeded_village_sits_on_tarsis_3() {
        // startup_POI.json seeds a village at this planet-frame position; its
        // norm is the planet radius plus a little terrain.
        let p = Vec3 { x: 4_465_613.947, y: 2_669_284.115, z: -3_663_506.259 };
        let r = length(p);
        assert!(r > 6_356_000.0 && r < 6_380_000.0, "radius {r}");
    }

    #[test]
    fn arc_is_accurate_at_metre_scale() {
        let r = 6_356_000.0;
        let a = lonlat_to_unit(10.0, 20.0);
        // ~1 m north: 1 / (r · π/180) degrees of latitude.
        let b = lonlat_to_unit(10.0, 20.0 + 1.0 / (r * std::f64::consts::PI / 180.0));
        assert!(close(arc_m(a, b, r), 1.0, 1e-6));
    }

    #[test]
    fn lerp_takes_the_short_way_across_the_antimeridian() {
        let (lon, _) = lerp_lonlat((179.0, 0.0), (-179.0, 0.0), 0.5);
        assert!(close(lon.abs(), 180.0, 1e-9), "lon {lon}");
    }

    #[test]
    fn tangent_projection_is_east_north() {
        let r = 6_356_000.0;
        let o = scale(lonlat_to_unit(10.0, 20.0), r);
        let t = Tangent::at(o);
        let north = scale(lonlat_to_unit(10.0, 20.001), r);
        let east = scale(lonlat_to_unit(10.001, 20.0), r);
        let (e, n) = t.project(north);
        assert!(n > 100.0 && e.abs() < 1e-3 * n, "north → ({e}, {n})");
        let (e, n) = t.project(east);
        assert!(e > 100.0 && n.abs() < 1e-2 * e, "east → ({e}, {n})");
        let back = t.unproject(3.0, 4.0);
        let (e, n) = t.project(back);
        assert!(close(e, 3.0, 1e-6) && close(n, 4.0, 1e-6));
    }

    #[test]
    fn euler_yaw_follows_godot() {
        // Basis(Vector3.UP, PI/2) * Vector3(1,0,0) == Vector3(0,0,-1)
        let r = rotate_euler_yxz(
            Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            Vec3 { x: 0.0, y: std::f64::consts::FRAC_PI_2, z: 0.0 },
        );
        assert!(close(r.x, 0.0, 1e-12) && close(r.z, -1.0, 1e-12));
    }
}
