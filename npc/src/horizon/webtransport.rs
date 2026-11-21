//! WebTransport + bitcode connection to Horizon (Horizon Client Transport v2).
//!
//! Same contract as the WebSocket client ([`super::client`]): a [`HorizonSender`]
//! of `{namespace, event, data}` envelopes, a [`HorizonReceiver`] of standardised
//! frames, and the player UUID from `init_ack`. Underneath:
//!
//! - one bidirectional QUIC stream, opened by the client, carries every reliable
//!   frame both ways, each prefixed by its length (LEB128). Everything the NPC
//!   sends goes there: inputs must arrive, and in order;
//! - Horizon's unreliable frames (transforms) arrive as datagrams.
//!
//! The first frame is `Hello`; `HelloAck` gives the event ids
//! ([`BitcodeSession`]).

use std::sync::{Arc, Mutex};

use horizon_protocol::{
    stream::{write_frame, FrameSplitter},
    Compression,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wtransport::{ClientConfig, Endpoint};

use super::{
    bitcode::BitcodeSession,
    client::{HorizonReceiver, HorizonSender},
    messages::StandardizedFrame,
};
use crate::{
    config::Config,
    error::{NpcError, Result},
};

/// Largest reliable frame accepted from Horizon, before decompression.
const MAX_STREAM_FRAME: usize = 4 * 1024 * 1024;

fn err(context: &str, e: impl std::fmt::Display) -> NpcError {
    NpcError::WebSocket(format!("webtransport {context}: {e}"))
}

/// Connect, send `Hello` then `player/init { token }`, await `init_ack`, then start
/// the reader and writer tasks.
pub async fn connect(cfg: &Config, token: &str) -> Result<(HorizonSender, HorizonReceiver, String)> {
    let url = &cfg.horizon_wt_url;
    info!(%url, "connecting to Horizon (WebTransport, bitcode)");
    let builder = ClientConfig::builder().with_bind_default();
    let config = if cfg.horizon_wt_insecure {
        builder.with_no_cert_validation().build()
    } else {
        builder.with_native_certs().build()
    };
    let endpoint = Endpoint::client(config).map_err(|e| err("endpoint", e))?;
    let connection = endpoint.connect(url).await.map_err(|e| err(&format!("dial {url}"), e))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| err("open stream", e))?
        .await
        .map_err(|e| err("open stream", e))?;

    let session = Arc::new(Mutex::new(BitcodeSession::new(Compression::with_codec(cfg.horizon_compression))));
    let init = json!({
        "namespace": "player",
        "event": "init",
        "data": { "token": token, "is_npc": true },
    });
    let mut out = Vec::new();
    {
        let session = session.lock().unwrap();
        write_frame(&mut out, &session.hello());
        write_frame(&mut out, &session.encode_envelope(&init).map_err(NpcError::Protocol)?);
    }
    send.write_all(&out).await.map_err(|e| err("send init", e))?;

    // Await init_ack; anything before it is dropped, as on WebSocket.
    let mut splitter = FrameSplitter::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let my_uuid = 'handshake: loop {
        let received: Vec<Vec<u8>> = tokio::select! {
            read = recv.read(&mut buffer) => match read.map_err(|e| err("recv during init", e))? {
                Some(read) => {
                    splitter.push(&buffer[..read]);
                    let mut frames = Vec::new();
                    while let Some(frame) = splitter.next_frame(MAX_STREAM_FRAME).map_err(|e| err("recv during init", e))? {
                        frames.push(frame);
                    }
                    frames
                }
                None => return Err(NpcError::WebSocket("webtransport closed before init_ack".into())),
            },
            datagram = connection.receive_datagram() => vec![datagram.map_err(|e| err("recv during init", e))?.payload().to_vec()],
        };
        for bytes in received {
            for frame in session.lock().unwrap().decode(&bytes) {
                match frame {
                    StandardizedFrame::InitAck { player_id } => break 'handshake player_id,
                    StandardizedFrame::Error { message } => {
                        return Err(NpcError::Protocol(format!("server rejected init: {message}")));
                    }
                    other => debug!(?other, "ignoring frame before init_ack"),
                }
            }
        }
    };
    info!(%my_uuid, "Horizon handshake complete (WebTransport)");

    let (out_tx, mut out_rx) = mpsc::channel::<Value>(64);
    let (in_tx, in_rx) = mpsc::channel::<StandardizedFrame>(256);

    // Writer task: every envelope on the reliable stream.
    let writer_session = Arc::clone(&session);
    tokio::spawn(async move {
        while let Some(envelope) = out_rx.recv().await {
            let frame = match writer_session.lock().unwrap().encode_envelope(&envelope) {
                Ok(frame) => frame,
                Err(e) => {
                    warn!(error = %e, "drop unencodable envelope");
                    continue;
                }
            };
            let mut out = Vec::with_capacity(frame.len() + 4);
            write_frame(&mut out, &frame);
            if let Err(e) = send.write_all(&out).await {
                warn!(error = %e, "Horizon writer error");
                break;
            }
        }
        let _ = send.finish().await;
        debug!("Horizon writer task ended");
    });

    // Reader task: stream frames and datagrams. Keeps the endpoint and the
    // connection alive; dropping them closes the session.
    tokio::spawn(async move {
        let _endpoint = endpoint;
        loop {
            let received: Vec<Vec<u8>> = tokio::select! {
                read = recv.read(&mut buffer) => match read {
                    Ok(Some(read)) => {
                        splitter.push(&buffer[..read]);
                        let mut frames = Vec::new();
                        loop {
                            match splitter.next_frame(MAX_STREAM_FRAME) {
                                Ok(Some(frame)) => frames.push(frame),
                                Ok(None) => break,
                                Err(e) => {
                                    warn!(error = %e, "Horizon stream corrupted");
                                    return;
                                }
                            }
                        }
                        frames
                    }
                    Ok(None) | Err(_) => break,
                },
                datagram = connection.receive_datagram() => match datagram {
                    Ok(datagram) => vec![datagram.payload().to_vec()],
                    Err(_) => break,
                },
            };
            let frames: Vec<StandardizedFrame> =
                received.iter().flat_map(|bytes| session.lock().unwrap().decode(bytes)).collect();
            for frame in frames {
                if in_tx.send(frame).await.is_err() {
                    return;
                }
            }
        }
        // The close code (4000-4008, see horizon_protocol::CloseCode) when Horizon closed it.
        let reason = tokio::time::timeout(std::time::Duration::from_millis(100), connection.closed()).await;
        info!(reason = ?reason.ok(), "Horizon connection closed (WebTransport)");
    });

    Ok((HorizonSender::from_channel(out_tx), in_rx, my_uuid))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a running Horizon with the DyingStar plugins and a WebTransport
    /// listener (not run by default):
    ///
    /// ```text
    /// HORIZON_WT_URL=https://localhost:17943 HORIZON_WT_INSECURE=true \
    ///     cargo test -- --ignored handshakes_with_a_running_horizon
    /// ```
    ///
    /// Without a Godot server connected Horizon refuses the init ("Server not yet
    /// ready"), which still goes through the whole bitcode path both ways.
    #[tokio::test]
    #[ignore]
    async fn handshakes_with_a_running_horizon() {
        let cfg = Config::from_env();
        let token = std::env::var("HORIZON_TOKEN").unwrap_or_else(|_| "npc-e2e".into());
        match connect(&cfg, &token).await {
            Ok((_, _, player_id)) => assert!(!player_id.is_empty()),
            Err(NpcError::Protocol(message)) if message.contains("Server not yet ready") => {}
            Err(e) => panic!("{e}"),
        }
    }
}
