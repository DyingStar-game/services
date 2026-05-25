use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use scylla::client::session::Session;
use tokio::sync::broadcast;
use tracing::{debug, info, warn, error};

use serde_json::Value;

use crate::{
    cache::DualCache,
    config::Config,
    db::queries::{get_all_items, get_item_by_uuid, Queries},
    websocket::messages::{BridgeEventEnvelope, GenericPropsRequest, Item, UpdateObjectRequest},
};

/// Handle a single WebSocket connection for its entire lifetime.
pub async fn handle_socket(
    socket: WebSocket,
    session: Arc<Session>,
    queries: Arc<Queries>,
    cache: DualCache,
    config: Arc<Config>,
    broadcaster: broadcast::Sender<String>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut bcast_rx = broadcaster.subscribe();

    loop {
        tokio::select! {
            // ── Incoming WebSocket message ───────────────────────────────
            msg = receiver.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        debug!("WebSocket receive error: {e}");
                        break;
                    }
                    None => break,
                };

                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue, // ignore binary / ping / pong
                };

                let envelope: BridgeEventEnvelope = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Failed to parse message as BridgeEventEnvelope: {e}");
                        continue;
                    }
                };

                match envelope.name.as_str() {
                    "get_all_items" => {
                        if let Err(e) = handle_get_all_items(
                            &mut sender,
                            &session,
                            &config,
                        )
                        .await
                        {
                            error!("get_all_items error: {e}");
                        }
                    }
                    "update_item" => {
                        match serde_json::from_value::<Item>(envelope.payload) {
                            Ok(item) => {
                                debug!("Caching update for item uuid={}", item.uuid);
                                cache.insert(item);
                            }
                            Err(e) => {
                                error!("update_item: invalid payload: {e}");
                            }
                        }
                    }
                    "update_object" | "update_object_from_external" => {
                        match serde_json::from_value::<UpdateObjectRequest>(envelope.payload) {
                            Ok(req) => {
                                let mut incoming_map = req.object_data
                                    .as_object()
                                    .cloned()
                                    .unwrap_or_default();
                                // Extract special fields from the incoming partial data.
                                let new_parent_id = incoming_map
                                    .remove("parent_id")
                                    .and_then(|v| v.as_str().map(str::to_owned));
                                let new_scenename = incoming_map
                                    .remove("scenename")
                                    .and_then(|v| v.as_str().map(str::to_owned));
                                let new_position = incoming_map
                                    .remove("position")
                                    .and_then(|v| serde_json::from_value(v).ok());
                                let new_rotation = incoming_map
                                    .remove("rotation")
                                    .and_then(|v| serde_json::from_value(v).ok());

                                // Fetch current state: cache first (covers items not yet flushed
                                // to DB), then DB (covers items not in cache).
                                let existing = match cache.get(&req.object_uuid) {
                                    Some(item) => Some(item),
                                    None => match get_item_by_uuid(&session, &queries, &req.object_uuid).await {
                                        Ok(opt) => opt,
                                        Err(e) => {
                                            error!("{}: DB lookup error for uuid={}: {e}", envelope.name, req.object_uuid);
                                            continue;
                                        }
                                    },
                                };

                                let Some(base) = existing else {
                                    warn!("{}: uuid={} not found in cache or DB, skipping", envelope.name, req.object_uuid);
                                    continue;
                                };

                                // Merge: start from existing object_data, then overwrite with
                                // incoming fields so that untouched fields are preserved.
                                let mut merged_data = base
                                    .object_data
                                    .first()
                                    .and_then(|v| v.as_object())
                                    .cloned()
                                    .unwrap_or_default();
                                merged_data.extend(incoming_map);

                                let merged = Item {
                                    uuid: base.uuid,
                                    object_type: req.object_type,
                                    parent_id: new_parent_id.or(base.parent_id),
                                    scenename: new_scenename.or(base.scenename),
                                    position: new_position.or(base.position),
                                    rotation: new_rotation.or(base.rotation),
                                    object_data: vec![Value::Object(merged_data)],
                                };
                                debug!("{}: merged update for uuid={} type={}", envelope.name, merged.uuid, merged.object_type);
                                cache.insert(merged);
                            }
                            Err(e) => {
                                error!("{}: invalid payload: {e}", envelope.name);
                            }
                        }
                    }
                    "create_object" | "create_object_from_gameserver" => {
                        match serde_json::from_value::<GenericPropsRequest>(envelope.payload) {
                            Ok(req) => {
                                let mut data_map = req.object_data
                                    .as_object()
                                    .cloned()
                                    .unwrap_or_default();
                                let parent_id = data_map
                                    .remove("parent_id")
                                    .and_then(|v| v.as_str().map(str::to_owned));
                                let scenename = data_map
                                    .remove("scenename")
                                    .and_then(|v| v.as_str().map(str::to_owned));
                                let position = data_map
                                    .remove("position")
                                    .and_then(|v| serde_json::from_value(v).ok());
                                let rotation = data_map
                                    .remove("rotation")
                                    .and_then(|v| serde_json::from_value(v).ok());
                                let item = Item {
                                    object_type: req.object_type,
                                    uuid: req.object_uuid,
                                    parent_id,
                                    scenename,
                                    position,
                                    rotation,
                                    object_data: vec![Value::Object(data_map)],
                                };
                                info!("Caching new object uuid={} type={}", item.uuid, item.object_type);
                                cache.insert(item);
                            }
                            Err(e) => {
                                error!("{}: invalid payload on create_object*: {e}", envelope.name);
                            }
                        }
                    }
                    "player_spawn" => {
                        debug!("Player spawn event received");
                        match serde_json::from_value::<GenericPropsRequest>(envelope.payload) {
                            Ok(req) if req.object_type == "player" => {
                                match get_item_by_uuid(&session, &queries, &req.object_uuid).await {
                                    Ok(Some(item)) => {
                                        info!("Player found in DB: uuid={} type={}", item.uuid, item.object_type);
                                        let mut data_map = item
                                            .object_data
                                            .first()
                                            .and_then(|v| v.as_object())
                                            .cloned()
                                            .unwrap_or_default();
                                        if let Some(ref v) = item.parent_id {
                                            data_map.insert("parent_id".to_string(), serde_json::json!(v));
                                        }
                                        if let Some(ref v) = item.scenename {
                                            data_map.insert("scenename".to_string(), serde_json::json!(v));
                                        }
                                        if let Some(ref v) = item.position {
                                            data_map.insert("position".to_string(), serde_json::json!(v));
                                        }
                                        if let Some(ref v) = item.rotation {
                                            data_map.insert("rotation".to_string(), serde_json::json!(v));
                                        }
                                        let resp = BridgeEventEnvelope {
                                            event_type: "plugin".to_string(),
                                            namespace: Some("genericprops".to_string()),
                                            name: "create_object".to_string(),
                                            payload: serde_json::json!({
                                                "object_type": item.object_type,
                                                "object_uuid": item.uuid,
                                                "object_data": Value::Object(data_map),
                                            }),
                                        };
                                        let _ = send_json(&mut sender, &resp).await;
                                    }
                                    Ok(None) => {
                                        warn!("player_spawn: player not found in DB for uuid={}", req.object_uuid);
                                        let resp = BridgeEventEnvelope {
                                            event_type: "plugin".to_string(),
                                            namespace: Some("genericprops".to_string()),
                                            name: "new_player".to_string(),
                                            payload: serde_json::json!({
                                                "object_type": "player",
                                                "object_uuid": &req.object_uuid,
                                                "object_data": {
                                                    "name": &req.object_data["name"],
                                                }
                                            }),
                                        };
                                        let _ = send_json(&mut sender, &resp).await;
                                    }
                                    Err(e) => {
                                        error!("player_spawn: DB query error: {e}");
                                    }
                                }
                            }
                            Ok(req) => {
                                warn!("player_spawn: unexpected object_type={}", req.object_type);
                            }
                            Err(e) => {
                                error!("player_spawn: invalid payload: {e}");
                            }
                        }
                    }
                    other => {
                        warn!("Unknown event name {:?}, ignoring", other);
                    }
                }
            }

            // ── Broadcast event from REST API ────────────────────────────
            result = bcast_rx.recv() => {
                match result {
                    Ok(text) => {
                        let _ = sender.send(Message::Text(text.into())).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WS broadcast: client lagged, {n} notification(s) dropped");
                    }
                    Err(_) => break,
                }
            }
        }
    }

    info!("WebSocket connection closed");
}

/// Stream all items from the database back to the client in chunks.
async fn handle_get_all_items(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    session: &Session,
    config: &Config,
) -> Result<(), crate::error::AppError> {
    info!("Handling get_all_items request");

    let items = get_all_items(session, &config.scylla_keyspace).await?;
    let total_items = items.len();
    let chunks: Vec<Vec<serde_json::Value>> = items
        .chunks(config.chunk_size)
        .map(|chunk| {
            chunk
                .iter()
                .map(|item| {
                    let mut data_map = item
                        .object_data
                        .first()
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    if let Some(ref v) = item.parent_id {
                        data_map.insert("parent_id".to_string(), serde_json::json!(v));
                    }
                    if let Some(ref v) = item.scenename {
                        data_map.insert("scenename".to_string(), serde_json::json!(v));
                    }
                    if let Some(ref v) = item.position {
                        data_map.insert("position".to_string(), serde_json::json!(v));
                    }
                    if let Some(ref v) = item.rotation {
                        data_map.insert("rotation".to_string(), serde_json::json!(v));
                    }
                    serde_json::json!({
                        "object_type": item.object_type,
                        "object_uuid": item.uuid,
                        "object_data": Value::Object(data_map),
                    })
                })
                .collect()
        })
        .collect();
    let total_chunks = chunks.len();

    for (index, chunk) in chunks.iter().enumerate() {
        let resp = BridgeEventEnvelope {
            event_type: "plugin".to_string(),
            namespace: Some("genericprops".to_string()),
            name: "items_chunk".to_string(),
            payload: serde_json::json!({ "items": chunk, "chunk_index": index }),
        };
        send_json(sender, &resp).await?;
    }

    let end = BridgeEventEnvelope {
        event_type: "plugin".to_string(),
        namespace: Some("genericprops".to_string()),
        name: "items_end".to_string(),
        payload: serde_json::json!({ "total_chunks": total_chunks, "total_items": total_items }),
    };
    send_json(sender, &end).await?;

    info!("Sent {total_chunks} chunk(s) to client");
    Ok(())
}

/// Serialise a response to JSON and send it as a WebSocket text message.
async fn send_json<T: serde::Serialize>(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    value: &T,
) -> Result<(), crate::error::AppError> {
    let text = serde_json::to_string(value)?;
    sender
        .send(Message::Text(text.into()))
        .await
        .map_err(|e| crate::error::AppError::WebSocket(e.to_string()))?;
    Ok(())
}
