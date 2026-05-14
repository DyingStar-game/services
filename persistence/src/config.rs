use std::env;

/// Application configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Port the WebSocket server listens on.
    pub ws_port: u16,
    /// Port the REST API server listens on.
    pub rest_port: u16,
    /// Comma-separated list of ScyllaDB node addresses.
    pub scylla_nodes: Vec<String>,
    /// ScyllaDB keyspace name.
    pub scylla_keyspace: String,
    /// How often (in seconds) to flush the cache to ScyllaDB.
    pub cache_flush_interval_secs: u64,
    /// Number of items per WebSocket chunk message.
    pub chunk_size: usize,
    /// ScyllaDB username (optional, for PasswordAuthenticator).
    pub scylla_username: Option<String>,
    /// ScyllaDB password (optional, for PasswordAuthenticator).
    pub scylla_password: Option<String>,
}

impl Config {
    /// Load configuration from environment variables.
    /// Missing variables fall back to defaults.
    pub fn from_env() -> Self {
        let ws_port = env::var("WS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9100);

        let rest_port = env::var("REST_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3001);

        let scylla_nodes = env::var("SCYLLA_NODES")
            .unwrap_or_else(|_| "127.0.0.1:9042".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let scylla_keyspace = env::var("SCYLLA_KEYSPACE")
            .unwrap_or_else(|_| "dyingstar".to_string());

        let cache_flush_interval_secs = env::var("CACHE_FLUSH_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let chunk_size = env::var("CHUNK_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);

        let scylla_username = env::var("SCYLLA_USERNAME").ok();
        let scylla_password = env::var("SCYLLA_PASSWORD").ok();

        Self {
            ws_port,
            rest_port,
            scylla_nodes,
            scylla_keyspace,
            cache_flush_interval_secs,
            chunk_size,
            scylla_username,
            scylla_password,
        }
    }
}
