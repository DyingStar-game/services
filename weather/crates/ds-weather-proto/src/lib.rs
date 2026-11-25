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

// ------------------------------------------------------------------ weather_field
//
// The near-ground fields of the atmosphere model (crate `weather-atmosphere`), one object per
// simulated body, parented to the planet at its centre. Contract (2026-11-25): 128 × 64
// equirectangular grid, column i at longitude -180 + (i + 0.5) 360 / w increasing EASTWARD (the
// convention of `/point` and `weather_core::spin::dir_of`), row j at latitude 90 - (j + 0.5) 180 / h
// (row 0 = north); five quantities, each quantised to u16 with its own `[offset, scale]`
// (value = offset + scale * u16), all packed into ONE base64 string (standard alphabet, padded),
// little-endian, quantity-major then row-major: index = ((q * h) + j) * w + i.

/// The object type of the near-ground fields.
pub const FIELD_OBJECT_TYPE: &str = "weather_field";
/// The scene the client instantiates for the fields of one body.
pub const FIELD_SCENENAME: &str = "scenes/_universe/environment/weather/weather_field.tscn";
pub const FIELD_GRID_W: usize = 128;
pub const FIELD_GRID_H: usize = 64;
/// The quantities, in this order: wind 10 m above the ground east and north (m/s), dust optical
/// depth of the column, air temperature near the ground (K), pressure reduced to the reference
/// sphere (Pa).
pub const FIELD_QUANTITIES: [&str; 5] = ["wind_east", "wind_north", "dust_tau", "t_air", "p_sea"];

/// A stable uuid for the fields of a body (v5, same namespace as the systems).
pub fn field_uuid(planet_name: &str) -> Uuid {
    Uuid::new_v5(&UUID_NAMESPACE, format!("{planet_name}/weather_field").as_bytes())
}

/// The `object_data` of a `weather_field`. Field names are the channel properties of
/// `weather_field_def.json` (`FIELD_DEF_JSON`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WeatherFieldData {
    pub name: String,
    /// Horizon uuid of the parent planet.
    pub parent_id: String,
    pub scenename: String,
    /// (0, 0, 0): the planet's centre, in its frame.
    pub position: Vec3f,
    pub rotation: Vec3f,
    pub grid_w: u32,
    pub grid_h: u32,
    /// `FIELD_QUANTITIES`.
    pub quantities: Vec<String>,
    /// One `[offset, scale]` per quantity: value = offset + scale * u16.
    pub scales: Vec<[f64; 2]>,
    /// Unix time (s) at which these fields are valid (the end of the model chunk).
    pub valid_at: f64,
    /// Model time in model days (diagnostic).
    pub model_day: f64,
    /// The u16 values, base64.
    pub fields: String,
}

/// Quantises quantity-major values (`n_q` blocks of `w * h`) to u16 with one `[offset, scale]` per
/// quantity covering its min..max (a flat or empty quantity gets scale 1e-6; non-finite values
/// count as the offset), and packs them little-endian in base64.
pub fn encode_fields(values: &[f32], n_q: usize) -> (Vec<[f64; 2]>, String) {
    let per = values.len() / n_q.max(1);
    let mut scales = Vec::with_capacity(n_q);
    let mut bytes = Vec::with_capacity(values.len() * 2);
    for q in 0..n_q {
        let block = &values[q * per..(q + 1) * per];
        let (lo, hi) = block.iter().filter(|x| x.is_finite()).fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x as f64), b.max(x as f64)));
        let (offset, scale) = if lo <= hi && hi > lo { (lo, (hi - lo) / 65535.0) } else if lo <= hi { (lo, 1e-6) } else { (0.0, 1e-6) };
        for &x in block {
            let u = if x.is_finite() { ((x as f64 - offset) / scale).round().clamp(0.0, 65535.0) as u16 } else { 0 };
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        scales.push([offset, scale]);
    }
    (scales, base64_encode(&bytes))
}

/// The inverse of `encode_fields`: the values, quantity-major.
pub fn decode_fields(scales: &[[f64; 2]], b64: &str) -> Option<Vec<f64>> {
    let bytes = base64_decode(b64)?;
    let n_q = scales.len().max(1);
    let per = bytes.len() / 2 / n_q;
    Some(bytes.chunks_exact(2).enumerate().map(|(k, c)| {
        let [o, s] = scales[(k / per).min(n_q - 1)];
        o + s * u16::from_le_bytes([c[0], c[1]]) as f64
    }).collect())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64, standard alphabet, with padding (RFC 4648).
pub fn base64_encode(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Decodes `base64_encode`'s output (None on a malformed string).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if s.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| B64.iter().position(|&x| x == c).map(|v| v as u32);
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for c in s.chunks(4) {
        let pad = c.iter().rev().take_while(|&&x| x == b'=').count();
        let mut n = 0u32;
        for (k, &x) in c.iter().enumerate() {
            n |= if k >= 4 - pad { 0 } else { val(x)? } << (18 - 6 * k);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

fn object_envelope(name: &str, object_type: &str, uuid: Uuid, data: Value) -> BridgeEnvelope {
    BridgeEnvelope {
        event_type: "plugin".into(),
        namespace: Some("genericprops".into()),
        name: name.into(),
        payload: json!({ "object_type": object_type, "object_uuid": uuid.to_string(), "object_data": data }),
    }
}

/// Envelope creating the fields of a body.
pub fn field_create_envelope(uuid: Uuid, data: &WeatherFieldData) -> BridgeEnvelope {
    object_envelope("create_object", FIELD_OBJECT_TYPE, uuid, serde_json::to_value(data).unwrap_or_default())
}

/// Envelope refreshing the fields of a body (the whole data; Horizon broadcasts the changed channels).
pub fn field_update_envelope(uuid: Uuid, data: &WeatherFieldData) -> BridgeEnvelope {
    object_envelope("update_object", FIELD_OBJECT_TYPE, uuid, serde_json::to_value(data).unwrap_or_default())
}

/// Envelope removing the fields of a body.
pub fn field_delete_envelope(uuid: Uuid) -> BridgeEnvelope {
    object_envelope("delete_object", FIELD_OBJECT_TYPE, uuid, json!({}))
}

/// `weather_field_def.json`: the channels Horizon replicates the fields through (identical in
/// horizonserver `ds_genericprops/props/` and the game's `items_def/`). Zone 0: the frame (grid
/// size, quantities, scales); zone 1, low rate: the time and the data; zone 6: the scene (mandatory,
/// delete channel). The object sits at the planet's centre: 20 000 km covers the whole planet.
pub const FIELD_DEF_JSON: &str = r#"{
    "channels": [
        {
            "zone": 0,
            "distance": 20000000.0,
            "frequency": 1.0,
            "properties": ["name", "parent_id", "position", "rotation", "grid_w", "grid_h", "quantities", "scales"]
        },
        {
            "zone": 1,
            "distance": 20000000.0,
            "frequency": 0.2,
            "properties": ["valid_at", "model_day", "fields"]
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

    fn listed(def_json: &str) -> Vec<String> {
        let def: Value = serde_json::from_str(def_json).expect("valid json");
        assert!(def["channels"].as_array().unwrap().iter().any(|c| c["zone"] == 6), "zone 6 is mandatory");
        def["channels"].as_array().unwrap().iter().flat_map(|ch| ch["properties"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string())).collect()
    }

    fn sample_field() -> WeatherFieldData {
        let values: Vec<f32> = (0..5 * FIELD_GRID_W * FIELD_GRID_H).map(|k| (k % 977) as f32 * 0.37 - 50.0).collect();
        let (scales, fields) = encode_fields(&values, 5);
        WeatherFieldData {
            name: "Sandbox-weather_field".into(),
            parent_id: "p".into(),
            scenename: FIELD_SCENENAME.into(),
            position: Vec3f::default(),
            rotation: Vec3f::default(),
            grid_w: FIELD_GRID_W as u32,
            grid_h: FIELD_GRID_H as u32,
            quantities: FIELD_QUANTITIES.iter().map(|s| s.to_string()).collect(),
            scales,
            valid_at: 1.8e9,
            model_day: 250.04,
            fields,
        }
    }

    #[test]
    fn every_field_property_is_in_a_channel() {
        let listed = listed(FIELD_DEF_JSON);
        let as_json = serde_json::to_value(sample_field()).unwrap();
        for key in as_json.as_object().unwrap().keys() {
            assert!(listed.contains(key), "field `{key}` is not in any channel: Horizon would drop it silently");
        }
        assert!(!listed.iter().any(|p| p == "type" || p == "uuid"), "envelope keys must not be properties");
    }

    /// 5 x 128 x 64 u16 = 81 920 bytes = 109 228 base64 characters; values come back within half
    /// a quantisation step; a flat quantity gets scale 1e-6.
    #[test]
    fn fields_round_trip() {
        let mut values: Vec<f32> = (0..5 * FIELD_GRID_W * FIELD_GRID_H).map(|k| ((k as f32) * 0.013).sin() * 30.0 + 280.0).collect();
        let per = FIELD_GRID_W * FIELD_GRID_H;
        values[2 * per..3 * per].iter_mut().for_each(|x| *x = 0.25);
        let (scales, b64) = encode_fields(&values, 5);
        assert_eq!(b64.len(), 109_228);
        assert_eq!(base64_decode(&b64).unwrap().len(), 81_920);
        assert_eq!(scales[2], [0.25f32 as f64, 1e-6]);
        let back = decode_fields(&scales, &b64).unwrap();
        for (k, (a, b)) in values.iter().zip(&back).enumerate() {
            let step = scales[k / per][1];
            assert!((*a as f64 - b).abs() <= 0.5 * step + 1e-9, "{k}: {a} vs {b}");
        }
        // Index convention: ((q * h) + j) * w + i, little-endian.
        let raw = base64_decode(&b64).unwrap();
        let (q, j, i) = (3, 10, 77);
        let idx = (q * FIELD_GRID_H + j) * FIELD_GRID_W + i;
        let u = u16::from_le_bytes([raw[2 * idx], raw[2 * idx + 1]]) as f64;
        assert!((scales[q][0] + scales[q][1] * u - values[idx] as f64).abs() <= scales[q][1]);
    }

    #[test]
    fn base64_matches_rfc4648() {
        for (raw, enc) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(raw.as_bytes()), enc);
            assert_eq!(base64_decode(enc).unwrap(), raw.as_bytes());
        }
    }

    #[test]
    fn field_envelopes_and_uuid() {
        assert_eq!(field_uuid("Sandbox"), field_uuid("Sandbox"));
        assert_ne!(field_uuid("Sandbox"), field_uuid("Gaea"));
        let v = serde_json::to_value(field_update_envelope(field_uuid("Sandbox"), &sample_field())).unwrap();
        assert_eq!(v["name"], "update_object");
        assert_eq!(v["payload"]["object_type"], FIELD_OBJECT_TYPE);
        assert_eq!(v["payload"]["object_data"]["grid_w"], 128);
        let d = serde_json::to_value(field_delete_envelope(field_uuid("Sandbox"))).unwrap();
        assert!(d["payload"]["object_data"].is_object());
    }

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
