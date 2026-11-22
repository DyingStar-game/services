//! The WebSocket Horizon's `ds_bridge` connects to. Everything the simulation publishes is forwarded
//! here; the only message read back is the bridge's handshake, which triggers a replay of the whole
//! state (a restarted Horizon knows nothing) and a refresh of the planets' uuids (a restarted
//! celestial service minted new ones).

use crate::sim::replay;
use crate::state::Shared;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use ds_weather_proto::BridgeEnvelope;
use futures::{SinkExt, StreamExt};
use tracing::{debug, info, warn};

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, state))
}

async fn handle(socket: WebSocket, state: Shared) {
    state.metrics.bridges_connected.inc();
    info!("bridge connected");
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.tx.subscribe();
    let forward = {
        let state = state.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(text) => {
                        if sink.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "bridge too slow: envelopes dropped, replaying");
                        replay(&state);
                    }
                    Err(_) => break,
                }
            }
        }
    };
    let read = {
        let state = state.clone();
        async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Text(text)) => on_envelope(&state, text.as_str()).await,
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        }
    };
    tokio::select! {
        _ = forward => {}
        _ = read => {}
    }
    state.metrics.bridges_connected.dec();
    info!("bridge disconnected");
}

async fn on_envelope(state: &Shared, raw: &str) {
    let env: BridgeEnvelope = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, raw = raw.chars().take(200).collect::<String>(), "unreadable envelope");
            return;
        }
    };
    match (env.event_type.as_str(), env.name.as_str()) {
        ("bridge", "connected") => {
            let reconnection = env.payload.get("is_reconnection").and_then(|v| v.as_bool()).unwrap_or(false);
            info!(reconnection, "Horizon handshake: refreshing planet uuids and replaying the sky");
            if let Err(e) = state.refresh_planet_uuids().await {
                warn!(error = %e, "planet uuids not refreshed");
            }
            // The uuids may have changed: rebuild everything on the next tick, and replay what we
            // have now so a restarted Horizon is not empty meanwhile.
            let n = replay(state);
            debug!(systems = n, "replayed");
        }
        _ => debug!(event_type = %env.event_type, name = %env.name, "ignored"),
    }
}
