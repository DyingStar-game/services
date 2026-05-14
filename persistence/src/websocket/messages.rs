use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 3-D vector with decimal components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// An item stored in the persistence layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub object_type: String,
    pub uuid: String,
    pub parent_id: Option<String>,
    pub scenename: Option<String>,
    pub position: Option<Vec3>,
    pub rotation: Option<Vec3>,
    /// Remaining heterogeneous object data (without the extracted fields above).
    pub object_data: Vec<Value>,
}

// ─── WebSocket request messages ────────────────────────────────────────────

/// Incoming request payload for create_object / create_object_from_gameserver
/// events forwarded by ds_bridge from the genericprops plugin.
#[derive(Debug, Deserialize)]
pub struct GenericPropsRequest {
    pub object_type: String,
    pub object_uuid: String,
    pub object_data: serde_json::Value,
}

// ─── REST request messages ─────────────────────────────────────────────────

/// Request body for PUT /items/{uuid}.
/// The uuid comes from the URL path; this carries only the mutable fields.
#[derive(Debug, Deserialize)]
pub struct PutItemRequest {
    pub object_type: String,
    pub object_data: serde_json::Value,
}

// ─── WebSocket response messages ───────────────────────────────────────────

/// Envelope used for all outgoing WebSocket messages, matching the
/// `BridgeEventEnvelope` format defined in `ds_bridge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEventEnvelope {
    /// "core" or "plugin"
    pub event_type: String,
    /// Plugin namespace when event_type == "plugin", otherwise None
    pub namespace: Option<String>,
    /// Event name (e.g. "items_chunk", "items_end", "error")
    pub name: String,
    /// Arbitrary JSON payload — the event data
    pub payload: serde_json::Value,
}
