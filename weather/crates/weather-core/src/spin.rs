//! A body's rotation and what follows from it: body-fixed directions, longitude and latitude, the
//! sub-stellar point, the local solar time and the declination of the star. Twin of
//! `Planet.spin_basis_at`, `HEALPix.vec2lonlat` and `Planet.get_local_solar_time` in the game.

use crate::math::{fposmod, Mat3, Vec3};
use std::f64::consts::TAU;

/// The body's orientation in its parent frame at absolute time `t`: tipped by its axial tilt about
/// +Z, then turned about its own +Y by its day (`Planet.spin_basis_at`). A period ≤ 0 means no spin.
pub fn spin_basis(tilt_rad: f64, rotation_period_s: f64, t: f64) -> Mat3 {
    let turns = if rotation_period_s > 0.0 { (t / rotation_period_s).rem_euclid(1.0) } else { 0.0 };
    Mat3::rz(tilt_rad).mul(Mat3::ry(turns * TAU))
}

/// The spin axis in the parent frame: +Y tipped by the tilt (independent of the time of day).
pub fn spin_axis(tilt_rad: f64) -> Vec3 {
    Mat3::rz(tilt_rad).apply(Vec3::UP)
}

/// A parent-frame direction expressed in the body-fixed frame (the inverse of the spin basis).
pub fn to_body_frame(basis: Mat3, dir: Vec3) -> Vec3 {
    basis.transposed().apply(dir)
}

/// Longitude and latitude (degrees) of a body-fixed unit direction: `lon = atan2(z, x)`,
/// `lat = asin(y)`, the game's HEALPix.vec2lonlat.
pub fn lonlat_of(dir: Vec3) -> (f64, f64) {
    let d = dir.normalized();
    (d.z.atan2(d.x).to_degrees(), d.y.clamp(-1.0, 1.0).asin().to_degrees())
}

/// Body-fixed unit direction of a longitude/latitude (degrees): HEALPix.lonlat2vec.
pub fn dir_of(lon_deg: f64, lat_deg: f64) -> Vec3 {
    let (lon, lat) = (lon_deg.to_radians(), lat_deg.to_radians());
    Vec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin())
}

/// Where the star stands overhead: longitude and latitude (degrees) of the sub-stellar point, from
/// the direction body → star in the PARENT frame. The latitude is the star's declination.
pub fn substellar_point(tilt_rad: f64, rotation_period_s: f64, t: f64, to_star_parent: Vec3) -> (f64, f64) {
    let basis = spin_basis(tilt_rad, rotation_period_s, t);
    lonlat_of(to_body_frame(basis, to_star_parent))
}

/// Declination of the star (degrees): the latitude of the sub-stellar point, which does not depend
/// on the time of day — only on the tilt and where the body is on its orbit.
pub fn declination_deg(tilt_rad: f64, to_star_parent: Vec3) -> f64 {
    spin_axis(tilt_rad).dot(to_star_parent.normalized()).clamp(-1.0, 1.0).asin().to_degrees()
}

/// Local solar time in hours (0–24, 12 = the star at its highest) at longitude `lon_deg`, given
/// the sub-stellar longitude. The day is split into 24 hours whatever its real length, as
/// `Planet.get_local_solar_time` does: with a positive spin about +Y, longitude `atan2(z, x)`
/// DEcreases as the body turns, so a point east of noon (smaller longitude) is in the afternoon.
pub fn local_solar_hours(lon_deg: f64, substellar_lon_deg: f64) -> f64 {
    let angle = substellar_lon_deg - lon_deg;
    fposmod(angle / 360.0 * 24.0 + 12.0, 24.0)
}

/// Hour angle in radians (0 at noon, positive in the afternoon), from local solar hours.
pub fn hour_angle_rad(local_solar_hours: f64) -> f64 {
    (local_solar_hours - 12.0) / 24.0 * TAU
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substellar_point_is_noon_and_its_antipode_midnight() {
        let to_star = Vec3::new(1.0, 0.0, 0.0);
        let (lon, lat) = substellar_point(0.0, 90_000.0, 0.0, to_star);
        assert!(lon.abs() < 1e-9 && lat.abs() < 1e-9);
        assert!((local_solar_hours(lon, lon) - 12.0).abs() < 1e-9);
        assert!((local_solar_hours(lon + 180.0, lon) - 0.0).abs() < 1e-9 || (local_solar_hours(lon + 180.0, lon) - 24.0).abs() < 1e-9);
    }

    #[test]
    fn a_quarter_turn_later_the_substellar_longitude_moved_by_90_degrees() {
        // Spin is a positive turn about +Y: +X goes toward -Z, so the star, fixed in the parent
        // frame, drifts toward +Z in the body frame — longitude atan2(z, x) INCREASES.
        let to_star = Vec3::new(1.0, 0.0, 0.0);
        let (lon0, _) = substellar_point(0.0, 100.0, 0.0, to_star);
        let (lon1, _) = substellar_point(0.0, 100.0, 25.0, to_star);
        assert!(((lon1 - lon0).rem_euclid(360.0) - 90.0).abs() < 1e-6, "{lon0} -> {lon1}");
    }

    #[test]
    fn declination_swings_with_the_tilt() {
        let tilt = 15f64.to_radians();
        // The axis tips toward -X (Rz(tilt)·UP = (-sin, cos, 0)): a star at -X sees the north pole
        // leaning toward it (summer in the north), a star at +X the opposite.
        assert!((declination_deg(tilt, Vec3::new(-1.0, 0.0, 0.0)) - 15.0).abs() < 1e-9);
        assert!((declination_deg(tilt, Vec3::new(1.0, 0.0, 0.0)) + 15.0).abs() < 1e-9);
        assert!(declination_deg(tilt, Vec3::new(0.0, 0.0, 1.0)).abs() < 1e-9, "equinox");
    }

    #[test]
    fn lonlat_round_trip() {
        for (lon, lat) in [(0.0, 0.0), (123.4, -45.6), (-170.0, 80.0)] {
            let (l2, b2) = lonlat_of(dir_of(lon, lat));
            assert!((l2 - lon).abs() < 1e-9 && (b2 - lat).abs() < 1e-9);
        }
    }
}
