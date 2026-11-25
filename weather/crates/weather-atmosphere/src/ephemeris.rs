//! Where the star stands in the body's sky at a given model time, from the body's REAL orbit and
//! spin (`weather_core::orbit` and `weather_core::spin`, the twins of the game's `kepler_orbit.gd`
//! and `Planet.spin_basis_at`), instead of the model's own idealised calendar (t = 0 = northern
//! spring equinox, a sine for the seasons, a uniform solar day).
//!
//! Model time and Unix time are tied by a fixed offset: `unix = model time + unix_offset_s`. The
//! offset is chosen once, at a cold start, and saved with every checkpoint, so that a resumed model
//! keeps seeing the same sky at the same instants (and stays bit-exact).
//!
//! Conventions: the longitude is the game's (`spin::lonlat_of`, increasing as the body turns), the
//! model's grid uses the same longitudes as the game's rasters, so the sub-stellar longitude goes
//! straight into the model. The declination is the latitude of the sub-stellar point. The distance
//! to the star scales the overhead flux by `a^2 sqrt(1 - e^2) / r^2`, whose mean over an orbit is
//! exactly 1: the model's flux setting stays the annual mean the data gives, and the instantaneous
//! flux swings around it (perihelion brighter, aphelion dimmer).

use weather_core::math::Vec3;
use weather_core::orbit::KeplerOrbit;
use weather_core::spin::{declination_deg, substellar_point};

#[derive(Clone, Debug)]
pub struct Ephemeris {
    /// Unix time (s) = model time (s) + this offset.
    pub unix_offset_s: f64,
    /// Axial tilt (radians, as in the data: > 90 degrees = retrograde spin).
    pub tilt_rad: f64,
    /// Sidereal rotation period (s); <= 0 = no spin.
    pub rotation_period_s: f64,
    /// The orbit around the STAR: the body's own for a planet, its planet's for a moon (a moon
    /// shares its planet's direction to the star, as everywhere else in the weather service).
    pub star_orbit: KeplerOrbit,
}

/// The star seen from the body at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunPosition {
    /// Declination = latitude of the sub-stellar point (radians).
    pub decl_rad: f64,
    /// Longitude of the sub-stellar point (radians, game convention).
    pub sub_lon_rad: f64,
    /// Overhead flux relative to its annual mean (distance to the star).
    pub flux_factor: f64,
}

impl Ephemeris {
    /// Direction body -> star in the parent frame at Unix time `unix` (not normalised).
    pub fn to_star(&self, unix: f64) -> Vec3 {
        self.star_orbit.position_at(unix).scale(-1.0)
    }

    /// The star at MODEL time `t`.
    pub fn at(&self, t: f64) -> SunPosition {
        let unix = t + self.unix_offset_s;
        let ts = self.to_star(unix);
        let decl = declination_deg(self.tilt_rad, ts);
        let (sub_lon, _) = substellar_point(self.tilt_rad, self.rotation_period_s, unix, ts);
        let r = ts.length();
        let (a, e) = (self.star_orbit.semi_major_axis_m(), self.star_orbit.eccentricity());
        let flux_factor = if r > 0.0 { a * a * (1.0 - e * e).max(0.0).sqrt() / (r * r) } else { 1.0 };
        SunPosition { decl_rad: decl.to_radians(), sub_lon_rad: sub_lon.to_radians(), flux_factor }
    }
}
