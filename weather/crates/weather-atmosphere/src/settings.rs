//! Settings of a run, read from TOML files (the service's `data/atmosphere/`): `defaut.toml` holds
//! the model's generic choices (numerics, the shape of the Held-Suarez recipe, the dust physics
//! constants...) and every body file inherits it (`inherit = "defaut.toml"`) and adds what the
//! shared data does not give (a dust sea, a lid, a relief file, worldbuilding decisions). A value
//! written as optional (`Option`) is derived from the body's data when the file leaves it out (see
//! `setup`).
//!
//! Files are merged key by key (a child replaces its parent's value; tables merge, arrays are
//! replaced), then the caller may set any key (`lid.enabled=false`). Unknown keys are errors, so a
//! typo never goes unnoticed. Relative paths written in a file are relative to that file's folder.

use crate::body::{BodyOverrides, DataFiles};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use toml::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Parent file (consumed by the loader, kept for the record).
    #[serde(default)]
    pub inherit: Option<String>,
    /// Name of the body in the system data ("Sandbox", "Gaea", "P5_M2"...).
    #[serde(default)]
    pub body: Option<String>,
    /// Shared data files (offline runs only: the service passes the body it has already loaded).
    #[serde(default)]
    pub data: Option<DataSettings>,
    #[serde(default)]
    pub body_overrides: BodyOverrides,
    pub grid: GridSettings,
    pub numerics: Numerics,
    pub run: RunSettings,
    pub relief: ReliefSettings,
    pub relaxation: RelaxationSettings,
    pub reference: ReferenceSettings,
    pub ground: GroundSettings,
    pub convection: Switch,
    pub dust: DustSettings,
    pub sea: SeaSettings,
    pub lid: LidSettings,
    pub events: Switch,
    pub outputs: OutputSettings,
    pub shallow_water: ShallowWaterSettings,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSettings {
    pub system: PathBuf,
    pub extras: Option<PathBuf>,
}

impl DataSettings {
    pub fn files(&self) -> DataFiles {
        DataFiles { system: self.system.clone(), extras: self.extras.clone() }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSettings {
    pub level: u32,
    pub layers: usize,
    /// Explicit half levels eta = p / p_surface (top 0 to ground 1); default: the level rule.
    pub eta_half: Option<Vec<f64>>,
    /// Top of the terrain-following part of the hybrid coordinate (eta); default: the interior
    /// half level nearest `hybrid_top_factor` x tropopause height.
    pub hybrid_eta_t: Option<f64>,
    /// Level rule: the highest interior half level at `rule_top_factor` x tropopause height,
    /// the others at (j / (N - 1))^`rule_stretch` of it (finer near the ground).
    pub rule_top_factor: f64,
    pub rule_stretch: f64,
    pub hybrid_top_factor: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numerics {
    pub courant: f64,
    pub chunks_per_day: u64,
    pub hyper_order: u32,
    pub tau_wind_hours: f64,
    pub tau_temperature_hours: f64,
    pub tau_ps_hours: f64,
    pub noise_k: f64,
    /// Guard: surface pressure bounds as fractions of the mean surface pressure, largest wind.
    pub guard_ps_min_frac: f64,
    pub guard_ps_max_frac: f64,
    pub guard_max_wind_m_s: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSettings {
    pub spinup_days: f64,
    pub record_days: f64,
    /// PNG tables every N model days of the recorded period (0 = never).
    pub tables_every_days: f64,
    /// Climate record (and log line) every N model days.
    pub diag_every_days: f64,
    /// Restart file every N model days (0 = never).
    pub restart_every_days: f64,
    pub out_dir: Option<PathBuf>,
    pub restart_from: Option<PathBuf>,
    pub img_w: usize,
    pub img_h: usize,
    pub quiet: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReliefSettings {
    /// Equirectangular float32 raster of the relief (m); none = a smooth sphere.
    pub file: Option<PathBuf>,
    pub width: usize,
    pub height: usize,
    pub scale: f64,
    pub smooth_passes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelaxationSettings {
    pub enabled: bool,
    pub seasons: bool,
    /// Equator-to-pole contrast of the equilibrium temperature = factor x the data's contrast.
    pub contrast_factor: f64,
    pub equator_pole_contrast_k: Option<f64>,
    /// Equilibrium temperature at the subsolar ground; default: t_surface + contrast / 3.
    pub t_max_k: Option<f64>,
    pub static_stability_k: f64,
    /// Floor of the equilibrium temperature; default: the data's skin temperature.
    pub t_min_k: Option<f64>,
    /// Slow (free-air) relaxation time; default: the data's radiative time.
    pub slow_time_s: Option<f64>,
    pub fast_time_s: f64,
    pub friction_time_s: f64,
    pub sigma_b: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSettings {
    /// Reference profile: ground temperature (default t_max - contrast / 3), lapse rate (default
    /// the data's environmental lapse rate) and stratosphere (default the skin temperature).
    pub t0_k: Option<f64>,
    pub lapse_k_m: Option<f64>,
    pub t_strat_k: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Switch {
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundSettings {
    pub enabled: bool,
    /// Ground albedo; default: the body's Bond albedo.
    pub albedo: Option<f64>,
    /// Optional ground albedo map (float32 raster, relief size): replaces `albedo` cell by cell.
    #[serde(default)]
    pub albedo_map: Option<PathBuf>,
    /// Effective emissivity of the air for the down-welling infrared; default 1 - exp(-tau_ir).
    pub eps_air: Option<f64>,
    pub thermal_inertia_tiu: Option<f64>,
    pub roughness_m: Option<f64>,
    pub v_floor_m_s: f64,
    pub stable_floor: f64,
    pub sun_lon0_deg: f64,
    /// Test hook: a steady overhead sun of this flux everywhere (W/m^2).
    pub uniform_sun_w_m2: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DustSettings {
    /// Two dust classes (fine, coarse) transported and acted on by the physics.
    pub enabled: bool,
    pub lifting: bool,
    pub radiative: bool,
    /// Lifting threshold on the surface stress (Pa); default rho0 u*_t^2 from the data.
    pub threshold_pa: Option<f64>,
    pub saltation_constant: f64,
    /// Sandblasting efficiency (1/m); default alpha_per_bias x the data's dust lift bias.
    pub alpha_per_m: Option<f64>,
    pub alpha_per_bias: f64,
    pub fine_fraction: f64,
    pub diameters_m: [f64; 2],
    pub grain_density_kg_m3: Option<f64>,
    /// Air viscosity (default 1.8e-5 (T/293)^0.7 Pa s) and molecular mean free path (default
    /// mu / p sqrt(pi R T / 2)).
    pub viscosity_pa_s: Option<f64>,
    pub mean_free_path_m: Option<f64>,
    pub qext: [f64; 2],
    pub deposition_turbulent: f64,
    /// Absorption of the intercepted sunlight: a map of the dust-colour albedo (absorption =
    /// scale x (1 - albedo)), else the provisional biome table, else uniform (default
    /// 1 - ground albedo).
    pub absorption_albedo_map: Option<PathBuf>,
    pub absorption_table: Option<PathBuf>,
    pub absorption_scale: f64,
    pub absorption_uniform: Option<f64>,
    /// Optics of the airborne dust: "colour" (absorption from the colour map, half of the scattered
    /// light down, half up) or "mie" (Mie optics of the grains, imaginary index from the colour by
    /// Hapke's model, delta-Eddington two-stream over the ground).
    #[serde(default)]
    pub optics: DustOptics,
    /// Mie optics: real refractive index of the grains (visible).
    #[serde(default = "default_refractive_index")]
    pub refractive_index_real: f64,
    /// Mie optics: wavelength of the single visible band (m).
    #[serde(default = "default_mie_wavelength")]
    pub mie_wavelength_m: f64,
    /// Mie optics: geometric standard deviation of the lognormal size distribution of each class
    /// (the class diameter is its effective diameter); 1 = one size.
    #[serde(default = "default_mie_sigma_g")]
    pub mie_sigma_g: f64,
    /// Mie optics: grain diameter of the ground (m), used to read the colour map as the absorption
    /// of the material (Hapke's equivalent slab).
    #[serde(default = "default_ground_grain")]
    pub ground_grain_m: f64,
}

/// Optics of the airborne dust (see `DustSettings::optics`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DustOptics {
    #[default]
    Colour,
    Mie,
}

fn default_refractive_index() -> f64 {
    1.77
}
fn default_mie_wavelength() -> f64 {
    0.55e-6
}
fn default_mie_sigma_g() -> f64 {
    1.5
}
fn default_ground_grain() -> f64 {
    100e-6
}

/// Every field defaults to zero / off: a body without this feature only writes `enabled = false`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SeaSettings {
    pub enabled: bool,
    pub z_base_m: f64,
    pub z_top_m: f64,
    pub optical_depth: f64,
    pub fill_days: f64,
    pub excess_days: f64,
    pub clear_days: f64,
    /// Outputs: top of the "above the sea" wind band, and the altitude bins of the profiles.
    pub above_band_top_m: f64,
    pub profile_bottom_m: f64,
    pub profile_step_m: f64,
    pub profile_bins: usize,
}

/// Every field defaults to zero / off: a body without this feature only writes `enabled = false`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LidSettings {
    pub enabled: bool,
    pub z_m: f64,
    pub depth_m: f64,
    pub step_k: f64,
    pub fast_days: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedPoint {
    pub name: String,
    pub lon: f64,
    pub lat: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSettings {
    /// Points of the hourly record (besides the lowest and highest tropical cells).
    pub tracked_points: Vec<TrackedPoint>,
    /// Altitude classes of the summary: basins below [0], [0]..[1], above [2] (m).
    pub altitude_classes_m: [f64; 3],
    /// Region of the passive dust release of the runs without physics: lon min, max, lat min, max.
    pub dust_release: [f64; 4],
    /// Latitudes of the vertical cross-sections (degrees).
    pub section_lats: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShallowWaterSettings {
    /// Mean layer depth (m); default: the scale height.
    pub mean_depth_m: Option<f64>,
    pub u0_m_s: f64,
    pub relief_frac: f64,
    pub tau_hours: f64,
    pub days: f64,
    pub every_days: f64,
}

/// Recursive merge: `top` wins; tables are merged key by key.
fn merge(base: &mut Value, top: Value) {
    match (base, top) {
        (Value::Table(b), Value::Table(t)) => {
            for (k, v) in t {
                match b.get_mut(&k) {
                    Some(bv) if bv.is_table() && v.is_table() => merge(bv, v),
                    _ => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, t) => *b = t,
    }
}

/// Keys that hold file paths: a relative path is made relative to the folder of the file it is
/// written in, so that settings files work wherever the process is started from.
const PATH_KEYS: [(&str, &str); 8] = [
    ("data", "system"),
    ("data", "extras"),
    ("relief", "file"),
    ("ground", "albedo_map"),
    ("dust", "absorption_albedo_map"),
    ("dust", "absorption_table"),
    ("run", "out_dir"),
    ("run", "restart_from"),
];

fn resolve_paths(v: &mut Value, dir: &Path) {
    for (table, key) in PATH_KEYS {
        if let Some(Value::String(s)) = v.get_mut(table).and_then(|t| t.get_mut(key)) {
            let p = Path::new(s.as_str());
            if p.is_relative() {
                *s = dir.join(p).to_string_lossy().into_owned();
            }
        }
    }
}

fn load_merged(path: &Path, depth: usize) -> Result<Value, String> {
    if depth > 8 {
        return Err(format!("{}: inheritance chain too long", path.display()));
    }
    let txt = std::fs::read_to_string(path).map_err(|e| format!("settings file {}: {e}", path.display()))?;
    let mut v: Value = toml::from_str(&txt).map_err(|e| format!("settings file {}: {e}", path.display()))?;
    resolve_paths(&mut v, path.parent().unwrap_or(Path::new(".")));
    match v.get("inherit").and_then(|x| x.as_str()) {
        Some(parent) => {
            let pp = path.parent().unwrap_or(Path::new(".")).join(parent);
            let mut base = load_merged(&pp, depth + 1)?;
            merge(&mut base, v);
            Ok(base)
        }
        None => Ok(v),
    }
}

/// Sets `a.b.c = value` in a merged table (the value is parsed as TOML; a bare word is a string).
fn set_key(root: &mut Value, key: &str, raw: &str) -> Result<(), String> {
    let val: Value = match toml::from_str::<toml::Table>(&format!("x = {raw}")) {
        Ok(t) => t["x"].clone(),
        Err(_) => Value::String(raw.to_string()),
    };
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = root;
    for p in &parts[..parts.len() - 1] {
        let t = cur.as_table_mut().ok_or_else(|| format!("--set {key}: `{p}` is not a table"))?;
        cur = t.entry(p.to_string()).or_insert_with(|| Value::Table(toml::Table::new()));
    }
    let t = cur.as_table_mut().ok_or_else(|| format!("--set {key}: not a table"))?;
    t.insert(parts[parts.len() - 1].to_string(), val);
    Ok(())
}

impl Settings {
    /// Loads a settings file (with its parents) and applies `key=value` overrides.
    pub fn load(path: &Path, sets: &[(String, String)]) -> Result<Settings, String> {
        let mut v = load_merged(path, 0)?;
        for (k, x) in sets {
            set_key(&mut v, k, x)?;
        }
        v.try_into::<Settings>().map_err(|e| format!("settings {}: {e}", path.display()))
    }

    /// The settings file of a body: `<dir>/<name in lower case>.toml` if it exists, else
    /// `<dir>/defaut.toml` (every derived value then comes from the data).
    pub fn file_for_body(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(format!("{}.toml", name.to_lowercase()));
        if p.exists() {
            p
        } else {
            dir.join("defaut.toml")
        }
    }

    /// Model time in days -> chunks.
    pub fn chunks(&self, days: f64) -> u64 {
        (days * self.numerics.chunks_per_day as f64).round() as u64
    }
}
