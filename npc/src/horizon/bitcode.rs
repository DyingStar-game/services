//! Bitcode side of the Horizon protocol (Horizon Client Transport v2).
//!
//! The rest of the NPC speaks the JSON shapes of the Godot client: outbound
//! `{namespace, event, data}` envelopes, inbound frames normalised by
//! [`gorc::standardize`]. A [`BitcodeSession`] translates both ways, so a
//! bitcode connection feeds the NPC exactly what a JSON one would:
//!
//! - outbound, an envelope becomes a `ClientFrame::Event` whose payload is the
//!   `ds_protocol` type the plugins decode for that event (`PlayerInit`,
//!   `UpdateVelocity`, a `DynValue` for free-form events);
//! - inbound, each `ServerFrame` is turned back into the JSON object a JSON client
//!   would have received, then standardised.
//!
//! GORC frames name objects by a handle (from their zone entry). Props send
//! position and rotation as `<event>/transform` frames, unreliable: a transform
//! older than the last one applied (GORC `seq`) or of another epoch than the
//! object's parent (see `ds_protocol::props`) is dropped.

use std::collections::HashMap;

use horizon_protocol::{
    bitcode, decode_server, encode_client, ClientFrame, Compression, EventKey, ServerFrame, PROTOCOL_VERSION,
};
use serde_json::{json, Map, Value};
use tracing::{debug, warn};

use super::{
    gorc,
    messages::{RawFrame, StandardizedFrame},
};

/// Largest frame accepted from Horizon, after decompression.
const MAX_FRAME: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
struct ObjectInfo {
    object_id: String,
    object_type: String,
    /// Epoch of the object's parent, from its zone entry or its reliable updates.
    epoch: u32,
}

/// State of one bitcode connection. See the module docs.
pub struct BitcodeSession {
    compression: Compression,
    /// Event ids from `HelloAck`, both ways.
    ids: HashMap<String, u16>,
    names: HashMap<u16, String>,
    objects: HashMap<u32, ObjectInfo>,
    /// Last transform applied, per (handle, channel).
    last_transform: HashMap<(u32, u8), u16>,
}

impl BitcodeSession {
    pub fn new(compression: Compression) -> Self {
        Self {
            compression,
            ids: HashMap::new(),
            names: HashMap::new(),
            objects: HashMap::new(),
            last_transform: HashMap::new(),
        }
    }

    /// The first frame of the connection.
    pub fn hello(&self) -> Vec<u8> {
        let hello = ClientFrame::Hello {
            protocol_version: PROTOCOL_VERSION,
            client: "dyingstar-npc".into(),
            dictionary_id: None,
        };
        self.compression.wrap(&encode_client(&hello)).to_vec()
    }

    fn key(&self, name: &str) -> EventKey {
        match self.ids.get(name) {
            Some(id) => EventKey::Id(*id),
            None => EventKey::name(name),
        }
    }

    fn name(&self, key: &EventKey) -> Option<String> {
        match key {
            EventKey::Id(id) => self.names.get(id).cloned(),
            EventKey::Name(name) => Some(name.clone()),
        }
    }

    /// Encodes an outbound `{namespace, event, data}` envelope.
    pub fn encode_envelope(&self, envelope: &Value) -> Result<Vec<u8>, String> {
        let namespace = envelope["namespace"].as_str().ok_or("envelope without namespace")?;
        let event = envelope["event"].as_str().ok_or("envelope without event")?;
        let name = format!("{namespace}:{event}");
        let data = &envelope["data"];
        // The payload type each plugin handler decodes (on_client_packed).
        let payload = match name.as_str() {
            "player:init" => bitcode::encode(&parse::<ds_protocol::PlayerInit>(&name, data)?),
            "movement:update_velocity" => bitcode::encode(&parse::<ds_protocol::UpdateVelocity>(&name, data)?),
            _ => bitcode::encode(&ds_protocol::DynValue::from(data)),
        };
        let frame = ClientFrame::Event { key: self.key(&name), payload };
        Ok(self.compression.wrap(&encode_client(&frame)).to_vec())
    }

    /// Decodes what Horizon sent (one frame, or a batch) into standardised frames.
    pub fn decode(&mut self, bytes: &[u8]) -> Vec<StandardizedFrame> {
        let frames = match self
            .compression
            .unwrap(bytes, MAX_FRAME)
            .map_err(|e| e.to_string())
            .and_then(|body| decode_server(&body, MAX_FRAME).map_err(|e| e.to_string()))
        {
            Ok(ServerFrame::Batch { frames }) => match ServerFrame::unbatch(&frames) {
                Ok(frames) => frames,
                Err(e) => {
                    warn!(error = %e, "drop undecodable batch");
                    return Vec::new();
                }
            },
            Ok(frame) => vec![frame],
            Err(e) => {
                warn!(error = %e, len = bytes.len(), "drop undecodable frame");
                return Vec::new();
            }
        };
        frames.into_iter().filter_map(|frame| self.standardize(frame)).collect()
    }

    fn standardize(&mut self, frame: ServerFrame) -> Option<StandardizedFrame> {
        let raw = match frame {
            ServerFrame::HelloAck { event_table, .. } => {
                for (id, name) in event_table {
                    self.ids.insert(name.clone(), id);
                    self.names.insert(id, name);
                }
                return None;
            }
            ServerFrame::Event { key, payload } => {
                // The JSON message of a JSON client, carried as is.
                let message = bitcode::decode::<ds_protocol::DynValue>(&payload).ok()?.to_value().ok()?;
                if !message.is_object() {
                    debug!(event = ?self.name(&key), "event without an object payload");
                    return None;
                }
                message
            }
            ServerFrame::ZoneEnter { handle, object_id, object_type, channel, zone_data } => {
                let object_id = uuid::Uuid::from_bytes(object_id).to_string();
                let (zone_data, epoch) = match bitcode::decode::<ds_protocol::PropState>(&zone_data) {
                    Ok(state) => (Value::Object(joined(&state.transform, state.data.into_value())), state.transform.epoch),
                    Err(_) => (Value::Null, 0),
                };
                self.objects.insert(handle, ObjectInfo { object_id: object_id.clone(), object_type: object_type.clone(), epoch });
                json!({
                    "type": "gorc_zone_enter",
                    "object_id": object_id,
                    "object_type": object_type,
                    "channel": channel,
                    "zone_data": zone_data,
                })
            }
            ServerFrame::ZoneExit { handle, channel } => {
                let object = self.objects.get(&handle)?;
                json!({
                    "type": "gorc_zone_exit",
                    "object_id": object.object_id,
                    "object_type": object.object_type,
                    "channel": channel,
                })
            }
            ServerFrame::GorcEvent { handle, channel, seq, event, payload } => {
                let name = self.name(&event)?;
                let data = match ds_protocol::transform_of(&name) {
                    Some(base) => {
                        let base = base.to_string();
                        let transform = bitcode::decode::<ds_protocol::TransformUpdate>(&payload).ok()?;
                        let object = self.objects.get(&handle)?;
                        if transform.epoch != object.epoch {
                            return None; // under a parent this NPC does not know yet, or no longer
                        }
                        let last = self.last_transform.entry((handle, channel)).or_insert(seq.wrapping_sub(1));
                        if (seq.wrapping_sub(*last) as i16) < 0 {
                            return None; // late datagram
                        }
                        *last = seq;
                        let mut data = joined(&transform, Value::Null);
                        if base == "move" {
                            // Left out of bitcode frames: it is the object's own id.
                            data.insert("player_id".into(), json!(object.object_id));
                        }
                        return Some(gorc_frame(&base, object, channel, Value::Object(data)));
                    }
                    None => {
                        let update = bitcode::decode::<ds_protocol::PropUpdate>(&payload).ok()?;
                        let object = self.objects.get_mut(&handle)?;
                        // A reliable update announces the parent of its epoch.
                        if update.epoch > object.epoch || object.epoch.wrapping_sub(update.epoch) > u32::MAX / 2 {
                            object.epoch = update.epoch;
                        }
                        update.data.into_value()
                    }
                };
                let object = self.objects.get(&handle)?;
                return Some(gorc_frame(&name, object, channel, data));
            }
            ServerFrame::Disconnect { code, reason } => {
                return Some(StandardizedFrame::Error { message: format!("disconnected ({code}): {reason}") });
            }
            ServerFrame::Pong { .. } | ServerFrame::Batch { .. } => return None,
        };
        let raw: RawFrame = serde_json::from_value(raw).ok()?;
        Some(gorc::standardize(raw))
    }
}

/// The envelope of a GORC event, as a JSON client gets it.
fn gorc_frame(event: &str, object: &ObjectInfo, channel: u8, data: Value) -> StandardizedFrame {
    let raw = json!({
        "event_type": event,
        "object_id": object.object_id,
        "object_type": object.object_type,
        "channel": channel,
        "player_id": object.object_id,
        "data": data,
    });
    match serde_json::from_value::<RawFrame>(raw) {
        Ok(raw) => gorc::standardize(raw),
        Err(_) => StandardizedFrame::Other { name: event.to_string(), payload: Value::Null },
    }
}

/// A transform and other keys back in one object.
fn joined(transform: &ds_protocol::TransformUpdate, rest: Value) -> Map<String, Value> {
    let rest = match rest {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    ds_protocol::props::join(transform, &rest)
}

fn parse<T: serde::de::DeserializeOwned>(name: &str, data: &Value) -> Result<T, String> {
    serde_json::from_value(data.clone()).map_err(|e| format!("{name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon::messages::GorcKind;
    use horizon_protocol::{decode_client, encode_server, Codec};

    fn session() -> BitcodeSession {
        BitcodeSession::new(Compression::with_codec(Codec::Zstd))
    }

    fn wire(session: &BitcodeSession, frame: &ServerFrame) -> Vec<u8> {
        session.compression.wrap(&encode_server(frame)).to_vec()
    }

    fn ack(session: &mut BitcodeSession, names: &[&str]) {
        let event_table = names.iter().enumerate().map(|(i, n)| (i as u16 + 1, n.to_string())).collect();
        let bytes = wire(session, &ServerFrame::HelloAck { protocol_version: PROTOCOL_VERSION, codec: 1, dictionary_id: None, event_table });
        assert!(session.decode(&bytes).is_empty());
    }

    fn state(position: [f64; 3], epoch: u32, data: Value) -> Vec<u8> {
        bitcode::encode(&ds_protocol::PropState {
            transform: ds_protocol::TransformUpdate { epoch, position: Some(position), rotation: Some([0.0, 0.5, 0.0]), ..Default::default() },
            data: ds_protocol::DynValue::from(&data),
        })
    }

    fn enter(session: &mut BitcodeSession, handle: u32, object_id: uuid::Uuid, epoch: u32) -> StandardizedFrame {
        let bytes = wire(session, &ServerFrame::ZoneEnter {
            handle,
            object_id: *object_id.as_bytes(),
            object_type: "player".into(),
            channel: 0,
            zone_data: state([1.0, 2.0, 3.0], epoch, json!({"parent_id": "planet"})),
        });
        session.decode(&bytes).pop().unwrap()
    }

    fn transform(session: &BitcodeSession, handle: u32, seq: u16, epoch: u32, x: f64) -> Vec<u8> {
        wire(session, &ServerFrame::GorcEvent {
            handle,
            channel: 0,
            seq,
            event: session.key("move/transform"),
            payload: bitcode::encode(&ds_protocol::TransformUpdate { epoch, position: Some([x, 0.0, 0.0]), ..Default::default() }),
        })
    }

    #[test]
    fn events_become_the_json_messages() {
        let mut s = session();
        let bytes = wire(&s, &ServerFrame::Event {
            key: EventKey::name("player:init_ack"),
            payload: bitcode::encode(&ds_protocol::DynValue::from(&json!({"type": "init_ack", "player_id": "u1"}))),
        });
        match s.decode(&bytes).as_slice() {
            [StandardizedFrame::InitAck { player_id }] => assert_eq!(player_id, "u1"),
            other => panic!("expected InitAck, got {other:?}"),
        }
    }

    #[test]
    fn zone_entries_carry_the_whole_state() {
        let mut s = session();
        let id = uuid::Uuid::new_v4();
        match enter(&mut s, 5, id, 0) {
            StandardizedFrame::Gorc(g) => {
                assert_eq!(g.kind, GorcKind::ZoneEnter);
                assert_eq!(g.object_id, id.to_string());
                assert_eq!(g.data["position"], json!({"x": 1.0, "y": 2.0, "z": 3.0}));
                assert_eq!(g.data["parent_id"], json!("planet"));
            }
            other => panic!("expected Gorc, got {other:?}"),
        }
    }

    #[test]
    fn transforms_are_moves_and_late_ones_are_dropped() {
        let mut s = session();
        ack(&mut s, &["move", "move/transform"]);
        let id = uuid::Uuid::new_v4();
        enter(&mut s, 5, id, 0);
        match s.decode(&transform(&s, 5, 10, 0, 7.0)).as_slice() {
            [StandardizedFrame::Gorc(g)] => {
                assert_eq!(g.kind, GorcKind::Update);
                assert_eq!(g.data["position"]["x"], json!(7.0));
                assert_eq!(g.player_id.as_deref(), Some(id.to_string().as_str()));
            }
            other => panic!("expected a move, got {other:?}"),
        }
        assert!(s.decode(&transform(&s, 5, 9, 0, 6.0)).is_empty(), "older seq");
        assert_eq!(s.decode(&transform(&s, 5, 10, 0, 7.0)).len(), 1, "a repeat of the last one is applied again");
    }

    #[test]
    fn transforms_wait_for_their_parent() {
        let mut s = session();
        ack(&mut s, &["move", "move/transform"]);
        enter(&mut s, 5, uuid::Uuid::new_v4(), 1);
        assert!(s.decode(&transform(&s, 5, 3, 2, 1.0)).is_empty(), "epoch 2 not announced yet");
        let update = wire(&s, &ServerFrame::GorcEvent {
            handle: 5,
            channel: 0,
            seq: 3,
            event: s.key("move"),
            payload: bitcode::encode(&ds_protocol::PropUpdate { epoch: 2, data: ds_protocol::DynValue::from(&json!({"parent_id": "city"})) }),
        });
        match s.decode(&update).as_slice() {
            [StandardizedFrame::Gorc(g)] => assert_eq!(g.data["parent_id"], json!("city")),
            other => panic!("expected the update, got {other:?}"),
        }
        assert_eq!(s.decode(&transform(&s, 5, 4, 2, 1.0)).len(), 1);
    }

    #[test]
    fn envelopes_use_the_plugins_payload_types() {
        let mut s = session();
        ack(&mut s, &["player:client_action"]);
        let init = s.encode_envelope(&json!({"namespace": "player", "event": "init", "data": {"token": "t", "is_npc": true}})).unwrap();
        let body = s.compression.unwrap(&init, MAX_FRAME).unwrap();
        match decode_client(&body, MAX_FRAME).unwrap() {
            ClientFrame::Event { key, payload } => {
                assert_eq!(key, EventKey::name("player:init"));
                let init: ds_protocol::PlayerInit = bitcode::decode(&payload).unwrap();
                assert!(init.is_npc);
            }
            other => panic!("expected an event, got {other:?}"),
        }
        let action = s.encode_envelope(&json!({"namespace": "player", "event": "client_action", "data": {"action": "sit"}})).unwrap();
        match decode_client(&s.compression.unwrap(&action, MAX_FRAME).unwrap(), MAX_FRAME).unwrap() {
            ClientFrame::Event { key, payload } => {
                assert_eq!(key, EventKey::Id(1), "declared events go by id");
                let data: ds_protocol::DynValue = bitcode::decode(&payload).unwrap();
                assert_eq!(data.to_value().unwrap(), json!({"action": "sit"}));
            }
            other => panic!("expected an event, got {other:?}"),
        }
    }
}
