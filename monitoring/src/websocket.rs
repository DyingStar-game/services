use axum::{
    extract::{
        ws::{Message, WebSocket},
        WebSocketUpgrade,
    },
    response::IntoResponse,
};
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::metrics::Metrics;

/// Event envelope sent by ds_bridge to external services.
/// Must match the BridgeEventEnvelope struct in ds_bridge/src/config.rs.
#[derive(Debug, Deserialize)]
struct BridgeEventEnvelope {
    event_type: String,
    namespace: Option<String>,
    name: String,
    // payload fields we don't inspect — keep as raw JSON
    #[allow(dead_code)]
    payload: serde_json::Value,
}

/// Axum handler — upgrades an HTTP connection to WebSocket.
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

/// Process messages on an accepted WebSocket connection.
async fn handle_socket(mut socket: WebSocket) {
    debug!("ds_bridge connected");

    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(Message::Text(text)) => {
                handle_envelope(text.as_str());
            }
            Ok(Message::Ping(data)) => {
                // Axum auto-replies to pings; send explicit Pong anyway.
                let _ = socket.send(Message::Pong(data)).await;
            }
            Ok(Message::Close(_)) | Err(_) => {
                debug!("ds_bridge disconnected");
                break;
            }
            _ => {} // Binary / Pong — ignore
        }
    }
}

/// Dispatch a raw JSON envelope string to the appropriate metric update.
fn handle_envelope(raw: &str) {
    let env: BridgeEventEnvelope = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            warn!("failed to parse envelope: {} — raw: {}", e, raw);
            return;
        }
    };

    debug!(
        event_type = %env.event_type,
        namespace = ?env.namespace,
        name = %env.name,
        "received event"
    );

    let m = Metrics::get();

    match (env.event_type.as_str(), env.namespace.as_deref(), env.name.as_str()) {
        // ── Bridge lifecycle ──────────────────────────────────────────────────
        ("bridge", _, "connected") => {
            let is_reconnection = env.payload
                .get("is_reconnection")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_reconnection {
                info!("Horizon restarted — resetting all metrics");
                m.reset();
            } else {
                debug!("Horizon reconnected (reconnection=true) — keeping metrics");
            }
        }

        // ── Core events ──────────────────────────────────────────────────────
        ("core", _, "player_connected") => {
            m.players_connected.inc();
            m.players_connect_total.inc();
        }
        ("core", _, "player_disconnected") => {
            // Clamp at 0 — in case a disconnect arrives before a reconnect.
            let current = m.players_connected.get();
            m.players_connected.set((current - 1.0).max(0.0));
            m.players_disconnect_total.inc();
        }

        // ── Generic props ─────────────────────────────────────────────────────
        ("plugin", Some("genericprops"), "create_object") => {
            m.items_created_total.with_label_values(&["generic"]).inc();
        }
        ("plugin", Some("genericprops"), "create_object_from_gameserver") => {
            m.items_created_total.with_label_values(&["gameserver"]).inc();
        }

        // ── Authentication ────────────────────────────────────────────────────
        ("plugin", Some("ds_player_authentication"), "player_authenticated") => {
            m.auth_success_total.inc();
        }

        // ── Godot game servers ────────────────────────────────────────────────
        ("plugin", Some("ds_game_server"), "server_registered") => {
            m.godot_servers_active.inc();
        }
        ("plugin", Some("ds_game_server"), "server_unregistered") => {
            let current = m.godot_servers_active.get();
            m.godot_servers_active.set((current - 1.0).max(0.0));
        }

        _ => {
            debug!(
                event_type = %env.event_type,
                namespace = ?env.namespace,
                name = %env.name,
                "unhandled event — ignoring"
            );
        }
    }
}
