use std::env;

/// Service configuration loaded from environment variables.
#[derive(Clone)]
pub struct Config {
    /// WebSocket URL of the Horizon server.
    pub horizon_url: String,
    /// Bind address for the admin REST API (e.g. `0.0.0.0:9400`).
    pub admin_bind: String,
    /// PostgreSQL URL of the NPC database (roster, schedules, tasks, events).
    pub database_url: String,
    /// Delay (ms) between two consecutive spawns, so ramping up many NPCs
    /// doesn't open all their Horizon sessions in the same instant.
    pub spawn_stagger_ms: u64,
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

    /// Keycloak base URL reachable from the service (e.g. `http://keycloak:8080`),
    /// for the token endpoint and the Admin REST API.
    pub keycloak_url: String,
    /// Realm of the NPC accounts.
    pub keycloak_realm: String,
    /// Confidential client NPCs log in with (password grant).
    pub npc_login_client_id: String,
    pub npc_login_client_secret: String,
    /// Service account of this service (`svc-npc`): manages the NPC users and
    /// reads universesimulator.
    pub service_client_id: String,
    pub service_client_secret: String,
    /// Base64 of the 32-byte AES key sealing NPC passwords in the database.
    pub npc_password_key: String,

    /// Issuer and JWKS of the tokens accepted on the admin API.
    pub oidc_issuer: String,
    pub oidc_jwks_url: String,
    /// Expected `aud` of admin tokens; unset = not checked.
    pub oidc_audience: Option<String>,
    /// Dev only: accept `X-Player-Id` / `X-Player-Roles` instead of a token.
    pub auth_dev_bypass: bool,
    /// `production` refuses the dev bypass.
    pub app_env: String,

    /// universesimulator base URL (without `/api`); finds a village's main
    /// warehouse for its storekeepers. Empty = not used.
    pub universesimulator_url: String,

    /// Days of NPC events kept in the database.
    pub events_retention_days: u32,
    /// Log lines kept in memory per NPC for the admin panel.
    pub log_buffer_lines: usize,
    /// Role templates (storekeeper, villager).
    pub roles_path: String,

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
    /// Path to the shift-schedules YAML, imported into the database on first
    /// boot and on demand (`POST /api/admin/schedule/import`).
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

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://npc:npc@localhost:5432/npc".to_string());

        let spawn_stagger_ms = env::var("SPAWN_STAGGER_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);

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

        let keycloak_url = env::var("KEYCLOAK_URL")
            .unwrap_or_else(|_| "http://keycloak:8080".to_string())
            .trim_end_matches('/')
            .to_string();
        let keycloak_realm = env::var("KEYCLOAK_REALM").unwrap_or_else(|_| "dyingstar".to_string());
        let npc_login_client_id =
            env::var("NPC_LOGIN_CLIENT_ID").unwrap_or_else(|_| "npc-login".to_string());
        let npc_login_client_secret = env::var("NPC_LOGIN_CLIENT_SECRET").unwrap_or_default();
        let service_client_id =
            env::var("SERVICE_CLIENT_ID").unwrap_or_else(|_| "svc-npc".to_string());
        let service_client_secret = env::var("SERVICE_CLIENT_SECRET").unwrap_or_default();
        let npc_password_key = env::var("NPC_PASSWORD_KEY").unwrap_or_default();

        // An empty variable (a Helm value left blank) counts as unset.
        let non_empty = |name: &str| env::var(name).ok().filter(|v| !v.trim().is_empty());
        let oidc_issuer = non_empty("OIDC_ISSUER")
            .unwrap_or_else(|| format!("{keycloak_url}/realms/{keycloak_realm}"));
        let oidc_jwks_url = non_empty("OIDC_JWKS_URL").unwrap_or_else(|| {
            format!("{keycloak_url}/realms/{keycloak_realm}/protocol/openid-connect/certs")
        });
        let oidc_audience = env::var("OIDC_AUDIENCE").ok().filter(|s| !s.is_empty());
        let auth_dev_bypass = env::var("AUTH_DEV_BYPASS").is_ok_and(|v| v == "true" || v == "1");
        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

        let universesimulator_url = env::var("UNIVERSESIMULATOR_URL")
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();

        let events_retention_days = env::var("NPC_EVENTS_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        let log_buffer_lines = env::var("NPC_LOG_BUFFER_LINES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        let roles_path = env::var("ROLES_PATH").unwrap_or_else(|_| "config/roles.yaml".to_string());

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
            database_url,
            spawn_stagger_ms,
            tick_interval_ms,
            action_startup_delay_ms,
            reconnect_backoff_initial_ms,
            reconnect_backoff_max_ms,
            keycloak_url,
            keycloak_realm,
            npc_login_client_id,
            npc_login_client_secret,
            service_client_id,
            service_client_secret,
            npc_password_key,
            oidc_issuer,
            oidc_jwks_url,
            oidc_audience,
            auth_dev_bypass,
            app_env,
            universesimulator_url,
            events_retention_days,
            log_buffer_lines,
            roles_path,
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

impl Config {
    pub fn is_production(&self) -> bool {
        self.app_env == "production"
    }

    /// Keycloak token endpoint of the realm.
    pub fn token_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.keycloak_url, self.keycloak_realm
        )
    }
}

/// Secrets are never logged: the startup log prints the config through this.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let db_target = self.database_url.rsplit('@').next().unwrap_or_default();
        f.debug_struct("Config")
            .field("horizon_url", &self.horizon_url)
            .field("admin_bind", &self.admin_bind)
            .field("database", &db_target)
            .field("keycloak_url", &self.keycloak_url)
            .field("keycloak_realm", &self.keycloak_realm)
            .field("npc_login_client_id", &self.npc_login_client_id)
            .field("service_client_id", &self.service_client_id)
            .field("oidc_issuer", &self.oidc_issuer)
            .field("auth_dev_bypass", &self.auth_dev_bypass)
            .field("universesimulator_url", &self.universesimulator_url)
            .field("resources_dynamic_http", &self.resources_dynamic_http)
            .field("planet_internal_name", &self.planet_internal_name)
            .field("schedules_path", &self.schedules_path)
            .field("spawn_stagger_ms", &self.spawn_stagger_ms)
            .field("tick_interval_ms", &self.tick_interval_ms)
            .finish_non_exhaustive()
    }
}
