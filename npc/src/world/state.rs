use std::time::Instant;

use serde_json::Value;

use crate::horizon::messages::Vec3;

#[derive(Debug, Clone)]
pub struct PlayerState {
    pub uuid: String,
    /// Display name Horizon replicates on channel 0. `None` until a frame
    /// carrying it arrives.
    pub name: Option<String>,
    pub parent_id: Option<String>,
    pub position: Vec3,
    pub rotation: Vec3,
    pub last_seen: Instant,
}

impl PlayerState {
    pub fn new(uuid: String) -> Self {
        Self {
            uuid,
            name: None,
            parent_id: None,
            position: Vec3::default(),
            rotation: Vec3::default(),
            last_seen: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObjectState {
    pub uuid: String,
    pub object_type: String,
    pub parent_id: Option<String>,
    pub position: Vec3,
    pub rotation: Vec3,
    /// Latest value of every replicated property, merged across frames: each
    /// frame's keys overwrite, so the newest frame wins for the keys it
    /// carries. (This used to be stored per channel, but the same key can
    /// arrive on several channels — the zone-enter snapshot AND a live
    /// update — and a per-channel lookup let whichever channel iterated
    /// first shadow the fresh value with the stale one, forever.)
    pub props: serde_json::Map<String, Value>,
    pub scenename: Option<String>,
    pub last_seen: Instant,
}

impl ObjectState {
    pub fn new(uuid: String, object_type: String) -> Self {
        Self {
            uuid,
            object_type,
            parent_id: None,
            position: Vec3::default(),
            rotation: Vec3::default(),
            props: serde_json::Map::new(),
            scenename: None,
            last_seen: Instant::now(),
        }
    }

    /// Latest replicated value of `key`, whichever frame carried it last.
    pub fn property(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }
}

/// Extract `{position:{x,y,z}}` from a JSON value if present.
pub fn parse_vec3(v: &Value, key: &str) -> Option<Vec3> {
    let o = v.get(key)?.as_object()?;
    Some(Vec3 {
        x: o.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0),
        y: o.get("y").and_then(|y| y.as_f64()).unwrap_or(0.0),
        z: o.get("z").and_then(|z| z.as_f64()).unwrap_or(0.0),
    })
}
