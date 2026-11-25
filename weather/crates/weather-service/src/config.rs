//! Configuration from the environment, with defaults that run in the dev cluster.

use std::env;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    /// One port for everything: `/ws` (ds_bridge), `/health`, `/metrics`, `/api/v1/...`.
    pub port: u16,
    /// The celestial data file shared with resourcesDynamic (copied into the image).
    pub system_json: String,
    /// The weather's own knobs per body.
    pub climate_json: String,
    /// Internal name of the system in the data ("tarsis").
    pub system_name: String,
    /// Base URL of the celestial service's HTTP API, to resolve the planets' Horizon uuids.
    pub celestial_http: String,
    /// How often the simulation re-evaluates and publishes, seconds.
    pub tick_s: f64,
    /// Spacing of the keyframes a system carries, seconds, and how many.
    pub keyframe_dt_s: f64,
    pub keyframes: usize,
    /// A system is re-sent at most every this many seconds unless its intensity moved by more
    /// than 5 % (the keyframes carry it in between).
    pub update_every_s: f64,
    /// The stateful atmosphere model (off by default).
    pub atmosphere: AtmosphereConfig,
}

/// The atmosphere model (crate `weather-atmosphere`) runs only for the bodies listed in
/// `ATMOSPHERE_BODIES`; with the list empty (the default) the service is exactly the pure
/// (data, seed, time) service. The model never replaces the synoptic generator: it runs beside it.
#[derive(Clone, Debug)]
pub struct AtmosphereConfig {
    /// Body names (case-insensitive), from `ATMOSPHERE_BODIES` = "Sandbox,Gaea".
    pub bodies: Vec<String>,
    /// Settings files and rasters (`defaut.toml`, `<body>.toml`, `*.f32`).
    pub data_dir: PathBuf,
    /// Where checkpoints go: PostgreSQL when `DATABASE_URL` is set, else files under
    /// `ATMOSPHERE_STATE_DIR` (one folder per body; a persistent volume in a cluster). With
    /// neither, the atmosphere mode refuses to start (the rest of the service runs).
    pub database_url: Option<Secret>,
    pub state_dir: Option<PathBuf>,
    /// Threads of each body's model (1 to 4).
    pub threads: usize,
    /// A checkpoint at most every this many wall-clock seconds (and only after a healthy chunk).
    pub checkpoint_every_s: f64,
    /// Overrides the settings' spin-up length (model days) of a cold start: a short value is for
    /// smoke tests only.
    pub spinup_days: Option<f64>,
    /// After a long stop, catch up at most this many model days; older time is skipped.
    pub max_catchup_days: f64,
    /// Extra settings overrides, `ATMOSPHERE_SET` = "grid.level=4;lid.enabled=false".
    pub sets: Vec<(String, String)>,
}

/// A value that must never reach the logs (the database URL carries a password): the config is
/// logged whole at start.
#[derive(Clone)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<set>")
    }
}

impl AtmosphereConfig {
    pub fn enabled(&self) -> bool {
        !self.bodies.is_empty()
    }
}

impl Config {
    pub fn from_env() -> Self {
        let get = |k: &str, d: &str| env::var(k).unwrap_or_else(|_| d.to_string());
        let atmosphere = AtmosphereConfig {
            bodies: get("ATMOSPHERE_BODIES", "").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            data_dir: PathBuf::from(get("ATMOSPHERE_DATA_DIR", "data/atmosphere")),
            database_url: env::var("DATABASE_URL").ok().filter(|s| !s.trim().is_empty()).map(Secret),
            state_dir: env::var("ATMOSPHERE_STATE_DIR").ok().filter(|s| !s.trim().is_empty()).map(PathBuf::from),
            threads: get("ATMOSPHERE_THREADS", "2").parse().unwrap_or(2usize).clamp(1, 4),
            checkpoint_every_s: get("ATMOSPHERE_CHECKPOINT_S", "300").parse().unwrap_or(300.0f64).max(10.0),
            spinup_days: env::var("ATMOSPHERE_SPINUP_DAYS").ok().and_then(|v| v.parse().ok()),
            max_catchup_days: get("ATMOSPHERE_MAX_CATCHUP_DAYS", "30").parse().unwrap_or(30.0),
            sets: get("ATMOSPHERE_SET", "")
                .split(';')
                .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .filter(|(k, _)| !k.is_empty())
                .collect(),
        };
        Self {
            port: get("PORT", "9400").parse().unwrap_or(9400),
            system_json: get("SYSTEM_JSON", "data/system/tarsis.json"),
            climate_json: get("CLIMATE_JSON", "data/climate/tarsis.json"),
            system_name: get("SYSTEM_NAME", "tarsis"),
            celestial_http: get("CELESTIAL_HTTP", "http://service-resourcesdynamic:3001"),
            tick_s: get("TICK_S", "30").parse().unwrap_or(30.0),
            keyframe_dt_s: get("KEYFRAME_DT_S", "600").parse().unwrap_or(600.0),
            keyframes: get("KEYFRAMES", "6").parse().unwrap_or(6),
            update_every_s: get("UPDATE_EVERY_S", "120").parse().unwrap_or(120.0),
            atmosphere,
        }
    }
}
