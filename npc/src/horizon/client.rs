//! Async WebSocket client for the Horizon Godot-client protocol.
//!
//! Usage:
//! ```ignore
//! let (sender, mut receiver, my_uuid) = HorizonClient::connect(url, token).await?;
//! sender.send(player_action(...)).await?;
//! while let Some(frame) = receiver.recv().await { ... }
//! ```

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    net::TcpStream,
    sync::mpsc,
    time,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, protocol::CloseFrame, protocol::frame::coding::CloseCode},
};
use tracing::{debug, info, warn};

use crate::{
    config::Config,
    error::{NpcError, Result},
    horizon::{
        gorc,
        messages::{OutboundEnvelope, RawFrame, StandardizedFrame},
    },
};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Sender side of an active Horizon connection. Cheap to clone; messages are
/// queued onto an mpsc and serialised by the writer task.
#[derive(Clone)]
pub struct HorizonSender {
    tx: mpsc::Sender<Value>,
}

impl HorizonSender {
    /// Send an arbitrary outbound envelope `{namespace, event, data}`.
    pub async fn send(&self, namespace: &str, event: &str, data: Value) -> Result<()> {
        let env = OutboundEnvelope { namespace, event, data };
        let v = serde_json::to_value(&env)?;
        self.tx
            .send(v)
            .await
            .map_err(|e| NpcError::WebSocket(format!("send channel closed: {e}")))
    }

    /// Test-only constructor.
    #[doc(hidden)]
    pub fn __test_new(tx: mpsc::Sender<Value>) -> Self {
        Self { tx }
    }
}

/// Receiver side: a stream of normalised inbound frames.
pub type HorizonReceiver = mpsc::Receiver<StandardizedFrame>;

pub struct HorizonClient;

impl HorizonClient {
    /// Connect, send `player/init { token }`, await `init_ack`, then start
    /// reader & writer tasks. Returns a sender, a receiver and the assigned
    /// player UUID.
    pub async fn connect(
        url: &str,
        token: &str,
    ) -> Result<(HorizonSender, HorizonReceiver, String)> {
        info!(%url, "connecting to Horizon");
        let (ws, _resp) = connect_async(url)
            .await
            .map_err(|e| NpcError::WebSocket(format!("dial {url}: {e}")))?;

        let (sender, receiver, my_uuid) = handshake_and_split(ws, token).await?;
        Ok((sender, receiver, my_uuid))
    }

    /// Like `connect`, but keeps retrying with exponential backoff. Yields
    /// when a connection is established successfully.
    pub async fn connect_with_retry(
        cfg: &Config,
        url: &str,
        token: &str,
    ) -> (HorizonSender, HorizonReceiver, String) {
        let mut backoff = cfg.reconnect_backoff_initial_ms;
        loop {
            match Self::connect(url, token).await {
                Ok(t) => return t,
                Err(e) => {
                    warn!(error = %e, backoff_ms = backoff, "Horizon connect failed; retrying");
                    time::sleep(Duration::from_millis(backoff)).await;
                    backoff = (backoff.saturating_mul(2)).min(cfg.reconnect_backoff_max_ms);
                }
            }
        }
    }
}

async fn handshake_and_split(
    mut ws: WsStream,
    token: &str,
) -> Result<(HorizonSender, HorizonReceiver, String)> {
    // 1. Send player/init.
    let init = json!({
        "namespace": "player",
        "event": "init",
        "data": { "token": token, "is_npc": true },
    });
    ws.send(Message::Text(init.to_string().into()))
        .await
        .map_err(|e| NpcError::WebSocket(format!("send init: {e}")))?;

    // 2. Await init_ack (skip pings / non-text frames).
    let my_uuid = loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => {
                let raw: RawFrame = serde_json::from_str(&t)
                    .map_err(|e| NpcError::Protocol(format!("init reply parse: {e}")))?;
                match gorc::standardize(raw) {
                    StandardizedFrame::InitAck { player_id } => break player_id,
                    StandardizedFrame::Error { message } => {
                        return Err(NpcError::Protocol(format!("server rejected init: {message}")));
                    }
                    other => {
                        debug!(?other, "ignoring frame before init_ack");
                    }
                }
            }
            Some(Ok(Message::Ping(p))) => {
                let _ = ws.send(Message::Pong(p)).await;
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(NpcError::WebSocket("closed before init_ack".into()));
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                return Err(NpcError::WebSocket(format!("recv during init: {e}")));
            }
        }
    };
    info!(%my_uuid, "Horizon handshake complete");

    // 3. Split & spawn IO tasks.
    let (mut sink, mut stream) = ws.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Value>(64);
    let (in_tx, in_rx) = mpsc::channel::<StandardizedFrame>(256);

    // Writer task.
    tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            let msg = Message::Text(v.to_string().into());
            if let Err(e) = sink.send(msg).await {
                warn!(error = %e, "Horizon writer error");
                break;
            }
        }
        let _ = sink
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "client closing".into(),
            })))
            .await;
        debug!("Horizon writer task ended");
    });

    // Reader task.
    let in_tx_for_reader = in_tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(t)) => match serde_json::from_str::<RawFrame>(&t) {
                    Ok(raw) => {
                        let f = gorc::standardize(raw);
                        if in_tx_for_reader.send(f).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => warn!(error = %e, payload = %t, "drop unparseable frame"),
                },
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Binary(b)) => {
                    debug!(len = b.len(), "ignoring binary frame");
                }
                Ok(Message::Close(_)) | Ok(Message::Frame(_)) | Err(_) => {
                    break;
                }
            }
        }
        debug!("Horizon reader task ended");
    });

    Ok((HorizonSender { tx: out_tx }, in_rx, my_uuid))
}
