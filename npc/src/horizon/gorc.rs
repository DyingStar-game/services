//! Normalisation of inbound Horizon frames. Port of the
//! `_standardize_object` logic from `DyingStar/server/client.gd`.

use serde_json::Value;

use super::messages::{GorcFrame, GorcKind, RawFrame, StandardizedFrame};

/// Take a parsed [`RawFrame`] and dispatch it into a [`StandardizedFrame`].
pub fn standardize(raw: RawFrame) -> StandardizedFrame {
    // System shape A: `type` field present.
    if let Some(t) = raw.type_.as_deref() {
        match t {
            "init_ack" => {
                let pid = raw.player_id.clone().unwrap_or_default();
                return StandardizedFrame::InitAck { player_id: pid };
            }
            "error" => {
                return StandardizedFrame::Error {
                    message: raw.message.clone().unwrap_or_default(),
                };
            }
            // gorc_zone_enter / gorc_zone_exit can ride on either `type` or `event_type`.
            "gorc_zone_enter" | "gorc_zone_exit" => {
                if let Some(g) = build_gorc(t, &raw) {
                    return StandardizedFrame::Gorc(g);
                }
            }
            _ => {}
        }
    }

    // GORC shape B: dispatch on `event_type`.
    if let Some(et) = raw.event_type.as_deref() {
        if GorcKind::from_str(et).is_some() {
            if let Some(g) = build_gorc(et, &raw) {
                return StandardizedFrame::Gorc(g);
            }
        }
        // Service shape C: only `event_type`, no GORC mapping.
        return StandardizedFrame::Other {
            name: et.to_string(),
            payload: raw.data.unwrap_or(Value::Null),
        };
    }

    // Unknown frame: surface as `Other` with whatever name we have.
    StandardizedFrame::Other {
        name: raw.type_.unwrap_or_else(|| "unknown".to_string()),
        payload: Value::Null,
    }
}

fn build_gorc(name: &str, raw: &RawFrame) -> Option<GorcFrame> {
    let kind = GorcKind::from_str(name)?;
    let object_id = raw.object_id.clone()?;
    let object_type = raw.object_type.clone().unwrap_or_default();

    // Payload precedence mirrors the GDScript: zone_data → object_data → data → data.object_data.
    let mut data: Value = Value::Null;
    let mut sub_event: Option<String> = None;
    if let Some(v) = raw.zone_data.as_ref() {
        data = v.clone();
    } else if let Some(v) = raw.object_data.as_ref() {
        data = v.clone();
    } else if let Some(v) = raw.data.as_ref() {
        // `data` for player update frames carries an inner `event_type` plus position/rotation.
        if let Some(obj) = v.as_object() {
            if let Some(inner_obj) = obj.get("object_data") {
                data = inner_obj.clone();
            } else {
                data = v.clone();
            }
            if let Some(se) = obj.get("event_type").and_then(|s| s.as_str()) {
                sub_event = Some(se.to_string());
            }
        } else {
            data = v.clone();
        }
    }

    Some(GorcFrame {
        kind,
        channel: raw.channel.unwrap_or(0),
        object_id,
        object_type,
        player_id: raw.player_id.clone(),
        timestamp: raw.timestamp,
        data,
        sub_event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(v: serde_json::Value) -> RawFrame {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn standardises_init_ack() {
        let f = standardize(raw(json!({"type":"init_ack","player_id":"u1"})));
        match f {
            StandardizedFrame::InitAck { player_id } => assert_eq!(player_id, "u1"),
            _ => panic!("expected InitAck"),
        }
    }

    #[test]
    fn standardises_zone_enter_uses_zone_data() {
        let f = standardize(raw(json!({
            "type":"gorc_zone_enter",
            "channel":6,
            "object_id":"o1",
            "object_type":"ship",
            "zone_data":{"scenename":"res://x.tscn","position":{"x":1.0,"y":2.0,"z":3.0}}
        })));
        match f {
            StandardizedFrame::Gorc(g) => {
                assert_eq!(g.kind, GorcKind::ZoneEnter);
                assert_eq!(g.channel, 6);
                assert_eq!(g.object_id, "o1");
                assert_eq!(g.object_type, "ship");
                assert_eq!(
                    g.data.get("scenename").and_then(|s| s.as_str()),
                    Some("res://x.tscn")
                );
            }
            _ => panic!("expected Gorc"),
        }
    }

    #[test]
    fn standardises_player_move_extracts_sub_event() {
        let f = standardize(raw(json!({
            "event_type":"update",
            "object_type":"player",
            "channel":0,
            "object_id":"p",
            "data":{
                "event_type":"move",
                "position":{"x":1.0,"y":2.0,"z":3.0},
                "rotation":{"x":0.0,"y":0.0,"z":0.0}
            }
        })));
        match f {
            StandardizedFrame::Gorc(g) => {
                assert_eq!(g.kind, GorcKind::Update);
                assert_eq!(g.sub_event.as_deref(), Some("move"));
                assert_eq!(g.data.get("position").is_some(), true);
            }
            _ => panic!("expected Gorc"),
        }
    }

    #[test]
    fn standardises_zone_exit() {
        let f = standardize(raw(json!({
            "type":"gorc_zone_exit",
            "channel":0,
            "object_id":"p",
            "object_type":"GorcPlayer"
        })));
        match f {
            StandardizedFrame::Gorc(g) => {
                assert_eq!(g.kind, GorcKind::ZoneExit);
                assert_eq!(g.channel, 0);
            }
            _ => panic!("expected Gorc"),
        }
    }

    #[test]
    fn standardises_error() {
        let f = standardize(raw(json!({"type":"error","message":"oops"})));
        match f {
            StandardizedFrame::Error { message } => assert_eq!(message, "oops"),
            _ => panic!("expected Error"),
        }
    }
}
