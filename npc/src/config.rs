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
        }
    }
}
