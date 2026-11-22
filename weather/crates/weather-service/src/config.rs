//! Configuration from the environment, with defaults that run in the dev cluster.

use std::env;

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
}

impl Config {
    pub fn from_env() -> Self {
        let get = |k: &str, d: &str| env::var(k).unwrap_or_else(|_| d.to_string());
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
        }
    }
}
