//! Keplerian orbit as a pure function of absolute time — the twin of the celestial service's
//! `kepler-orbit.ts` and the game's `kepler_orbit.gd`, pinned by the same closed-form test.
//! The weather needs it for the season (where the body is on its orbit decides the declination of
//! its star) and for the insolation (distance to the star).

use crate::constants::{AU_M, G, SOLAR_MASS_KG};
use crate::math::{normalize_angle, Mat3, Vec3};

#[derive(Clone, Debug)]
pub struct KeplerOrbit {
    semi_major_axis_m: f64,
    eccentricity: f64,
    mean_motion: f64,
    mean_anomaly_epoch: f64,
    basis: Mat3,
}

impl KeplerOrbit {
    /// Periapsis / apoapsis in AU, angles in radians, masses in kg (primary = the star for a planet,
    /// the planet for a moon).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        periapsis_au: f64,
        apoapsis_au: f64,
        inc_rad: f64,
        node_rad: f64,
        arg_peri_rad: f64,
        mean_anomaly_rad: f64,
        primary_mass_kg: f64,
        object_mass_kg: f64,
    ) -> Self {
        let mut rp = if periapsis_au > 0.0 { periapsis_au } else { apoapsis_au };
        let mut ra = if apoapsis_au > 0.0 { apoapsis_au } else { periapsis_au };
        if ra < rp {
            std::mem::swap(&mut rp, &mut ra);
        }
        let a = 0.5 * (rp + ra) * AU_M;
        let e = ((ra - rp) / (ra + rp).max(1e-12)).max(0.0);
        let primary = if primary_mass_kg > 0.0 { primary_mass_kg } else { SOLAR_MASS_KG };
        let mu = G * (primary + object_mass_kg);
        // Ry(node)·Rx(inc)·Ry(argPeri): the textbook Rz(Ω)·Rx(i)·Rz(ω) with the up axis renamed.
        let basis = Mat3::ry(node_rad).mul(Mat3::rx(inc_rad)).mul(Mat3::ry(arg_peri_rad));
        Self { semi_major_axis_m: a, eccentricity: e, mean_motion: (mu / a.powi(3)).sqrt(), mean_anomaly_epoch: mean_anomaly_rad, basis }
    }

    /// Position in the parent frame (metres) at absolute time `t` (unix seconds).
    pub fn position_at(&self, t: f64) -> Vec3 {
        let m = normalize_angle(self.mean_anomaly_epoch + self.mean_motion * t);
        let ea = solve_kepler(m, self.eccentricity);
        let a = self.semi_major_axis_m;
        let xp = a * (ea.cos() - self.eccentricity);
        // -sin: from +X toward -Z is the positive turn about +Y, the sense bodies spin in.
        let zp = -a * (1.0 - self.eccentricity * self.eccentricity).max(0.0).sqrt() * ea.sin();
        self.basis.apply(Vec3::new(xp, 0.0, zp))
    }

    pub fn period_s(&self) -> f64 {
        if self.mean_motion == 0.0 {
            0.0
        } else {
            std::f64::consts::TAU / self.mean_motion
        }
    }

    pub fn mean_motion(&self) -> f64 {
        self.mean_motion
    }

    pub fn semi_major_axis_m(&self) -> f64 {
        self.semi_major_axis_m
    }

    pub fn eccentricity(&self) -> f64 {
        self.eccentricity
    }

    /// Mean anomaly at `t`, radians in [0, 2π): where the body is in its year, the clock of the
    /// seasons when the orbit is nearly circular.
    pub fn mean_anomaly_at(&self, t: f64) -> f64 {
        normalize_angle(self.mean_anomaly_epoch + self.mean_motion * t)
    }

    /// Unit normal of the orbital plane (angular momentum direction): +Y for a flat orbit.
    pub fn normal(&self) -> Vec3 {
        self.basis.apply(Vec3::UP)
    }
}

/// Newton–Raphson on Kepler's equation E − e·sin E = M; 16 steps / 1e-12, as the service.
fn solve_kepler(mean_anomaly: f64, e: f64) -> f64 {
    let m = normalize_angle(mean_anomaly);
    let mut ea = if e > 0.8 { std::f64::consts::PI } else { m };
    for _ in 0..16 {
        let f = ea - e * ea.sin() - m;
        let fp = 1.0 - e * ea.cos();
        let step = -f / fp.max(1e-12);
        ea += step;
        if step.abs() < 1e-12 {
            break;
        }
    }
    normalize_angle(ea)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::EARTH_MASS_KG;

    const SUN_KG: f64 = 1.98892e30;
    const SANDBOX_STAR_KG: f64 = 1.5088184e30;

    fn ry(v: Vec3, a: f64) -> Vec3 {
        Vec3::new(v.x * a.cos() + v.z * a.sin(), v.y, -v.x * a.sin() + v.z * a.cos())
    }

    fn rx(v: Vec3, a: f64) -> Vec3 {
        Vec3::new(v.x, v.y * a.cos() - v.z * a.sin(), v.y * a.sin() + v.z * a.cos())
    }

    /// Ry(Ω)·Rx(i)·Ry(ω)·(a·cos M, 0, −a·sin M): the same closed form as the game's and the celestial
    /// service's tests, number for number.
    fn closed_form(a: f64, inc: f64, node: f64, argp: f64, m: f64) -> Vec3 {
        ry(rx(ry(Vec3::new(a * m.cos(), 0.0, -a * m.sin()), argp), inc), node)
    }

    #[test]
    fn inclined_circular_orbit_matches_the_shared_closed_form() {
        let (inc, node, argp) = (30f64.to_radians(), 40f64.to_radians(), 50f64.to_radians());
        let orbit = KeplerOrbit::new(1.0, 1.0, inc, node, argp, 0.0, SUN_KG, 0.0);
        let n = std::f64::consts::TAU / orbit.period_s();
        for fraction in [0.0, 0.25, 1.0 / 3.0, 0.7] {
            let t = fraction * orbit.period_s();
            let expected = closed_form(AU_M, inc, node, argp, n * t);
            assert!(orbit.position_at(t).distance_to(expected) < 1.0, "at {fraction}");
        }
    }

    #[test]
    fn flat_orbit_stays_in_xz_and_runs_counter_clockwise() {
        let orbit = KeplerOrbit::new(0.5335, 0.5665, 0.0, 110f64.to_radians(), 45f64.to_radians(), 20f64.to_radians(), SANDBOX_STAR_KG, 0.0);
        for i in 0..8 {
            assert!(orbit.position_at(orbit.period_s() * i as f64 / 8.0).y.abs() < 1.0);
        }
        let p0 = orbit.position_at(0.0);
        assert!(p0.cross(orbit.position_at(600.0).sub(p0)).y > 0.0, "momentum along +Y");
    }

    #[test]
    fn sandbox_period_and_epoch_position_match_the_game() {
        let orbit = KeplerOrbit::new(0.5335, 0.5665, 1.5f64.to_radians(), 110f64.to_radians(), 45f64.to_radians(), 20f64.to_radians(), SANDBOX_STAR_KG, 0.788 * EARTH_MASS_KG);
        assert!((orbit.period_s() / 86400.0 - 171.03).abs() < 0.05);
        // Measured on 2026-10-06 from both the deployed service and the game's solver at t = 0.
        let p = orbit.position_at(0.0);
        assert!(p.distance_to(Vec3::new(-79_770_702_964.0, 1_915_584_152.0, -5_282_535_365.0)) < 2.0, "{p:?}");
    }
}
