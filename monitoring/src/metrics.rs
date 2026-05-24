use prometheus::{
    register_gauge, register_int_counter, register_int_counter_vec,
    Gauge, IntCounter, IntCounterVec,
};
use std::sync::OnceLock;

/// All Prometheus metrics for the monitoring service.
/// Initialised once at startup via `Metrics::init()`.
pub struct Metrics {
    /// Current number of players connected to Horizon (gauge).
    pub players_connected: Gauge,
    /// Total player connections since startup (counter).
    pub players_connect_total: IntCounter,
    /// Total player disconnections since startup (counter).
    pub players_disconnect_total: IntCounter,
    /// Number of active Godot game server instances (gauge).
    pub godot_servers_active: Gauge,
    /// Total items created, by kind (counter, label: kind).
    pub items_created_total: IntCounterVec,
    /// Total successful player authentications (counter).
    pub auth_success_total: IntCounter,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

impl Metrics {
    /// Register all metrics with the default Prometheus registry.
    /// Must be called exactly once at startup before the HTTP server starts.
    pub fn init() -> &'static Metrics {
        METRICS.get_or_init(|| Metrics {
            players_connected: register_gauge!(
                "horizon_players_connected",
                "Current number of players connected to Horizon"
            )
            .expect("metric registration failed"),

            players_connect_total: register_int_counter!(
                "horizon_players_connect_total",
                "Total player connections since service start"
            )
            .expect("metric registration failed"),

            players_disconnect_total: register_int_counter!(
                "horizon_players_disconnect_total",
                "Total player disconnections since service start"
            )
            .expect("metric registration failed"),

            godot_servers_active: register_gauge!(
                "horizon_godot_servers_active",
                "Number of active Godot game server instances registered with Horizon"
            )
            .expect("metric registration failed"),

            items_created_total: register_int_counter_vec!(
                "horizon_items_created_total",
                "Total items created in Horizon, by kind",
                &["kind"]
            )
            .expect("metric registration failed"),

            auth_success_total: register_int_counter!(
                "horizon_auth_success_total",
                "Total successful player authentications"
            )
            .expect("metric registration failed"),
        })
    }

    /// Get the global metrics instance (panics if not yet initialised).
    pub fn get() -> &'static Metrics {
        METRICS.get().expect("Metrics not initialised — call Metrics::init() first")
    }

    /// Reset all metric values to zero (called when Horizon restarts).
    pub fn reset(&self) {
        self.players_connected.set(0.0);
        self.godot_servers_active.set(0.0);
        self.players_connect_total.reset();
        self.players_disconnect_total.reset();
        self.auth_success_total.reset();
        self.items_created_total.reset();
    }
}
