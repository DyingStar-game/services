use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

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
    /// Latest line this entity is "saying" — the replicated `conversation`
    /// property (see the depot reception negotiation). `None` once the speaker
    /// clears it. Lets an NPC HEAR a nearby peer (players and other NPCs alike),
    /// which is otherwise impossible: dialog is outbound-only.
    pub conversation: Option<String>,
    /// When [`conversation`](Self::conversation) was last set, so a listener can
    /// ignore stale lines.
    pub conversation_at: Option<Instant>,
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
            conversation: None,
            conversation_at: None,
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
    /// When each property last took a NEW value — not when it was last
    /// replicated. Lets an action ask "how long has the counter been holding
    /// this box?" without polling state of its own, and stays accurate while
    /// the NPC is busy, since the session loop keeps applying frames.
    pub props_changed_at: HashMap<String, Instant>,
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
            props_changed_at: HashMap::new(),
            scenename: None,
            last_seen: Instant::now(),
        }
    }

    /// Latest replicated value of `key`, whichever frame carried it last.
    pub fn property(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }

    /// How long `key` has held its current value. `None` when the property has
    /// never been replicated.
    pub fn property_age(&self, key: &str) -> Option<Duration> {
        self.props_changed_at.get(key).map(|t| t.elapsed())
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
