//! The wire contract between the weather service, Horizon and the game — ONE definition.
//!
//! Horizon's generic bridge plugin (`horizonserver/ds_bridge`) connects to the service's `/ws` and
//! re-emits every envelope it receives into Horizon's event system. The weather service therefore
//! speaks `genericprops` directly: `create_object`, `update_object`, `delete_object`, each carrying a
//! `weather_system` object. Horizon replicates it to clients through the channels of
//! `weather_system_def.json` (the same file lives in the game's `items_def/`, in lockstep), and the
//! game instantiates `scenename` under the parent planet.
//!
//! Conventions: positions and velocities are in the PARENT PLANET's body-fixed frame (metres,
//! metres per second), because the client parents the node to the planet, which spins. Times are
//! unix seconds, the clock Horizon calibrates every client to (`Globals.sim_time()`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// Envelope of `ds_bridge` (`horizonserver/ds_bridge/src/config.rs::BridgeEventEnvelope`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BridgeEnvelope {
    /// "core", "plugin" or "bridge".
    pub event_type: String,
    pub namespace: Option<String>,
    pub name: String,
    pub payload: Value,
}

/// The object type Horizon and the game know the weather by.
pub const OBJECT_TYPE: &str = "weather_system";
/// The scene the client instantiates for one system.
pub const SCENENAME: &str = "scenes/_universe/environment/weather/weather_system.tscn";
/// Namespace UUID of the deterministic (v5) object uuids.
pub const UUID_NAMESPACE: Uuid = Uuid::from_bytes([0x2f, 0x0c, 0x91, 0x6e, 0xd8, 0x1a, 0x4f, 0x8b, 0x9b, 0x15, 0x6d, 0x3a, 0x77, 0xe2, 0x41, 0x90]);

/// A stable uuid for a system: v5 of (planet name, system id). The same system gets the same uuid
/// after a restart, so Horizon's idempotent `create_object` does not duplicate it.
pub fn system_uuid(planet_name: &str, system_id: u64) -> Uuid {
    Uuid::new_v5(&UUID_NAMESPACE, format!("{planet_name}/{system_id:016x}").as_bytes())
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Vec3f {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// What kind of system this is: decides the sign of the pressure anomaly, the sense of rotation of
/// the wind around it, and which parameters matter.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemKind {
    Depression,
    Anticyclone,
    DustStorm,
    RainFront,
}

/// The parameters a system carries beyond its geometry. Wind, dust, rain and humidity are the
/// coupled consequences of ONE system, so they travel in one object (not one object per variable).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SystemParams {
    /// Peak tangential wind of the system at full intensity, m/s.
    pub wind_peak_ms: f64,
    /// Dust optical depth added at the centre at full intensity (0 = no dust).
    pub dust_tau: f64,
    /// Precipitation rate at the centre at full intensity, mm/h (0 = dry).
    pub precipitation_mm_h: f64,
    /// Relative humidity inside the system, 0–1.
    pub humidity: f64,
}

/// The `object_data` of a `weather_system`, as Horizon and the game see it. Field names are the
/// channel properties of `weather_system_def.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WeatherSystemData {
    pub name: String,
    /// Horizon uuid of the parent planet (`_planet_<Name>` is only resolved for seed files, so the
    /// service resolves the real uuid from the celestial service).
    pub parent_id: String,
    pub scenename: String,
    pub kind: SystemKind,
    /// Current centre in the planet's body-fixed frame, metres.
    pub position: Vec3f,
    pub rotation: Vec3f,
    pub radius_m: f64,
    /// 0–1 envelope of the system's life.
    pub intensity: f64,
    pub pressure_anomaly_pa: f64,
    /// Drift of the centre, m/s in the planet frame.
    pub velocity: Vec3f,
    /// Unix time of `track[0]`.
    pub from_timestamp: f64,
    pub keyframe_dt_s: f64,
    /// Future centres every `keyframe_dt_s` from `from_timestamp`, planet frame.
    pub track: Vec<Vec3f>,
    /// Future intensities, same timing as `track`.
    pub intensity_track: Vec<f64>,
    pub params: SystemParams,
    pub born_at: f64,
    pub dies_at: f64,
}

/// Envelope creating a system in Horizon (and, through it, on every client in range).
pub fn create_envelope(uuid: Uuid, data: &WeatherSystemData) -> BridgeEnvelope {
    BridgeEnvelope {
        event_type: "plugin".into(),
        namespace: Some("genericprops".into()),
        name: "create_object".into(),
        payload: json!({ "object_type": OBJECT_TYPE, "object_uuid": uuid.to_string(), "object_data": data }),
    }
}

/// Envelope refreshing a system: the whole data again (Horizon merges in place and only broadcasts
/// the channels whose properties changed).
pub fn update_envelope(uuid: Uuid, data: &WeatherSystemData) -> BridgeEnvelope {
    BridgeEnvelope {
        event_type: "plugin".into(),
        namespace: Some("genericprops".into()),
        name: "update_object".into(),
        payload: json!({ "object_type": OBJECT_TYPE, "object_uuid": uuid.to_string(), "object_data": data }),
    }
}

/// Envelope removing a dead system. `object_data` must be present (Horizon's struct has no default).
pub fn delete_envelope(uuid: Uuid) -> BridgeEnvelope {
    BridgeEnvelope {
        event_type: "plugin".into(),
        namespace: Some("genericprops".into()),
        name: "delete_object".into(),
        payload: json!({ "object_type": OBJECT_TYPE, "object_uuid": uuid.to_string(), "object_data": {} }),
    }
}

/// `weather_system_def.json`: the channels Horizon replicates a system through. Zone 6 carries the
/// scene name and MUST exist (the client instantiates only with scenename + parent_id, and deletes
/// only on a zone-6 exit); the large arrays sit in their own low-rate channel because every send is
/// a full snapshot of its channel. Distances are from the system's global position; a system is up
/// to 1 000 km wide, and a planet ~6 000 km across, so a whole planet subscribes.
pub const DEF_JSON: &str = r#"{
    "channels": [
        {
            "zone": 0,
            "distance": 20000000.0,
            "frequency": 1.0,
            "properties": ["name", "parent_id", "kind", "position", "rotation", "radius_m", "intensity", "pressure_anomaly_pa", "velocity", "params", "born_at", "dies_at"]
        },
        {
            "zone": 1,
            "distance": 20000000.0,
            "frequency": 0.2,
            "properties": ["from_timestamp", "keyframe_dt_s", "track", "intensity_track"]
        },
        {
            "zone": 6,
            "distance": 20000001.0,
            "frequency": 1.0,
            "properties": ["scenename"]
        }
    ]
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_definition_parses_and_lists_every_data_field() {
        let def: Value = serde_json::from_str(DEF_JSON).expect("valid json");
        let mut listed: Vec<String> = Vec::new();
        for ch in def["channels"].as_array().unwrap() {
            for p in ch["properties"].as_array().unwrap() {
                listed.push(p.as_str().unwrap().to_string());
            }
        }
        let sample = WeatherSystemData {
            name: "x".into(), parent_id: "p".into(), scenename: SCENENAME.into(), kind: SystemKind::Depression,
            position: Vec3f::default(), rotation: Vec3f::default(), radius_m: 1.0, intensity: 0.5, pressure_anomaly_pa: -1.0,
            velocity: Vec3f::default(), from_timestamp: 0.0, keyframe_dt_s: 60.0, track: vec![], intensity_track: vec![],
            params: SystemParams::default(), born_at: 0.0, dies_at: 1.0,
        };
        let as_json = serde_json::to_value(&sample).unwrap();
        for key in as_json.as_object().unwrap().keys() {
            assert!(listed.contains(key), "field `{key}` is not in any channel: Horizon would drop it silently");
        }
        assert!(def["channels"].as_array().unwrap().iter().any(|c| c["zone"] == 6), "zone 6 is mandatory");
    }

    #[test]
    fn uuids_are_stable_and_distinct() {
        assert_eq!(system_uuid("SandBox", 7), system_uuid("SandBox", 7));
        assert_ne!(system_uuid("SandBox", 7), system_uuid("SandBox", 8));
        assert_ne!(system_uuid("SandBox", 7), system_uuid("Gaea", 7));
    }

    #[test]
    fn envelopes_have_the_bridge_shape() {
        let e = delete_envelope(system_uuid("SandBox", 1));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["event_type"], "plugin");
        assert_eq!(v["namespace"], "genericprops");
        assert_eq!(v["name"], "delete_object");
        assert!(v["payload"]["object_data"].is_object());
    }
}
