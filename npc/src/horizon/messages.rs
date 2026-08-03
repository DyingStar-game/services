//! Wire-format types for the Horizon WebSocket protocol used by Godot clients.
//!
//! Reference: `DyingStar/server/client.gd`.
//!
//! Outbound envelope: `{ namespace, event, data }`.
//! Inbound: three shapes (see `InboundFrame`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── Outbound ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct OutboundEnvelope<'a> {
    pub namespace: &'a str,
    pub event: &'a str,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

// ─── Inbound ───────────────────────────────────────────────────────────────

/// Raw inbound frame as received over the WS. We deserialize it with
/// `serde_json::Value` first and then call [`crate::horizon::gorc::standardize`]
/// to normalise it into a [`StandardizedFrame`].
#[derive(Debug, Clone, Deserialize)]
pub struct RawFrame {
    #[serde(default)]
    #[serde(rename = "type")]
    pub type_: Option<String>,

    #[serde(default)]
    pub event_type: Option<String>,

    #[serde(default)]
    pub object_id: Option<String>,

    #[serde(default)]
    pub object_type: Option<String>,

    #[serde(default)]
    pub player_id: Option<String>,

    #[serde(default)]
    pub channel: Option<u32>,

    #[serde(default)]
    pub timestamp: Option<u64>,

    // Payload may be in any of these fields depending on the message kind.
    #[serde(default)]
    pub zone_data: Option<Value>,
    #[serde(default)]
    pub object_data: Option<Value>,
    #[serde(default)]
    pub data: Option<Value>,

    // System-only fields.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub player_id_init: Option<String>,
}

/// Normalised view of an inbound frame after `standardize()`.
#[derive(Debug, Clone)]
pub enum StandardizedFrame {
    /// `{type:"init_ack", player_id}` — auth succeeded.
    InitAck { player_id: String },

    /// `{type:"error", message}` — fatal server error.
    Error { message: String },

    /// A GORC-style frame. `kind` is the dispatched event
    /// (`gorc_zone_enter` | `gorc_zone_exit` | `update` | `update_property`).
    Gorc(GorcFrame),

    /// Anything else (e.g. `livekit_token`); ignored by the NPC for now.
    Other { name: String, payload: Value },
}

/// Normalised GORC frame. `data` is the merged per-channel payload (from
/// whichever of `zone_data`/`object_data`/`data`/`data.object_data` was set).
#[derive(Debug, Clone)]
pub struct GorcFrame {
    pub kind: GorcKind,
    pub channel: u32,
    pub object_id: String,
    pub object_type: String,
    pub player_id: Option<String>,
    pub timestamp: Option<u64>,
    pub data: Value,
    /// Sub-event for `update` frames (e.g. `"move"`).
    pub sub_event: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GorcKind {
    ZoneEnter,
    ZoneExit,
    Update,
    UpdateProperty,
}

impl GorcKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "gorc_zone_enter" => Some(Self::ZoneEnter),
            "gorc_zone_exit" => Some(Self::ZoneExit),
            // Horizon emits "move" on channel 0 for player transforms; "update"
            // is not a name it ever sends.
            "move" | "update" => Some(Self::Update),
            "update_property" => Some(Self::UpdateProperty),
            _ => None,
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> RawFrame {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn parses_init_ack() {
        let raw = parse(json!({"type":"init_ack","player_id":"abc"}));
        assert_eq!(raw.type_.as_deref(), Some("init_ack"));
    }

    #[test]
    fn parses_zone_enter_prop() {
        let raw = parse(json!({
            "channel": 0,
            "object_id": "dc804655-7af7-4172-9dd0-47d8497b722e",
            "object_type": "miningrock",
            "player_id": "371a6b85-d941-454a-8b91-a28eb1fbe188",
            "timestamp": 1763037787u64,
            "type": "gorc_zone_enter",
            "zone_data": {
                "parent_id": "3388a817-f3ef-421d-b10f-4325e105628e",
                "position": { "x": -2209850, "y": 1.2, "z": 46.7425 },
                "rotation": { "x": 0, "y": 0, "z": 0 }
            }
        }));
        assert_eq!(raw.object_type.as_deref(), Some("miningrock"));
        assert!(raw.zone_data.is_some());
    }

    #[test]
    fn parses_player_move_update() {
        let raw = parse(json!({
            "object_type": "player",
            "event_type": "update",
            "channel": 0,
            "object_id": "u",
            "data": {
                "event_type": "move",
                "position": {"x":1.0,"y":2.0,"z":3.0},
                "rotation": {"x":0.0,"y":0.0,"z":0.0}
            }
        }));
        assert_eq!(raw.event_type.as_deref(), Some("update"));
        assert!(raw.data.is_some());
    }

    #[test]
    fn parses_error() {
        let raw = parse(json!({"type":"error","message":"bad token"}));
        assert_eq!(raw.message.as_deref(), Some("bad token"));
    }
}
