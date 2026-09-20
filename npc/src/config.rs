use std::env;

/// Service configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// WebSocket URL of the Horizon server.
    pub horizon_url: String,
    /// Bind address for the admin REST API (e.g. `0.0.0.0:9400`).
    pub admin_bind: String,
    /// Path to the YAML roster of NPCs loaded at startup.
    pub roster_path: String,
    /// Path to the text file containing the pool of NPC display names.
    pub names_path: String,
    /// Interval (ms) between action-scheduler ticks.
    pub tick_interval_ms: u64,
    /// Settling delay (ms) after a Horizon session is established before the
    /// NPC runs any action. Inbound frames are still applied to the WorldView
    /// during this window, so actions start with a populated world.
    pub action_startup_delay_ms: u64,
    /// Initial reconnect backoff (ms) for the Horizon WS client.
    pub reconnect_backoff_initial_ms: u64,
    /// Maximum reconnect backoff (ms).
    pub reconnect_backoff_max_ms: u64,
    /// HMAC-SHA256 secret used to sign NPC JWTs. Must match what Horizon expects.
    pub jwt_secret: String,

    /// Base URL of the resourcesDynamic REST API (source of per-planet rotation
    /// periods), e.g. `http://host:3001`.
    pub resources_dynamic_http: String,
    /// Default planet (its `internal_name` or `name`) for NPCs whose
    /// `place_where_work.planet` is unset. Resolves the rotation period.
    pub planet_internal_name: String,
    /// Rotation period (hours) used when resourcesDynamic is unreachable or the
    /// planet is unknown.
    pub rotation_h_fallback: f64,
    /// How often (seconds) to refresh the planet catalogue from resourcesDynamic.
    pub clock_refresh_secs: u64,

    /// Number of planetary days in the shift rota (the spreadsheet has 5).
    pub rota_days: u32,
    /// Aligns which rota day "day 0" lands on.
    pub rota_epoch_offset_days: i64,
    /// Path to the shift-schedules YAML.
    pub schedules_path: String,

    /// Test hook: pin planetary time to this exact Unix second.
    pub clock_override_unix: Option<f64>,
    /// Test hook: speed up planetary time (`1.0` = real time).
    pub clock_time_scale: f64,
}

impl Config {
    pub fn from_env() -> Self {
        let horizon_url = env::var("HORIZON_URL")
            .unwrap_or_else(|_| "ws://server.dyingstar-game.space:7040".to_string());

        let admin_bind =
            env::var("ADMIN_BIND").unwrap_or_else(|_| "0.0.0.0:9400".to_string());

        let roster_path =
            env::var("ROSTER_PATH").unwrap_or_else(|_| "config/npcs.yaml".to_string());

        let names_path =
            env::var("NAMES_PATH").unwrap_or_else(|_| "config/names.txt".to_string());

        let tick_interval_ms = env::var("TICK_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);

        let action_startup_delay_ms = env::var("ACTION_STARTUP_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000);

        let reconnect_backoff_initial_ms = env::var("RECONNECT_BACKOFF_INITIAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_000);

        let reconnect_backoff_max_ms = env::var("RECONNECT_BACKOFF_MAX_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30_000);

        let jwt_secret = env::var("JWT_SECRET")
            .unwrap_or_else(|_| "changeme".to_string());

        let resources_dynamic_http = env::var("RESOURCES_DYNAMIC_HTTP")
            .unwrap_or_else(|_| "http://localhost:3001".to_string());

        let planet_internal_name =
            env::var("PLANET_INTERNAL_NAME").unwrap_or_else(|_| "tarsis_4".to_string());

        let rotation_h_fallback = env::var("ROTATION_H_FALLBACK")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25.0);

        let clock_refresh_secs = env::var("CLOCK_REFRESH_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3_600);

        let rota_days = env::var("ROTA_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let rota_epoch_offset_days = env::var("ROTA_EPOCH_OFFSET_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let schedules_path =
            env::var("SCHEDULES_PATH").unwrap_or_else(|_| "config/schedules.yaml".to_string());

        let clock_override_unix = env::var("CLOCK_OVERRIDE_UNIX")
            .ok()
            .and_then(|v| v.parse().ok());

        let clock_time_scale = env::var("CLOCK_TIME_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);

        Self {
            horizon_url,
            admin_bind,
            roster_path,
            names_path,
            tick_interval_ms,
            action_startup_delay_ms,
            reconnect_backoff_initial_ms,
            reconnect_backoff_max_ms,
            jwt_secret,
            resources_dynamic_http,
            planet_internal_name,
            rotation_h_fallback,
            clock_refresh_secs,
            rota_days,
            rota_epoch_offset_days,
            schedules_path,
            clock_override_unix,
            clock_time_scale,
        }
    }
}
