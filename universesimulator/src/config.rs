use std::env;

use crate::error::{AppError, Result};

/// How outbound writes to the other services are performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalMode {
    /// Real HTTP calls to social, inventory, mission, economie and persistence.
    Live,
    /// In-memory stand-in for every service (local development without the other services).
    DryRun,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// PostgreSQL connection URL.
    pub database_url: String,
    /// Bind address for the REST API (e.g. `0.0.0.0:9600`).
    pub bind: String,
    /// `production` disables the dev bypasses.
    pub app_env: String,

    // ── Simulation ──
    pub sim_enabled: bool,
    pub sim_tick_seconds: u64,
    /// Game hours elapsed per real hour.
    pub sim_time_scale: f64,
    pub sim_leg_max_attempts: i32,
    /// Minutes between two refreshes of the political cache.
    pub sim_political_refresh_minutes: i64,
    /// Days of stock and price history kept.
    pub sim_history_days: i64,

    // ── Inbound auth (ADR 0011) ──
    pub oidc_issuer: String,
    pub oidc_jwks_url: String,
    /// Audience of player tokens (optional check).
    pub oidc_audience: Option<String>,
    /// Audience of service-account tokens.
    pub oidc_service_audience: String,
    pub internal_service_clients: Vec<String>,
    pub internal_api_key: Option<String>,
    pub auth_dev_bypass: bool,
    pub internal_dev_bypass: bool,

    // ── Outbound ──
    pub external_mode: ExternalMode,
    pub oidc_token_url: String,
    pub service_client_id: String,
    pub service_client_secret: Option<String>,
    /// `X-Internal-Key` sent to the other services in dev when no secret is configured.
    pub outbound_internal_key: Option<String>,
    pub social_api_url: String,
    pub inventory_api_url: String,
    pub mission_api_url: String,
    pub economie_api_url: String,
    pub persistence_api_url: String,
    /// Check that warehouses exist in persistence on creation.
    pub persistence_validate: bool,
    pub http_timeout_ms: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let oidc_issuer = env::var("OIDC_ISSUER")
            .unwrap_or_else(|_| "http://keycloak:8080/realms/dyingstar".into());
        let external_mode = match env::var("EXTERNAL_MODE").as_deref() {
            Ok("dry_run") => ExternalMode::DryRun,
            Ok("live") | Err(_) => ExternalMode::Live,
            Ok(other) => {
                return Err(AppError::Config(format!(
                    "EXTERNAL_MODE must be live or dry_run, got {other}"
                )))
            }
        };
        let cfg = Self {
            database_url: require("DATABASE_URL")?,
            bind: env::var("BIND").unwrap_or_else(|_| "0.0.0.0:9600".into()),
            app_env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            sim_enabled: flag("SIM_ENABLED", true),
            sim_tick_seconds: parsed("SIM_TICK_SECONDS", 300),
            sim_time_scale: parsed("SIM_TIME_SCALE", 1.0),
            sim_leg_max_attempts: parsed("SIM_LEG_MAX_ATTEMPTS", 3),
            sim_political_refresh_minutes: parsed("SIM_POLITICAL_REFRESH_MINUTES", 15),
            sim_history_days: parsed("SIM_HISTORY_DAYS", 14),
            oidc_jwks_url: env::var("OIDC_JWKS_URL")
                .unwrap_or_else(|_| format!("{oidc_issuer}/protocol/openid-connect/certs")),
            oidc_token_url: env::var("OIDC_TOKEN_URL")
                .unwrap_or_else(|_| format!("{oidc_issuer}/protocol/openid-connect/token")),
            oidc_issuer,
            oidc_audience: optional("OIDC_AUDIENCE"),
            oidc_service_audience: env::var("OIDC_SERVICE_AUDIENCE")
                .unwrap_or_else(|_| "universesimulator-api".into()),
            internal_service_clients: env::var("INTERNAL_SERVICE_CLIENTS")
                .unwrap_or_else(|_| "svc-game,svc-admin".into())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            internal_api_key: optional("INTERNAL_API_KEY"),
            auth_dev_bypass: flag("AUTH_DEV_BYPASS", false),
            internal_dev_bypass: flag("INTERNAL_DEV_BYPASS", false),
            external_mode,
            service_client_id: env::var("SERVICE_CLIENT_ID")
                .unwrap_or_else(|_| "svc-universesimulator".into()),
            service_client_secret: optional("SERVICE_CLIENT_SECRET"),
            outbound_internal_key: optional("OUTBOUND_INTERNAL_API_KEY"),
            social_api_url: env::var("SOCIAL_API_URL").unwrap_or_else(|_| "http://social:3000".into()),
            inventory_api_url: env::var("INVENTORY_API_URL")
                .unwrap_or_else(|_| "http://inventory:3000".into()),
            mission_api_url: env::var("MISSION_API_URL").unwrap_or_else(|_| "http://mission:3000".into()),
            economie_api_url: env::var("ECONOMIE_API_URL")
                .unwrap_or_else(|_| "http://economie:3000".into()),
            persistence_api_url: env::var("PERSISTENCE_API_URL")
                .unwrap_or_else(|_| "http://service-persistence:3001".into()),
            persistence_validate: flag("PERSISTENCE_VALIDATE", true),
            http_timeout_ms: parsed("HTTP_TIMEOUT_MS", 5000),
        };
        if cfg.is_production() && (cfg.auth_dev_bypass || cfg.internal_dev_bypass) {
            return Err(AppError::Config("dev bypasses are refused in production".into()));
        }
        Ok(cfg)
    }

    pub fn is_production(&self) -> bool {
        self.app_env == "production"
    }
}

fn require(key: &str) -> Result<String> {
    env::var(key).map_err(|_| AppError::Config(format!("{key} is required")))
}

fn optional(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn flag(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(default)
}

fn parsed<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
