//! From a body's physical parameters to its climate: the numbers that never change (or only with
//! the season) and that every other layer reads — temperature, air column, circulation regime,
//! wind scale, dust thresholds. Pure functions; the formulas and their plain-language meaning are
//! in `.claude-docs/rapports/2026-10-06-meteo/00-synthese.md` §3 and in `01-climat-global.md`.
//!
//! Each derivation names its simplification. Reference checks (Earth, Mars, Tarsis III) live in
//! `tests/reference.rs`.

use crate::constants::{KARMAN, R_GAS, SIGMA};
use crate::gases::mixture;
use crate::hash::seed_from_name;
use crate::orbit::KeplerOrbit;
use crate::spin::{declination_deg, hour_angle_rad};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Everything the derivation needs about one body, in SI. Built by `bodies::load` from the
/// celestial data (`tarsis.json`) and the climate extras (`data/climate/<system>.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyInputs {
    /// Name as the celestial service knows it ("SandBox"); Horizon's uuid is `_planet_<name>`.
    pub name: String,
    pub kind: BodyKind,
    pub radius_m: f64,
    pub mass_kg: f64,
    /// Sidereal rotation period, seconds; ≤ 0 = no spin.
    pub rotation_period_s: f64,
    pub tidal_locked: bool,
    /// Axial tilt about +Z of the parent frame, degrees (> 90° = retrograde).
    pub tilt_deg: f64,
    pub orbit: OrbitElements,
    /// Distance to the star used for insolation (a planet's semi-major axis; its planet's for a moon).
    pub star_distance_m: f64,
    pub star: StarInputs,
    /// Surface pressure, Pa; 0 = airless.
    pub pressure_pa: f64,
    /// Composition as (symbol, percent).
    pub gases_pct: Vec<(String, f64)>,
    pub bond_albedo: f64,
    pub temperatures: Option<Temperatures>,
    pub extras: ClimateExtras,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    Planet,
    Moon,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrbitElements {
    pub periapsis_au: f64,
    pub apoapsis_au: f64,
    pub inclination_deg: f64,
    pub ascending_node_deg: f64,
    pub arg_peri_deg: f64,
    pub mean_anomaly_deg: f64,
    pub primary_mass_kg: f64,
}

impl OrbitElements {
    pub fn solver(&self, object_mass_kg: f64) -> KeplerOrbit {
        KeplerOrbit::new(
            self.periapsis_au,
            self.apoapsis_au,
            self.inclination_deg.to_radians(),
            self.ascending_node_deg.to_radians(),
            self.arg_peri_deg.to_radians(),
            self.mean_anomaly_deg.to_radians(),
            self.primary_mass_kg,
            object_mass_kg,
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StarInputs {
    pub luminosity_w: f64,
    pub mass_kg: f64,
    pub temperature_k: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Temperatures {
    pub day_k: f64,
    pub night_k: f64,
    pub surface_k: f64,
    pub eq_k: f64,
}

/// What the celestial data does not carry: the weather's own knobs, with defaults that read as
/// "a dry rocky world of quartz sand". `data/climate/<system>.json` overrides them per body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ClimateExtras {
    /// Seed of every random-looking decision; by default hashed from the body's name.
    pub weather_seed: Option<u64>,
    /// Density of the sand/dust grains, kg/m³ (quartz 2650, basalt 3000, corundum 4000).
    pub dust_grain_density_kg_m3: f64,
    /// Typical saltating grain diameter, m (100 µm: the easiest size to lift).
    pub dust_grain_diameter_m: f64,
    /// How much loose, liftable material the ground offers, 0 (bare rock) to 1 (dust plain).
    pub dust_lift_bias: f64,
    /// Thermal inertia of the ground, J m⁻² K⁻¹ s⁻¹ᐟ² (dust 56, sand 220, rock 2500, ocean ≫ 10⁴).
    pub thermal_inertia_tiu: f64,
    /// Available condensable volatiles, 0 (bone dry) to 1 (ocean world): drives humidity and clouds.
    pub humidity_inventory: f64,
    /// Gameplay cap on the surface wind, m/s (Star Citizen's precedent: landing zones stay usable).
    pub wind_scale_max_ms: f64,
    /// Gameplay cap on the dust optical depth of the worst storm.
    pub dust_tau_max: f64,
    /// Aerodynamic roughness length of the ground, m (sandy desert ~1e-4).
    pub roughness_length_m: f64,
    /// False for a body nobody lands on (a gas giant has no surface to be weathered).
    pub weather_enabled: bool,
    /// Cap on the number of system slots per body (the lattice spacing grows to fit).
    pub max_systems: u32,
}

impl Default for ClimateExtras {
    fn default() -> Self {
        Self {
            weather_seed: None,
            dust_grain_density_kg_m3: 2650.0,
            dust_grain_diameter_m: 100e-6,
            dust_lift_bias: 0.5,
            thermal_inertia_tiu: 300.0,
            humidity_inventory: 0.0,
            wind_scale_max_ms: 60.0,
            dust_tau_max: 10.0,
            roughness_length_m: 1e-4,
            weather_enabled: true,
            max_systems: 48,
        }
    }
}

/// The derived climate of one body. Units in the field names where they are not SI base.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Climate {
    pub name: String,
    pub kind: BodyKind,
    pub seed: u64,
    pub has_atmosphere: bool,
    pub radius_m: f64,
    pub surface_gravity: f64,
    pub rotation_period_s: f64,
    pub tidal_locked: bool,
    pub tilt_deg: f64,
    pub orbit_period_s: f64,
    // Air
    pub molar_mass_kg_mol: f64,
    pub cp_j_kg_k: f64,
    /// Ratio of specific heats γ.
    pub gamma: f64,
    pub greenhouse_fraction: f64,
    pub pressure_pa: f64,
    /// Annual-mean irradiance at the top of the air, W/m².
    pub irradiance_w_m2: f64,
    pub bond_albedo: f64,
    /// Equilibrium temperature of the bare body, K.
    pub t_eq_k: f64,
    /// Grey infrared optical depth of the air (the "blanket").
    pub tau_ir: f64,
    /// Mean surface temperature, K.
    pub t_surface_k: f64,
    /// Temperature of the upper air (grey model skin), K.
    pub t_skin_k: f64,
    /// Scale height, m: air is e times thinner every H.
    pub scale_height_m: f64,
    /// Air density at the surface, kg/m³.
    pub rho0_kg_m3: f64,
    /// Dry adiabatic lapse rate, K/m.
    pub lapse_dry_k_m: f64,
    /// Environmental lapse rate used for the tropopause, K/m.
    pub lapse_env_k_m: f64,
    pub tropopause_m: f64,
    pub sound_speed_m_s: f64,
    /// Radiative relaxation time of the air column, s (short = big day/night swing).
    pub radiative_time_s: f64,
    /// Half of the day/night temperature range, K.
    pub diurnal_amplitude_k: f64,
    // Circulation
    pub omega_rad_s: f64,
    pub coriolis_45_s: f64,
    pub rossby_thermal: f64,
    /// Latitude where the Hadley cell ends, degrees (≥ 90 = one cell pole to pole).
    pub hadley_edge_deg: f64,
    pub cells: u8,
    /// Annual-mean equator-to-pole temperature contrast, K.
    pub delta_t_equator_pole_k: f64,
    /// Jet speed aloft from the thermal wind, m/s.
    pub u_jet_m_s: f64,
    /// Surface wind scale (40 % of the jet, before the band factor), m/s.
    pub v10_scale_m_s: f64,
    /// Super-rotation cap for slow rotators, m/s.
    pub superrotation_cap_m_s: f64,
    // Dust
    pub u_star_threshold_m_s: f64,
    /// Wind at 10 m that starts lifting the sand, m/s.
    pub v10_threshold_m_s: f64,
    /// Settling speed of a 2 µm grain, m/day: how long a veil stays.
    pub settling_2um_m_day: f64,
    pub extras: ClimateExtras,
}

/// Derive the climate of a body. Pure; call once per body and keep the result.
pub fn derive(b: &BodyInputs) -> Climate {
    let g = crate::constants::G * b.mass_kg / (b.radius_m * b.radius_m);
    let mix = mixture(&b.gases_pct);
    let has_atmosphere = b.pressure_pa > 0.0;
    let r_spec = R_GAS / mix.molar_mass_kg_mol;
    let orbit = b.orbit.solver(b.mass_kg);
    let e = orbit.eccentricity();

    // --- 1. Energy balance: how warm on average -------------------------------------------------
    // Annual-mean irradiance at the top of the air: L / (4π d²), lifted by 1/√(1−e²) on an eccentric
    // orbit (the body spends more time near periapsis than the mean distance suggests).
    let irradiance = b.star.luminosity_w / (4.0 * PI * b.star_distance_m * b.star_distance_m) / (1.0 - e * e).sqrt();
    let t_eq = (irradiance * (1.0 - b.bond_albedo) / (4.0 * SIGMA)).powf(0.25);
    // The blanket: inverted from the data's mean surface temperature when it has one, else estimated
    // from the column mass and the greenhouse share, calibrated on Earth (1 bar, ~1 % → 0.84).
    let (tau_ir, t_surface) = match b.temperatures {
        Some(t) if has_atmosphere && t.surface_k > t_eq => ((4.0 / 3.0) * ((t.surface_k / t_eq).powi(4) - 1.0), t.surface_k),
        Some(t) if !has_atmosphere => (0.0, t.surface_k),
        _ => {
            let tau = if has_atmosphere { 0.84 * (b.pressure_pa / 1.013e5) * (mix.greenhouse_fraction / 0.01).max(1e-3).powf(0.15) } else { 0.0 };
            (tau, t_eq * (1.0 + 0.75 * tau).powf(0.25))
        }
    };
    let t_skin = t_eq * 2f64.powf(-0.25);

    // --- 2. The air column ----------------------------------------------------------------------
    let scale_height = if has_atmosphere { r_spec * t_surface / g } else { 0.0 };
    let rho0 = if has_atmosphere { b.pressure_pa / (r_spec * t_surface) } else { 0.0 };
    let lapse_dry = g / mix.cp_j_kg_k;
    // Real lapse rates run near two thirds of the dry one (moist or dusty convection); the
    // tropopause is where the surface air, cooling at that rate, meets the skin temperature.
    let lapse_env = 0.66 * lapse_dry;
    let tropopause = if has_atmosphere { ((t_surface - t_skin) / lapse_env).max(2_000.0) } else { 0.0 };
    let gamma = mix.cp_j_kg_k / (mix.cp_j_kg_k - r_spec);
    let sound_speed = if has_atmosphere { (gamma * r_spec * t_surface).sqrt() } else { 0.0 };
    // Column heat capacity c_p p_s / g against its own thermal emission: the time the air needs to
    // cool, which says whether nights are cold.
    let column_heat_capacity = mix.cp_j_kg_k * b.pressure_pa / g;
    let radiative_time = if has_atmosphere { column_heat_capacity / (4.0 * SIGMA * t_surface.powi(3)) } else { 0.0 };
    let diurnal_amplitude = diurnal_amplitude(b, irradiance, t_surface, column_heat_capacity, g);

    // --- 3. Circulation regime: how many cells, which way the wind -----------------------------
    let omega = if b.rotation_period_s > 0.0 { TAU / b.rotation_period_s } else { 0.0 };
    let coriolis_45 = 2.0 * omega * (45f64.to_radians()).sin();
    // Held & Hou: the cell ends where the thermal Rossby number says the deviation closes the loop.
    // The height is the dynamical depth (at least 1.5 scale heights: Mars's convective layer runs
    // well above its nominal tropopause); the fractional contrast Δ_H ≈ 1/3 for a thin air, shrinking
    // under a massive one (Venus-like airs smooth the contrast away).
    let pressure_bar = b.pressure_pa / 1e5;
    let delta_h = (1.0 / 3.0) / (1.0 + pressure_bar / 10.0);
    let dynamic_height = tropopause.max(1.5 * scale_height);
    let rossby_thermal = if omega > 0.0 && has_atmosphere { g * dynamic_height * delta_h / (omega * omega * b.radius_m * b.radius_m) } else { f64::INFINITY };
    let hadley_edge = if rossby_thermal.is_finite() { ((5.0 * rossby_thermal / 3.0).sqrt().to_degrees()).min(90.0) } else { 90.0 };
    let cells = if hadley_edge >= 85.0 { 1 } else if hadley_edge >= 42.0 { 2 } else { 3 };

    // Equator-to-pole contrast from North's energy balance truncated at P2:
    // T2 = (S0/4)·S2(β)·(1−α)/(B + 6D), ΔT = −1.5·T2. B linearises the thermal emission (Earth ≈ 2),
    // D the meridional diffusion, scaled by the column heat capacity against Earth's.
    let obliquity = b.tilt_deg.to_radians();
    let s2 = (5.0 / 16.0) * (3.0 * obliquity.sin().powi(2) - 2.0);
    let b_lin = 0.55 * 4.0 * SIGMA * t_eq.powi(3);
    let earth_column = 1004.0 * 1.013e5 / 9.82;
    let d_diff = if has_atmosphere { 0.555 * column_heat_capacity / earth_column } else { 0.0 };
    let t2 = (irradiance / 4.0) * s2 * (1.0 - b.bond_albedo) / (b_lin + 6.0 * d_diff);
    let delta_t = (-1.5 * t2).clamp(0.0, (t_surface - 3.0).max(0.0));

    // Thermal wind aloft, then 40 % of it at the surface (friction), capped by the super-rotation
    // bound of slow rotators. L = a·π/4: the span of the gradient.
    let p_tropo = b.pressure_pa * (-tropopause / scale_height.max(1.0)).exp();
    let log_depth = if has_atmosphere { (b.pressure_pa / p_tropo.max(1e-9)).ln().max(0.1) } else { 0.0 };
    let span = b.radius_m * PI / 4.0;
    let superrotation_cap = omega * b.radius_m * 0.707;
    let u_jet_raw = if has_atmosphere && coriolis_45 > 0.0 { r_spec * delta_t / (coriolis_45 * span) * log_depth } else { 0.0 };
    let u_jet = if superrotation_cap > 0.0 { u_jet_raw.min(superrotation_cap) } else { u_jet_raw };
    let v10_scale = (0.4 * u_jet).min(b.extras.wind_scale_max_ms);

    // --- 4. Dust: when the sand flies and how long it stays --------------------------------------
    let (u_star_t, v10_t, settling) = if has_atmosphere {
        let x = &b.extras;
        // Shao & Lu (2000): gravity term plus cohesion, A_N = 0.0123, γ = 1.65e-4 N/m on Earth. The
        // cohesion of the bed drops with gravity (Musiolik 2018, Kruss 2020, parabolic flights): a
        // linear scaling reproduces both Earth's 6–8 m/s and Mars's 25–35 m/s thresholds.
        let cohesion = 1.65e-4 * (g / 9.81).clamp(0.05, 2.0);
        let u_star = (0.0123 * ((x.dust_grain_density_kg_m3 - rho0) * g * x.dust_grain_diameter_m / rho0 + cohesion / (rho0 * x.dust_grain_diameter_m))).sqrt();
        let v10 = u_star / KARMAN * (10.0 / x.roughness_length_m).ln();
        // Stokes settling of a 2 µm grain (air viscosity ~1.8e-5 Pa·s near 300 K, ∝ T^0.7).
        let mu = 1.8e-5 * (t_surface / 293.0).powf(0.7);
        let w = x.dust_grain_density_kg_m3 * g * (2e-6f64).powi(2) / (18.0 * mu);
        (u_star, v10, w * 86_400.0)
    } else {
        (0.0, 0.0, 0.0)
    };

    Climate {
        name: b.name.clone(),
        kind: b.kind,
        seed: b.extras.weather_seed.unwrap_or_else(|| seed_from_name(&b.name)),
        has_atmosphere,
        radius_m: b.radius_m,
        surface_gravity: g,
        rotation_period_s: b.rotation_period_s,
        tidal_locked: b.tidal_locked,
        tilt_deg: b.tilt_deg,
        orbit_period_s: orbit.period_s(),
        molar_mass_kg_mol: mix.molar_mass_kg_mol,
        cp_j_kg_k: mix.cp_j_kg_k,
        gamma,
        greenhouse_fraction: mix.greenhouse_fraction,
        pressure_pa: b.pressure_pa,
        irradiance_w_m2: irradiance,
        bond_albedo: b.bond_albedo,
        t_eq_k: t_eq,
        tau_ir,
        t_surface_k: t_surface,
        t_skin_k: t_skin,
        scale_height_m: scale_height,
        rho0_kg_m3: rho0,
        lapse_dry_k_m: lapse_dry,
        lapse_env_k_m: lapse_env,
        tropopause_m: tropopause,
        sound_speed_m_s: sound_speed,
        radiative_time_s: radiative_time,
        diurnal_amplitude_k: diurnal_amplitude,
        omega_rad_s: omega,
        coriolis_45_s: coriolis_45,
        rossby_thermal,
        hadley_edge_deg: hadley_edge,
        cells,
        delta_t_equator_pole_k: delta_t,
        u_jet_m_s: u_jet,
        v10_scale_m_s: v10_scale,
        superrotation_cap_m_s: superrotation_cap,
        u_star_threshold_m_s: u_star_t,
        v10_threshold_m_s: v10_t,
        settling_2um_m_day: settling,
        extras: b.extras.clone(),
    }
}

/// Half of the day/night swing. The data's day and night temperatures win when present; otherwise
/// the noon forcing against what the ground (thermal inertia) and the air (column heat capacity)
/// can store over a day: ΔT ≈ ΔQ / (I·√ω + c_air·ω + 4σT³).
fn diurnal_amplitude(b: &BodyInputs, irradiance: f64, t_surface: f64, column_heat_capacity: f64, _g: f64) -> f64 {
    if let Some(t) = b.temperatures {
        if t.day_k > t.night_k {
            return 0.5 * (t.day_k - t.night_k);
        }
    }
    if b.rotation_period_s <= 0.0 || b.tidal_locked {
        return 0.0;
    }
    let omega = TAU / b.rotation_period_s;
    let forcing = 0.5 * (1.0 - b.bond_albedo) * irradiance;
    let damping = b.extras.thermal_inertia_tiu * omega.sqrt() + column_heat_capacity * omega + 4.0 * SIGMA * t_surface.powi(3);
    forcing / damping
}

/// Instantaneous insolation factor (0–1 of the irradiance) at latitude `lat_deg`, hour angle `h`
/// (radians, 0 at noon) and declination `decl_deg`: cos of the star's zenith angle, or 0 at night.
pub fn insolation_factor(lat_deg: f64, hour_angle: f64, decl_deg: f64) -> f64 {
    let (lat, decl) = (lat_deg.to_radians(), decl_deg.to_radians());
    (lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos()).max(0.0)
}

/// Daily-mean insolation factor at a latitude for a declination (the classic h0 formula; 0 in the
/// polar night, cos-weighted all day in the polar day).
pub fn daily_mean_insolation_factor(lat_deg: f64, decl_deg: f64) -> f64 {
    let (lat, decl) = (lat_deg.to_radians(), decl_deg.to_radians());
    let x = -lat.tan() * decl.tan();
    let h0 = if x <= -1.0 { PI } else if x >= 1.0 { 0.0 } else { x.acos() };
    ((h0 * lat.sin() * decl.sin() + lat.cos() * decl.cos() * h0.sin()) / PI).max(0.0)
}

/// Annual mean of the daily-mean factor at a latitude, for an obliquity: averaged over the year
/// with the declination sweeping ±obliquity (12 samples).
pub fn annual_mean_insolation_factor(lat_deg: f64, obliquity_deg: f64) -> f64 {
    let n = 12;
    (0..n).map(|k| daily_mean_insolation_factor(lat_deg, obliquity_deg * (TAU * k as f64 / n as f64).sin())).sum::<f64>() / n as f64
}

/// Legendre P2.
fn p2(x: f64) -> f64 {
    0.5 * (3.0 * x * x - 1.0)
}

/// Surface air temperature (K) at a latitude, local solar time and declination: the annual-mean
/// latitude profile (P2), the seasonal departure of the daily insolation (damped by the heat the
/// ground and the air store), and the day/night swing (max two hours after noon).
pub fn surface_temperature_k(c: &Climate, lat_deg: f64, local_solar_hours: f64, decl_deg: f64) -> f64 {
    if c.tidal_locked {
        // A fixed day side and night side: the contrast is spatial, by the angle to the star.
        let (day, night) = (c.t_surface_k + c.diurnal_amplitude_k, c.t_surface_k - c.diurnal_amplitude_k);
        let cos_zenith = insolation_factor(lat_deg, hour_angle_rad(local_solar_hours), decl_deg);
        return night + (day - night) * cos_zenith;
    }
    let x = lat_deg.to_radians().sin();
    let t2 = -c.delta_t_equator_pole_k / 1.5;
    let annual = c.t_surface_k + t2 * p2(x);
    // Season: the daily insolation today against its annual mean here, turned into kelvin by the
    // emission slope B and damped by how slowly the ground + air warm (ratio of the year to the
    // relaxation time of the column).
    let q_today = daily_mean_insolation_factor(lat_deg, decl_deg);
    let q_mean = annual_mean_insolation_factor(lat_deg, c.tilt_deg.min(180.0 - c.tilt_deg).abs());
    let b_lin = 0.55 * 4.0 * SIGMA * c.t_eq_k.powi(3);
    let relax_s = c.radiative_time_s + c.extras.thermal_inertia_tiu * (c.orbit_period_s / TAU).sqrt() / b_lin.max(1e-9);
    let damping = 1.0 / (1.0 + (TAU / c.orbit_period_s.max(1.0) * relax_s).powi(2)).sqrt();
    let seasonal = (1.0 - c.bond_albedo) * c.irradiance_w_m2 * (q_today - q_mean) / b_lin.max(1e-9) * damping;
    // Day/night: a sinusoid lagging noon by two hours, scaled by how high the star climbs here today
    // against the equator at equinox.
    let noon_here = insolation_factor(lat_deg, 0.0, decl_deg);
    let swing = c.diurnal_amplitude_k * noon_here * (hour_angle_rad(local_solar_hours) - hour_angle_rad(14.0)).cos();
    annual + seasonal + swing
}

/// Which wind band a latitude falls in, and the mean surface wind there as (east, north) m/s —
/// east = the direction the body turns toward, north = toward its spin pole. Hadley cell: surface
/// flow back to the equator, bent west by Coriolis (the trades); Ferrel cell: poleward and east
/// (the westerlies); polar cell: easterlies. One-cell worlds (slow rotators) flow gently to the
/// equator everywhere; the deviation shrinks with the Coriolis parameter.
pub fn band_wind(c: &Climate, lat_deg: f64) -> BandWind {
    if !c.has_atmosphere {
        return BandWind { band: Band::Calm, east_m_s: 0.0, north_m_s: 0.0 };
    }
    let toward_equator = if lat_deg >= 0.0 { -1.0 } else { 1.0 };
    let abs_lat = lat_deg.abs();
    // Coriolis turns the meridional flow: full deflection where the parameter is Earth-like, none
    // on a slow rotator (the flow runs straight along the pressure gradient).
    let deflection = (c.coriolis_45_s / 1.0e-4).clamp(0.0, 1.0);
    let (band, factor, meridional_sign, zonal_sign) = match c.cells {
        1 => (Band::SingleCell, 0.5, toward_equator, -1.0),
        2 => {
            if abs_lat < c.hadley_edge_deg {
                (Band::Trades, 0.5, toward_equator, -1.0)
            } else {
                (Band::Westerlies, 1.0, -toward_equator, 1.0)
            }
        }
        _ => {
            if abs_lat < c.hadley_edge_deg {
                (Band::Trades, 0.5, toward_equator, -1.0)
            } else if abs_lat < 60.0 {
                (Band::Westerlies, 1.0, -toward_equator, 1.0)
            } else {
                (Band::PolarEasterlies, 0.6, toward_equator, -1.0)
            }
        }
    };
    // Doldrums: the trades die out within a few degrees of the equator.
    let equator_fade = (abs_lat / 5.0).clamp(0.0, 1.0);
    let speed = c.v10_scale_m_s * factor * if matches!(band, Band::Trades) { equator_fade } else { 1.0 };
    let zonal = speed * deflection * zonal_sign;
    let meridional = speed * (1.0 - 0.7 * deflection) * meridional_sign;
    BandWind { band, east_m_s: zonal, north_m_s: meridional }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Calm,
    SingleCell,
    Trades,
    Westerlies,
    PolarEasterlies,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BandWind {
    pub band: Band,
    pub east_m_s: f64,
    pub north_m_s: f64,
}

/// Air density at an altitude above the reference sphere, kg/m³ (isothermal column).
pub fn air_density(c: &Climate, altitude_m: f64) -> f64 {
    if !c.has_atmosphere {
        return 0.0;
    }
    c.rho0_kg_m3 * (-altitude_m / c.scale_height_m).exp()
}

/// The declination of the star today, degrees, from the body's position on its orbit at `t`.
pub fn declination_at(b: &BodyInputs, t: f64) -> f64 {
    let pos = b.orbit.solver(b.mass_kg).position_at(t);
    // The star is at the origin of a planet's frame: the direction to it is minus the position.
    // (A moon's parent is its planet; its star direction is its planet's, close enough here.)
    declination_deg(b.tilt_deg.to_radians(), pos.scale(-1.0))
}
