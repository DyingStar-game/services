//! Loading the bodies: the celestial data the whole project shares
//! (`services/resourcesDynamic/data/system/<system>.json`, key `structured`) plus the weather's own
//! extras (`data/climate/<system>.json`), joined by body name.

use crate::climate::{BodyInputs, BodyKind, ClimateExtras, OrbitElements, StarInputs, Temperatures};
use crate::constants::{AU_M, EARTH_MASS_KG, G, SOLAR_MASS_KG};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The data writes `null` for an unknown number (a moon's albedo, a resource tonnage): read it as 0.
fn f64_or_zero<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(0.0))
}

#[derive(Debug, Deserialize)]
struct SystemFile {
    structured: Structured,
}

#[derive(Debug, Deserialize)]
struct Structured {
    stars: Vec<StarRecord>,
    planets: Vec<PlanetRecord>,
}

#[derive(Debug, Deserialize)]
struct StarRecord {
    name: String,
    #[serde(rename = "luminosity_W", default, deserialize_with = "f64_or_zero")]
    luminosity_w: f64,
    #[serde(rename = "mass_Sun", default, deserialize_with = "f64_or_zero")]
    mass_sun: f64,
    #[serde(rename = "temp_K", default, deserialize_with = "f64_or_zero")]
    temp_k: f64,
}

#[derive(Debug, Deserialize)]
struct PlanetRecord {
    name: String,
    #[serde(rename = "mass_Me", default, deserialize_with = "f64_or_zero")]
    mass_me: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    radius_km: f64,
    #[serde(rename = "semi_major_AU", default, deserialize_with = "f64_or_zero")]
    semi_major_au: f64,
    #[serde(rename = "periapsis_AU", default, deserialize_with = "f64_or_zero")]
    periapsis_au: f64,
    #[serde(rename = "apoapsis_AU", default, deserialize_with = "f64_or_zero")]
    apoapsis_au: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    inclination_deg: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    ascending_node_deg: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    arg_peri_deg: f64,
    #[serde(rename = "M0_deg", default, deserialize_with = "f64_or_zero")]
    m0_deg: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    rotation_h: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    tilt_deg: f64,
    #[serde(default)]
    tidal_locked: bool,
    #[serde(default, deserialize_with = "f64_or_zero")]
    albedo: f64,
    #[serde(default)]
    atmosphere: Option<AtmosphereRecord>,
    #[serde(default)]
    temperatures: Option<TemperatureRecord>,
    #[serde(default)]
    moons: Vec<MoonRecord>,
}

#[derive(Debug, Deserialize)]
struct MoonRecord {
    name: String,
    #[serde(rename = "mass_Me", default, deserialize_with = "f64_or_zero")]
    mass_me: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    radius_km: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    periapsis_km: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    apoapsis_km: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    inclination_deg: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    ascending_node_deg: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    arg_peri_deg: f64,
    #[serde(rename = "M0_deg", default, deserialize_with = "f64_or_zero")]
    m0_deg: f64,
    #[serde(default, deserialize_with = "f64_or_zero")]
    spin_period_h: f64,
    #[serde(default)]
    spin_locked: bool,
    #[serde(default, deserialize_with = "f64_or_zero")]
    albedo: f64,
    #[serde(default)]
    atmosphere: Option<AtmosphereRecord>,
    #[serde(default)]
    temperatures: Option<TemperatureRecord>,
}

#[derive(Debug, Default, Deserialize)]
struct AtmosphereRecord {
    #[serde(default)]
    gases_pct: BTreeMap<String, f64>,
    #[serde(default)]
    pressure_bar: Option<f64>,
    #[serde(default)]
    meta: Option<AtmosphereMeta>,
}

#[derive(Debug, Default, Deserialize)]
struct AtmosphereMeta {
    #[serde(default, deserialize_with = "f64_or_zero")]
    pressure_bar: f64,
}

#[derive(Debug, Deserialize)]
struct TemperatureRecord {
    #[serde(rename = "T_day_K", default, deserialize_with = "f64_or_zero")]
    day_k: f64,
    #[serde(rename = "T_night_K", default, deserialize_with = "f64_or_zero")]
    night_k: f64,
    #[serde(rename = "T_surface_K", default)]
    surface_k: Option<f64>,
    #[serde(rename = "Teq_K", default, deserialize_with = "f64_or_zero")]
    eq_k: f64,
}

/// The climate extras file: body name (matched without regard to case: the data writes "Sandbox",
/// the celestial service "SandBox") → knobs (any field may be omitted).
pub type ExtrasFile = BTreeMap<String, ClimateExtras>;

/// Parse the system file and the extras, and build the inputs of every body (planets, then their
/// moons). Airless bodies are kept: they have a day and a season, just no weather.
pub fn load(system_json: &str, extras_json: Option<&str>) -> Result<Vec<BodyInputs>, String> {
    let file: SystemFile = serde_json::from_str(system_json).map_err(|e| format!("system file: {e}"))?;
    let extras: ExtrasFile = match extras_json {
        // Keys starting with "_" are comments for humans, not bodies.
        Some(s) => {
            let raw: BTreeMap<String, serde_json::Value> = serde_json::from_str(s).map_err(|e| format!("climate extras: {e}"))?;
            raw.into_iter()
                .filter(|(k, _)| !k.starts_with('_'))
                .map(|(k, v)| serde_json::from_value::<ClimateExtras>(v).map(|x| (k.to_lowercase(), x)).map_err(|e| format!("climate extras `{k}`: {e}")))
                .collect::<Result<_, _>>()?
        }
        None => ExtrasFile::new(),
    };
    let star = file.structured.stars.first().ok_or("system file: no star")?;
    let star_inputs = StarInputs { luminosity_w: star.luminosity_w, mass_kg: star.mass_sun * SOLAR_MASS_KG, temperature_k: star.temp_k };
    let _ = &star.name;
    let mut out = Vec::new();
    for p in &file.structured.planets {
        let mass_kg = p.mass_me * EARTH_MASS_KG;
        let (pressure_pa, gases) = atmosphere_of(p.atmosphere.as_ref());
        out.push(BodyInputs {
            name: p.name.clone(),
            kind: BodyKind::Planet,
            radius_m: p.radius_km * 1e3,
            mass_kg,
            rotation_period_s: p.rotation_h * 3600.0,
            tidal_locked: p.tidal_locked,
            tilt_deg: p.tilt_deg,
            orbit: OrbitElements {
                periapsis_au: p.periapsis_au,
                apoapsis_au: p.apoapsis_au,
                inclination_deg: p.inclination_deg,
                ascending_node_deg: p.ascending_node_deg,
                arg_peri_deg: p.arg_peri_deg,
                mean_anomaly_deg: p.m0_deg,
                primary_mass_kg: star_inputs.mass_kg,
            },
            star_distance_m: p.semi_major_au * AU_M,
            star: star_inputs.clone(),
            pressure_pa,
            gases_pct: gases,
            bond_albedo: p.albedo,
            temperatures: p.temperatures.as_ref().map(temperatures_of),
            extras: extras.get(&p.name.to_lowercase()).cloned().unwrap_or_default(),
        });
        for m in &p.moons {
            let (pressure_pa, gases) = atmosphere_of(m.atmosphere.as_ref());
            out.push(BodyInputs {
                name: m.name.clone(),
                kind: BodyKind::Moon,
                radius_m: m.radius_km * 1e3,
                mass_kg: m.mass_me * EARTH_MASS_KG,
                rotation_period_s: m.spin_period_h * 3600.0,
                tidal_locked: m.spin_locked,
                // The data carries no tilt for moons (import-system.ts: "TODO: Missing data").
                tilt_deg: 0.0,
                orbit: OrbitElements {
                    periapsis_au: m.periapsis_km * 1e3 / AU_M,
                    apoapsis_au: m.apoapsis_km * 1e3 / AU_M,
                    inclination_deg: m.inclination_deg,
                    ascending_node_deg: m.ascending_node_deg,
                    arg_peri_deg: m.arg_peri_deg,
                    mean_anomaly_deg: m.m0_deg,
                    primary_mass_kg: mass_kg,
                },
                star_distance_m: p.semi_major_au * AU_M,
                star: star_inputs.clone(),
                pressure_pa,
                gases_pct: gases,
                bond_albedo: m.albedo,
                temperatures: m.temperatures.as_ref().map(temperatures_of),
                extras: extras.get(&m.name.to_lowercase()).cloned().unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

fn atmosphere_of(a: Option<&AtmosphereRecord>) -> (f64, Vec<(String, f64)>) {
    let Some(a) = a else { return (0.0, vec![]) };
    let bar = a.pressure_bar.or_else(|| a.meta.as_ref().map(|m| m.pressure_bar)).unwrap_or(0.0);
    let gases: Vec<(String, f64)> = a.gases_pct.iter().map(|(k, v)| (k.clone(), *v)).collect();
    (bar * 1e5, gases)
}

fn temperatures_of(t: &TemperatureRecord) -> Temperatures {
    let surface = t.surface_k.unwrap_or_else(|| if t.day_k > 0.0 && t.night_k > 0.0 { 0.5 * (t.day_k + t.night_k) } else { t.eq_k });
    Temperatures { day_k: t.day_k, night_k: t.night_k, surface_k: surface, eq_k: t.eq_k }
}

/// Surface gravity from mass and radius, m/s² — what the game's `surface_gravity` export holds.
pub fn surface_gravity(mass_kg: f64, radius_m: f64) -> f64 {
    G * mass_kg / (radius_m * radius_m)
}
