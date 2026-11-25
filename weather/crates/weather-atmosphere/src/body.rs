//! One body of the system as the atmosphere model sees it, built from the data the whole project
//! shares: the celestial system file (`resourcesDynamic/data/system/<system>.json`) and the climate
//! extras of the weather service, through the weather service's own library (`weather_core::bodies`
//! and `weather_core::climate::derive`). Nothing here is specific to a body: every number comes
//! from the data, or from an explicit override in the body's settings file (section
//! `[body_overrides]`), which is how the step-4 Sandbox numbers are pinned for the regression test.

use serde::Deserialize;
use std::f64::consts::PI;
use weather_core::climate::{BodyKind, Climate};

/// Optional overrides of numbers the data gives (each one replaces the data's value; the
/// quantities computed from it here, the specific gas constant, kappa, Omega and the solar day,
/// follow). Meant for regressions and for worldbuilding trials, not for everyday runs.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyOverrides {
    pub radius_m: Option<f64>,
    pub gravity_m_s2: Option<f64>,
    pub rotation_period_s: Option<f64>,
    pub solar_day_s: Option<f64>,
    pub year_s: Option<f64>,
    pub tilt_deg: Option<f64>,
    pub molar_mass_kg_mol: Option<f64>,
    pub gas_constant_j_mol_k: Option<f64>,
    pub cp_j_kg_k: Option<f64>,
    pub surface_pressure_pa: Option<f64>,
    pub t_surface_k: Option<f64>,
    pub irradiance_w_m2: Option<f64>,
    pub radiative_time_s: Option<f64>,
    pub bond_albedo: Option<f64>,
    pub thermal_inertia_tiu: Option<f64>,
    pub dust_grain_density_kg_m3: Option<f64>,
    pub roughness_length_m: Option<f64>,
}

/// Composition-derived properties of the air.
#[derive(Clone, Debug, PartialEq)]
pub struct Air {
    /// Mean molar mass (kg/mol), from the gas mixture.
    pub molar_mass: f64,
    /// Specific heat at constant pressure (J/kg/K), mass-weighted over the mixture.
    pub cp: f64,
    /// Molar gas constant used (J/mol/K).
    pub r_gas: f64,
    /// Specific gas constant R / M (J/kg/K) and kappa = R / cp.
    pub rd: f64,
    pub kappa: f64,
}

/// Everything the atmosphere model needs about one body, in SI units.
#[derive(Clone, Debug)]
pub struct Body {
    pub name: String,
    pub kind: BodyKind,
    pub radius_m: f64,
    pub gravity: f64,
    /// Sidereal rotation period (s) and the rotation rate 2 pi / period (rad/s).
    pub rotation_period_s: f64,
    pub omega: f64,
    /// Length of the year (s): the body's own orbit around the star for a planet, its planet's
    /// orbit for a moon (the seasons and the solar day follow the star, not the planet).
    pub year_s: f64,
    /// Axial tilt (degrees, folded into 0..90 when the spin is retrograde).
    pub tilt_deg: f64,
    pub retrograde: bool,
    /// Solar day (s), signed: positive when the star crosses the sky westward. Infinite for a
    /// body locked to its star (the star stands still in its sky).
    pub solar_day_s: f64,
    /// Time unit of the model (s): the solar day when it is shorter than the year, else the
    /// sidereal rotation (a body locked to its star has no solar day to count with).
    pub day_s: f64,
    pub air: Air,
    /// Mean surface pressure (Pa): the reference pressure of the model.
    pub p_surface: f64,
    /// Climate of the data (weather_core::climate::derive), kept whole for the derived settings.
    pub climate: Climate,
    /// Annual-mean irradiance at the top of the air (W/m^2), used as the overhead flux.
    pub irradiance: f64,
    pub radiative_time_s: f64,
    pub bond_albedo: f64,
    pub t_surface_k: f64,
    pub thermal_inertia_tiu: f64,
    pub dust_grain_density: f64,
    pub roughness_length_m: f64,
}

/// Paths of the shared data files.
#[derive(Clone, Debug)]
pub struct DataFiles {
    pub system: std::path::PathBuf,
    pub extras: Option<std::path::PathBuf>,
}

/// Names of every body of the system file (planets then their moons), with a flag for an air.
pub fn list_bodies(files: &DataFiles) -> Result<Vec<(String, bool)>, String> {
    let (inputs, _) = load_inputs(files)?;
    Ok(inputs.iter().map(|b| (b.name.clone(), b.pressure_pa > 0.0)).collect())
}

fn load_inputs(files: &DataFiles) -> Result<(Vec<weather_core::climate::BodyInputs>, String), String> {
    let system = std::fs::read_to_string(&files.system).map_err(|e| format!("system file {}: {e}", files.system.display()))?;
    let extras = match &files.extras {
        Some(p) => Some(std::fs::read_to_string(p).map_err(|e| format!("climate extras {}: {e}", p.display()))?),
        None => None,
    };
    let inputs = weather_core::bodies::load(&system, extras.as_deref())?;
    Ok((inputs, system))
}

impl Body {
    /// Loads the body called `name` (case-insensitive) from the data and applies the overrides.
    /// A body without an atmosphere is refused: there is nothing for this model to simulate.
    pub fn load(files: &DataFiles, name: &str, ov: &BodyOverrides) -> Result<Body, String> {
        let (inputs, _) = load_inputs(files)?;
        // Moons follow their planet in the file: remember the last planet's year.
        let mut parent_year = None;
        let mut found = None;
        for b in &inputs {
            if b.kind == BodyKind::Planet {
                parent_year = Some(b.orbit.solver(b.mass_kg).period_s());
            }
            if b.name.eq_ignore_ascii_case(name) {
                found = Some((b.clone(), parent_year));
                break;
            }
        }
        let Some((inp, parent_year)) = found else {
            let names: Vec<String> = inputs.iter().map(|b| b.name.clone()).collect();
            return Err(format!("no body called `{name}` in {} (bodies: {})", files.system.display(), names.join(", ")));
        };
        let climate = weather_core::climate::derive(&inp);
        Body::from_derived(climate, inp.kind, parent_year, ov)
    }

    /// Like `from_climate`, but refuses a body without an atmosphere (nothing to simulate).
    pub fn from_derived(climate: Climate, kind: BodyKind, planet_year_s: Option<f64>, ov: &BodyOverrides) -> Result<Body, String> {
        if !climate.has_atmosphere {
            return Err(format!(
                "`{}` has no atmosphere (surface pressure 0 in the system data): the atmosphere model does not apply to it",
                climate.name
            ));
        }
        Ok(Body::from_climate(climate, kind, planet_year_s, ov))
    }

    /// Builds the model's view of a derived climate. `planet_year_s` is the year of the planet
    /// (for a moon, its planet's; for a planet, its own).
    pub fn from_climate(climate: Climate, kind: BodyKind, planet_year_s: Option<f64>, ov: &BodyOverrides) -> Body {
        let pick = |o: Option<f64>, d: f64| o.unwrap_or(d);
        let radius_m = pick(ov.radius_m, climate.radius_m);
        let gravity = pick(ov.gravity_m_s2, climate.surface_gravity);
        let rotation_period_s = pick(ov.rotation_period_s, climate.rotation_period_s);
        let data_year = match kind {
            BodyKind::Planet => climate.orbit_period_s,
            BodyKind::Moon => planet_year_s.unwrap_or(climate.orbit_period_s),
        };
        let year_s = pick(ov.year_s, data_year);
        let raw_tilt = pick(ov.tilt_deg, climate.tilt_deg);
        let retrograde = raw_tilt > 90.0;
        let tilt_deg = if retrograde { 180.0 - raw_tilt } else { raw_tilt };
        let omega = if rotation_period_s > 0.0 { 2.0 * PI / rotation_period_s } else { 0.0 };
        // The star goes round the sky at the rotation rate minus the orbital rate (plus it for a
        // retrograde spin): 1 / solar day = 1 / sidereal day -/+ 1 / year.
        let solar_day_s = ov.solar_day_s.unwrap_or_else(|| {
            if rotation_period_s <= 0.0 {
                return f64::INFINITY;
            }
            let n = 1.0 / rotation_period_s + if retrograde { 1.0 / year_s } else { -1.0 / year_s };
            if n.abs() * year_s < 1.0 {
                // Locked to the star (less than one solar day per year): the star stands still.
                f64::INFINITY
            } else {
                1.0 / n
            }
        });
        let day_s = if solar_day_s.is_finite() { solar_day_s.abs() } else { rotation_period_s.abs().max(1.0) };
        let molar_mass = pick(ov.molar_mass_kg_mol, climate.molar_mass_kg_mol);
        let r_gas = pick(ov.gas_constant_j_mol_k, weather_core::constants::R_GAS);
        let cp = pick(ov.cp_j_kg_k, climate.cp_j_kg_k);
        let rd = r_gas / molar_mass;
        let air = Air { molar_mass, cp, r_gas, rd, kappa: rd / cp };
        Body {
            name: climate.name.clone(),
            kind,
            radius_m,
            gravity,
            rotation_period_s,
            omega,
            year_s,
            tilt_deg,
            retrograde,
            solar_day_s,
            day_s,
            air,
            p_surface: pick(ov.surface_pressure_pa, climate.pressure_pa),
            irradiance: pick(ov.irradiance_w_m2, climate.irradiance_w_m2),
            radiative_time_s: pick(ov.radiative_time_s, climate.radiative_time_s),
            bond_albedo: pick(ov.bond_albedo, climate.bond_albedo),
            t_surface_k: pick(ov.t_surface_k, climate.t_surface_k),
            thermal_inertia_tiu: pick(ov.thermal_inertia_tiu, climate.extras.thermal_inertia_tiu),
            dust_grain_density: pick(ov.dust_grain_density_kg_m3, climate.extras.dust_grain_density_kg_m3),
            roughness_length_m: pick(ov.roughness_length_m, climate.extras.roughness_length_m),
            climate,
        }
    }

    /// Surface air density (kg/m^3) at the mean pressure and temperature.
    pub fn rho0(&self) -> f64 {
        self.p_surface / (self.air.rd * self.t_surface_k)
    }

    /// Scale height (m) at the mean surface temperature, with the air actually used.
    pub fn scale_height(&self) -> f64 {
        self.air.rd * self.t_surface_k / self.gravity
    }

    /// One-paragraph summary for logs.
    pub fn describe(&self) -> String {
        let c = &self.climate;
        format!(
            "{} ({:?}): radius {:.0} km, g {:.4} m/s2, sidereal rotation {:.0} s, solar day {:.0} s (model day {:.0} s), year {:.4e} s, tilt {:.1} deg; \
             air M {:.4} g/mol, cp {:.2} J/kg/K, R {:.3} J/kg/K, ps {:.0} Pa, Ts {:.1} K, H {:.0} m, tropopause {:.0} m, \
             equator-pole contrast {:.1} K, irradiance {:.1} W/m2, albedo {:.3}, radiative time {:.3e} s, thermal inertia {:.0} TIU",
            self.name,
            self.kind,
            self.radius_m / 1e3,
            self.gravity,
            self.rotation_period_s,
            self.solar_day_s,
            self.day_s,
            self.year_s,
            self.tilt_deg,
            self.air.molar_mass * 1e3,
            self.air.cp,
            self.air.rd,
            self.p_surface,
            self.t_surface_k,
            c.scale_height_m,
            c.tropopause_m,
            c.delta_t_equator_pole_k,
            self.irradiance,
            self.bond_albedo,
            self.radiative_time_s,
            self.thermal_inertia_tiu
        )
    }
}

/// Seasonal cycle: the latitude of the subsolar point, `tilt sin(2 pi t / year)` (zero at t = 0,
/// rising: northern spring). Without an orbital ephemeris, t = 0 is the northern spring equinox.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seasons {
    pub tilt_deg: f64,
    pub year_s: f64,
}

impl Seasons {
    /// Subsolar latitude (radians) at time t (s).
    pub fn subsolar_lat(&self, t: f64) -> f64 {
        (self.tilt_deg * (2.0 * PI * t / self.year_s).sin()).to_radians()
    }
}
