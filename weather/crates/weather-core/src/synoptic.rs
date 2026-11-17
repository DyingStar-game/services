//! The dynamic weather: pressure systems (depressions, anticyclones), dust storms and rain fronts
//! that are born, drift with the flow of their latitude band, and die — as a PURE FUNCTION of
//! (climate, time). No state is kept: every discrete decision (does a system exist in this slot at
//! this epoch, how strong, which kind) comes from an integer hash, and every continuous quantity is
//! an envelope that is exactly zero at the system's birth and death. Any instance, at any time,
//! computes the same sky.
//!
//! "En clair" : la planète est découpée en cases (bandes de latitude × créneaux de longitude, de la
//! taille d'un vrai système météo) ; chaque case tire au sort, de façon reproductible, si un système
//! y naît à chaque « époque », de quel type et avec quelle force ; le système dérive avec le vent de
//! sa bande et s'éteint en douceur.

use crate::climate::{band_wind, daily_mean_insolation_factor, Climate};
use crate::hash::{mix, signed_unit, unit};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Depression,
    Anticyclone,
    DustStorm,
    RainFront,
}

/// One living system at a given time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SystemState {
    /// Stable id of the (slot, epoch): the same system keeps it from birth to death.
    pub id: u64,
    pub kind: Kind,
    pub born_at: f64,
    pub dies_at: f64,
    pub centre_lon_deg: f64,
    pub centre_lat_deg: f64,
    /// Gaussian half-width σ of the system, metres along the ground.
    pub sigma_m: f64,
    /// Envelope now, 0–1.
    pub intensity: f64,
    pub peak_intensity: f64,
    /// Pressure anomaly at the centre now, Pa (negative for a depression).
    pub pressure_anomaly_pa: f64,
    /// Drift of the centre, m/s along the ground (east, north).
    pub drift_east_m_s: f64,
    pub drift_north_m_s: f64,
    /// Peak tangential wind at full intensity, m/s.
    pub wind_peak_m_s: f64,
    /// Dust optical depth added at the centre at full intensity.
    pub dust_tau: f64,
    /// Precipitation at the centre at full intensity, mm/h.
    pub precipitation_mm_h: f64,
    pub humidity: f64,
    // Where it came from, for the readout.
    pub band: i32,
    pub slot: i32,
    pub epoch: i64,
}

/// The lattice and the time scales of a body's systems, from its climate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Lattice {
    pub band_step_deg: f64,
    pub bands: Vec<f64>,
    /// Spacing of the slots along a band, metres on the ground (≈ 2.5 deformation radii).
    pub spacing_m: f64,
    /// Lifecycle period of a slot, seconds; a system lives 80 % of it.
    pub period_s: f64,
}

/// Brunt–Väisälä frequency used for the deformation radius, s⁻¹ (an Earth-like troposphere).
const BRUNT_VAISALA: f64 = 0.012;
/// Systems exist in this share of (slot, epoch) draws.
const EXISTENCE: f64 = 0.55;
/// A system lives this share of its slot's period; the rest is quiet.
const LIFE_SHARE: f64 = 0.8;

pub fn lattice(c: &Climate) -> Lattice {
    let band_step = 15.0;
    let bands: Vec<f64> = (-5..=5).map(|i| i as f64 * band_step).collect();
    // Deformation radius N·H/f at 45°; a slow rotator has a huge one: systems the size of the
    // planet, capped at a quarter of its circumference.
    let f = c.coriolis_45_s.max(1e-7);
    let deformation_m = BRUNT_VAISALA * c.tropopause_m.max(2_000.0) / f;
    let mut spacing = (2.5 * deformation_m).clamp(1_500_000.0, PI * c.radius_m / 2.0);
    // The cap: with 11 bands, the equatorial band alone gets ~1/7 of the slots.
    let slots_wanted = (c.extras.max_systems.max(11) as f64 / 7.0).max(3.0);
    spacing = spacing.max(TAU * c.radius_m / slots_wanted).min(PI * c.radius_m / 2.0);
    // Earth's systems live ~4 days; the time scale stretches with the day (slower rotation).
    let omega_earth = TAU / 86_164.1;
    let period = (4.0 * 86_400.0 * (omega_earth / c.omega_rad_s.max(1e-7))).clamp(12.0 * 3600.0, 20.0 * 86_400.0);
    Lattice { band_step_deg: band_step, bands, spacing_m: spacing, period_s: period }
}

/// Number of longitude slots in a band.
pub fn slots_in_band(l: &Lattice, c: &Climate, lat_deg: f64) -> i32 {
    let circumference = TAU * c.radius_m * lat_deg.to_radians().cos();
    ((circumference / l.spacing_m).round() as i32).max(3)
}

/// Every system alive at `t`, given the star's declination today (seasons steer the dust storms).
pub fn systems_at(c: &Climate, t: f64, decl_deg: f64) -> Vec<SystemState> {
    if !c.has_atmosphere || !c.extras.weather_enabled {
        return vec![];
    }
    let l = lattice(c);
    let mut out = Vec::new();
    for (band, &lat) in l.bands.iter().enumerate() {
        let n = slots_in_band(&l, c, lat);
        for slot in 0..n {
            if let Some(s) = system_in_slot(c, &l, band as i32, slot, n, t, decl_deg) {
                out.push(s);
            }
        }
    }
    out
}

/// The system of one slot at `t`, if one is alive.
#[allow(clippy::too_many_arguments)]
pub fn system_in_slot(c: &Climate, l: &Lattice, band: i32, slot: i32, slots: i32, t: f64, decl_deg: f64) -> Option<SystemState> {
    let seed = c.seed;
    let lat = l.bands[band as usize];
    // Each slot runs its own clock, offset by a hashed phase, so births are spread over time.
    let phase = unit(mix(seed, band as i64, slot as i64, 0)) * l.period_s;
    let epoch = ((t - phase) / l.period_s).floor() as i64;
    let born = epoch as f64 * l.period_s + phase;
    let life = LIFE_SHARE * l.period_s;
    let dies = born + life;
    if t < born || t >= dies {
        return None;
    }
    let draw = |k: i64| mix(seed, band as i64 * 100_003 + slot as i64, epoch, k);
    if unit(draw(1)) >= EXISTENCE {
        return None;
    }
    // Kind: half lows, half highs; on a dusty world a low in the season of strong sun becomes a dust
    // storm; on a wet world it carries rain.
    let low = unit(draw(2)) < 0.5;
    let season = (daily_mean_insolation_factor(lat, decl_deg) / 0.35).clamp(0.0, 1.5);
    let kind = if low {
        let dusty = c.extras.dust_lift_bias * season * 0.8;
        let wet = c.extras.humidity_inventory;
        let r = unit(draw(3));
        if c.extras.humidity_inventory < 0.3 && r < dusty {
            Kind::DustStorm
        } else if wet >= 0.3 && r < wet {
            Kind::RainFront
        } else {
            Kind::Depression
        }
    } else {
        Kind::Anticyclone
    };
    let peak = 0.5 + 0.5 * unit(draw(4));
    // Envelope: a smooth hump for pressure systems; a dust storm rises in ~2 h, holds, and fades over
    // ~4 h (Mars's regional storms).
    let age = t - born;
    let intensity = match kind {
        Kind::DustStorm => {
            let rise = (2.0f64 * 3600.0).min(life / 4.0);
            let fall = (4.0f64 * 3600.0).min(life / 3.0);
            smoothstep((age / rise).min((dies - t) / fall).clamp(0.0, 1.0))
        }
        _ => (PI * age / life).sin().powi(2),
    } * peak;
    // Birth place: the slot centre, jittered; drift with the flow of the band (systems ride at about
    // half the jet, in the sense of the band's surface wind).
    let slot_width_deg = 360.0 / slots as f64;
    let lon0 = (slot as f64 + 0.5) * slot_width_deg + signed_unit(draw(5)) * 0.4 * slot_width_deg;
    let lat0 = lat + signed_unit(draw(6)) * 0.4 * l.band_step_deg;
    let band_flow = band_wind(c, lat0);
    let zonal_sense = if band_flow.east_m_s >= 0.0 { 1.0 } else { -1.0 };
    let drift_east = zonal_sense * 0.5 * c.u_jet_m_s * (0.5 + 0.5 * unit(draw(7)));
    let drift_north = 0.2 * drift_east.abs() * signed_unit(draw(8));
    let cos_lat = lat0.to_radians().cos().max(0.05);
    let lon = lon0 + (drift_east * age / (c.radius_m * cos_lat)).to_degrees();
    let lat_now = (lat0 + (drift_north * age / c.radius_m).to_degrees()).clamp(-85.0, 85.0);
    let sigma = 0.25 * l.spacing_m * (0.6 + 0.4 * unit(draw(9)));
    // Pressure: a deep Earth low is ~3 % of the surface pressure; the share shrinks with the
    // thermal contrast a massive air allows.
    let contrast = (c.delta_t_equator_pole_k / 35.0).clamp(0.2, 1.5);
    let dp_max = 0.03 * c.pressure_pa * contrast;
    let sign = match kind {
        Kind::Anticyclone => 1.0,
        _ => -1.0,
    };
    let pressure_anomaly = sign * dp_max * intensity;
    // Wind around a Gaussian pressure hump: geostrophic peak Δp·e^(−½)/(ρ f σ) at r = σ, or the
    // cyclostrophic bound √(Δp/ρ) when Coriolis is too weak to balance it; capped by the data.
    let f = c.coriolis_45_s.max(1e-7) * lat0.to_radians().sin().abs().max(0.15) / 45f64.to_radians().sin();
    let geostrophic = dp_max * peak * (-0.5f64).exp() / (c.rho0_kg_m3 * f * sigma);
    let cyclostrophic = (dp_max * peak / c.rho0_kg_m3).sqrt();
    let wind_peak = geostrophic.min(cyclostrophic).min(c.extras.wind_scale_max_ms);
    let (dust_tau, precipitation, humidity) = match kind {
        Kind::DustStorm => (c.extras.dust_tau_max * peak, 0.0, 0.0),
        Kind::RainFront => (0.0, 2.0 + 8.0 * peak, c.extras.humidity_inventory),
        Kind::Depression => (0.0, 0.0, 0.5 * c.extras.humidity_inventory),
        Kind::Anticyclone => (0.0, 0.0, 0.2 * c.extras.humidity_inventory),
    };
    Some(SystemState {
        id: draw(0),
        kind,
        born_at: born,
        dies_at: dies,
        centre_lon_deg: wrap_lon(lon),
        centre_lat_deg: lat_now,
        sigma_m: sigma,
        intensity,
        peak_intensity: peak,
        pressure_anomaly_pa: pressure_anomaly,
        drift_east_m_s: drift_east,
        drift_north_m_s: drift_north,
        wind_peak_m_s: wind_peak,
        dust_tau,
        precipitation_mm_h: precipitation,
        humidity,
        band,
        slot,
        epoch,
    })
}

fn smoothstep(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

pub fn wrap_lon(lon: f64) -> f64 {
    let x = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if x == -180.0 {
        180.0
    } else {
        x
    }
}

/// What one system adds at a point: wind (east, north), pressure, dust, rain, humidity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    pub wind_east_m_s: f64,
    pub wind_north_m_s: f64,
    pub pressure_pa: f64,
    pub dust_tau: f64,
    pub precipitation_mm_h: f64,
    pub humidity: f64,
    /// Gaussian weight at the point, 0–1 (1 at the centre).
    pub weight: f64,
}

/// Great-circle distance (m) and the bearing from the centre to the point as (east, north) unit
/// components, on a sphere of radius `r`.
pub fn offset_on_sphere(r: f64, from_lon_deg: f64, from_lat_deg: f64, to_lon_deg: f64, to_lat_deg: f64) -> (f64, f64, f64) {
    let (lon1, lat1, lon2, lat2) = (from_lon_deg.to_radians(), from_lat_deg.to_radians(), to_lon_deg.to_radians(), to_lat_deg.to_radians());
    let dlon = lon2 - lon1;
    let a = ((lat2 - lat1) / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    let d = 2.0 * r * a.sqrt().min(1.0).asin();
    // Longitude runs from +X toward -Z (the game's QGIS convention); east is the direction the
    // body turns toward, which is INcreasing longitude in this frame (see spin.rs). So the east
    // component is +Δlon.
    let east = dlon.sin() * lat2.cos();
    let north = lat2.cos() * lat1.sin() * dlon.cos() * -1.0 + lat2.sin() * lat1.cos();
    let n = (east * east + north * north).sqrt();
    if n < 1e-12 {
        (d, 0.0, 1.0)
    } else {
        (d, east / n, north / n)
    }
}

/// Contribution of a system at (lon, lat). Lows turn counter-clockwise seen from above in the
/// northern hemisphere (clockwise south), highs the other way; the surface flow leans 30 % inward
/// around a low, outward around a high.
pub fn contribution(c: &Climate, s: &SystemState, lon_deg: f64, lat_deg: f64) -> Contribution {
    let (r, be, bn) = offset_on_sphere(c.radius_m, s.centre_lon_deg, s.centre_lat_deg, lon_deg, lat_deg);
    let sigma = s.sigma_m;
    let weight = (-r * r / (2.0 * sigma * sigma)).exp();
    if weight < 1e-4 {
        return Contribution::default();
    }
    // Tangential speed of a Gaussian hump: peaks at r = σ.
    let v_t = s.wind_peak_m_s * s.intensity * (r / sigma) * (0.5 - r * r / (2.0 * sigma * sigma)).exp();
    let northern = s.centre_lat_deg >= 0.0;
    let low = matches!(s.kind, Kind::Depression | Kind::DustStorm | Kind::RainFront);
    // Counter-clockwise seen from above (north up, east right): bearing (e, n) → (−n, e).
    let ccw = low == northern;
    let (te, tn) = if ccw { (-bn, be) } else { (bn, -be) };
    let inward = if low { -0.3 } else { 0.3 };
    let we = v_t * (te + inward * be);
    let wn = v_t * (tn + inward * bn);
    Contribution {
        wind_east_m_s: we,
        wind_north_m_s: wn,
        pressure_pa: s.pressure_anomaly_pa * weight,
        dust_tau: s.dust_tau * s.intensity * weight,
        precipitation_mm_h: s.precipitation_mm_h * s.intensity * weight,
        humidity: s.humidity * weight,
        weight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate::{derive, BodyInputs, BodyKind, ClimateExtras, OrbitElements, StarInputs, Temperatures};
    use crate::constants::{AU_M, EARTH_MASS_KG};

    fn earth_like(seed: u64, dust: f64, humidity: f64) -> Climate {
        let sun = 1.989e30;
        derive(&BodyInputs {
            name: "Test".into(),
            kind: BodyKind::Planet,
            radius_m: 6.371e6,
            mass_kg: EARTH_MASS_KG,
            rotation_period_s: 86_164.1,
            tidal_locked: false,
            tilt_deg: 23.44,
            orbit: OrbitElements { periapsis_au: 0.9833, apoapsis_au: 1.0167, inclination_deg: 0.0, ascending_node_deg: 0.0, arg_peri_deg: 0.0, mean_anomaly_deg: 0.0, primary_mass_kg: sun },
            star_distance_m: AU_M,
            star: StarInputs { luminosity_w: 3.828e26, mass_kg: sun, temperature_k: 5772.0 },
            pressure_pa: 101_325.0,
            gases_pct: vec![("N2".into(), 78.0), ("O2".into(), 21.0), ("Ar".into(), 1.0)],
            bond_albedo: 0.3,
            temperatures: Some(Temperatures { day_k: 295.0, night_k: 281.0, surface_k: 288.0, eq_k: 255.0 }),
            extras: ClimateExtras { weather_seed: Some(seed), dust_lift_bias: dust, humidity_inventory: humidity, ..Default::default() },
        })
    }

    #[test]
    fn same_inputs_same_sky_and_different_seeds_differ() {
        let c = earth_like(1, 0.5, 0.0);
        let a = systems_at(&c, 1_791_313_357.0, 10.0);
        let b = systems_at(&c, 1_791_313_357.0, 10.0);
        assert_eq!(a, b);
        assert!(!a.is_empty());
        let other = systems_at(&earth_like(2, 0.5, 0.0), 1_791_313_357.0, 10.0);
        assert_ne!(a.iter().map(|s| s.id).collect::<Vec<_>>(), other.iter().map(|s| s.id).collect::<Vec<_>>());
    }

    #[test]
    fn an_earth_like_world_carries_a_few_dozen_systems() {
        let c = earth_like(3, 0.5, 0.0);
        let n = systems_at(&c, 1.0e9, 0.0).len();
        assert!((10..=80).contains(&n), "{n} systems alive");
    }

    #[test]
    fn envelopes_are_zero_at_birth_and_death_and_continuous() {
        let c = earth_like(4, 0.9, 0.0);
        let t0 = 5.0e8;
        let alive = systems_at(&c, t0, 0.0);
        let s = alive.iter().max_by(|a, b| a.intensity.partial_cmp(&b.intensity).unwrap()).unwrap();
        let l = lattice(&c);
        let at = |t: f64| system_in_slot(&c, &l, s.band, s.slot, slots_in_band(&l, &c, l.bands[s.band as usize]), t, 0.0);
        let just_born = at(s.born_at + 1e-3).expect("alive just after birth");
        assert!(just_born.intensity < 1e-3, "{}", just_born.intensity);
        let about_to_die = at(s.dies_at - 1e-3).expect("alive just before death");
        assert!(about_to_die.intensity < 1e-3, "{}", about_to_die.intensity);
        assert!(at(s.dies_at + 1.0).map(|n| n.id != s.id).unwrap_or(true), "gone after death");
        let (p, q) = (at(t0).unwrap(), at(t0 + 1.0).unwrap());
        assert!((p.intensity - q.intensity).abs() < 1e-3);
        assert!((p.centre_lon_deg - q.centre_lon_deg).abs() < 0.01 || (p.centre_lon_deg - q.centre_lon_deg).abs() > 359.0);
    }

    #[test]
    fn lows_in_the_northern_hemisphere_turn_counter_clockwise_and_drift_east_in_the_westerlies() {
        let c = earth_like(5, 0.0, 0.0);
        let mut found = false;
        // Scan a few days: the lattice is sparse, a mature mid-latitude low is not there every hour.
        let candidates: Vec<SystemState> = (0..80).flat_map(|k| systems_at(&c, 7.0e8 + k as f64 * 6.0 * 3600.0, 0.0)).collect();
        for s in candidates {
            if s.kind == Kind::Depression && s.centre_lat_deg > 35.0 && s.centre_lat_deg < 58.0 && s.intensity > 0.3 {
                found = true;
                // East of the centre (larger longitude) the wind blows north (counter-clockwise seen
                // from above).
                let east_point = wrap_lon(s.centre_lon_deg + (s.sigma_m / (c.radius_m * s.centre_lat_deg.to_radians().cos())).to_degrees());
                let k = contribution(&c, &s, east_point, s.centre_lat_deg);
                assert!(k.wind_north_m_s > 0.0, "{k:?} for {s:?}");
                assert!(s.drift_east_m_s > 0.0, "westerlies carry systems east: {}", s.drift_east_m_s);
                assert!(s.pressure_anomaly_pa < 0.0 && s.pressure_anomaly_pa > -6000.0, "{}", s.pressure_anomaly_pa);
                assert!(s.wind_peak_m_s > 3.0 && s.wind_peak_m_s <= 60.0, "{}", s.wind_peak_m_s);
            }
        }
        assert!(found, "no mature mid-latitude low in the sample");
    }

    #[test]
    fn dust_storms_need_a_dusty_dry_world_and_rain_a_wet_one() {
        let dry = systems_at(&earth_like(6, 0.9, 0.0), 3.0e8, 20.0);
        assert!(dry.iter().any(|s| s.kind == Kind::DustStorm), "a dusty world raises storms");
        assert!(dry.iter().all(|s| s.kind != Kind::RainFront));
        let wet = systems_at(&earth_like(6, 0.0, 0.8), 3.0e8, 20.0);
        assert!(wet.iter().any(|s| s.kind == Kind::RainFront), "a wet world has rain fronts");
        assert!(wet.iter().all(|s| s.kind != Kind::DustStorm));
    }

    #[test]
    fn a_contribution_fades_with_distance() {
        let c = earth_like(7, 0.5, 0.0);
        let s = systems_at(&c, 4.0e8, 0.0).into_iter().max_by(|a, b| a.intensity.partial_cmp(&b.intensity).unwrap()).unwrap();
        let near = contribution(&c, &s, s.centre_lon_deg, s.centre_lat_deg);
        let far_lon = wrap_lon(s.centre_lon_deg + 90.0);
        let far = contribution(&c, &s, far_lon, s.centre_lat_deg);
        assert!(near.weight > 0.99 && far.weight < 1e-3);
        assert!(near.wind_east_m_s.abs() < 1e-6 && near.wind_north_m_s.abs() < 1e-6, "calm eye at the centre");
    }
}
