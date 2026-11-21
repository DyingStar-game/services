//! Driving support for NPC actions: the route-following controller, and how
//! to read a replicated vehicle into planet-frame positions for it.
//!
//! Frames: villages, garages and vehicles are parented to the planet, whose
//! frame is the one the road network lives in (`world_data`). A vehicle
//! replicates its position LOCAL to its parent and its rotation as Godot
//! Euler angles (YXZ), on channel 0 as `update_property`, up to 30 Hz near
//! the observer — the NPC driving it is as near as it gets.

pub mod controller;
pub mod lanes;
pub mod obstacles;
pub mod offroad;
pub mod traffic;
#[cfg(test)]
pub mod testkit;

use serde_json::Value;

use crate::{
    horizon::messages::Vec3,
    world::{state::ObjectState, view::WorldView},
    world_data::geo,
};

/// How far (fraction of the radius) a position may lie from the planet's
/// reference sphere and still be read as "in the planet frame". Local offsets
/// inside buildings or vehicles are metres; planet-frame positions are
/// millions of metres from the centre, so the two never get confused.
const SHELL: f64 = 0.02;

fn in_shell(p: Vec3, radius: f64) -> bool {
    let r = geo::length(p);
    r > radius * (1.0 - SHELL) && r < radius * (1.0 + SHELL)
}

/// `local` (expressed in object `frame`'s local frame) in the planet frame,
/// composing each ancestor's position AND rotation until the result lands on
/// the planet's surface. `None` while a needed ancestor is not in view.
pub fn object_point_to_planet(world: &WorldView, frame: &str, local: Vec3, radius: f64) -> Option<Vec3> {
    let mut acc = local;
    let mut cursor = Some(frame.to_string());
    for _ in 0..16 {
        let id = cursor.filter(|c| !c.is_empty())?;
        let (pos, rot, parent) = {
            let o = world.objects.get(&id)?;
            (o.position, o.rotation, o.parent_id.clone())
        };
        acc = geo::add(pos, geo::rotate_euler_yxz(acc, rot));
        if in_shell(acc, radius) {
            return Some(acc);
        }
        cursor = parent;
    }
    None
}

/// A direction local to object `frame`, in the planet frame.
pub fn object_dir_to_planet(world: &WorldView, frame: &str, dir: Vec3, radius: f64) -> Option<Vec3> {
    let o = object_point_to_planet(world, frame, Vec3::default(), radius)?;
    let p = object_point_to_planet(world, frame, dir, radius)?;
    Some(geo::normalized(geo::sub(p, o)))
}

/// Where an object sits, planet frame.
pub fn object_planet_position(world: &WorldView, uuid: &str, radius: f64) -> Option<Vec3> {
    object_point_to_planet(world, uuid, Vec3::default(), radius)
}

/// The NPC's own position, planet frame.
pub fn my_planet_position(world: &WorldView, radius: f64) -> Option<Vec3> {
    let local = world.my_position()?;
    if in_shell(local, radius) {
        return Some(local);
    }
    let parent = world.my_parent()?;
    object_point_to_planet(world, &parent, local, radius)
}

/// Replicated driving state of a vehicle (`Vehicle.full_state`, channel 0).
#[derive(Debug, Clone, Default)]
pub struct VehicleState {
    pub pilot_uuid: String,
    /// Signed forward speed, m/s (from `velocity_local`, else unsigned `speed`).
    pub forward_mps: f64,
    pub steering_rad: Option<f64>,
    pub engine: bool,
    pub handbrake: bool,
}

impl VehicleState {
    pub fn read(o: &ObjectState) -> Self {
        let forward_mps = o
            .property("velocity_local")
            .and_then(|v| v.get("z").and_then(Value::as_f64))
            .map(|z| -z) // the cab faces -Z
            .or_else(|| o.property("speed").and_then(Value::as_f64).map(|kmh| kmh / 3.6))
            .unwrap_or(0.0);
        Self {
            pilot_uuid: o.property("pilot_uuid").and_then(Value::as_str).unwrap_or("").to_string(),
            forward_mps,
            steering_rad: o.property("steering").and_then(Value::as_f64),
            engine: o.property("engine").and_then(Value::as_bool).unwrap_or(false),
            handbrake: o.property("handbrake").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

/// Whether `door_id` is open on a replicated vehicle (`doors` map).
pub fn door_open(o: &ObjectState, door_id: &str) -> bool {
    o.property("doors")
        .and_then(|d| d.get(door_id))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Who occupies `seat` on a replicated vehicle (`seats` map; "" = free).
pub fn seat_occupant(o: &ObjectState, seat: &str) -> Option<String> {
    o.property("seats")
        .and_then(|s| s.get(seat))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::horizon::messages::{GorcFrame, GorcKind};

    const R: f64 = 6_356_000.0;

    fn enter(world: &WorldView, uuid: &str, ty: &str, data: Value) {
        world.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: uuid.into(),
            object_type: ty.into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data,
        });
    }

    #[test]
    fn composes_rotation_up_to_the_planet_frame() {
        let w = WorldView::new("me".into());
        let base = geo::scale(geo::lonlat_to_unit(10.0, 20.0), R);
        // A garage on the planet, turned a quarter round its Y.
        enter(&w, "garage", "simple_building", json!({
            "parent_id": "planet-uuid",
            "position": {"x": base.x, "y": base.y, "z": base.z},
            "rotation": {"x": 0.0, "y": std::f64::consts::FRAC_PI_2, "z": 0.0},
        }));
        let p = object_point_to_planet(&w, "garage", Vec3 { x: 1.0, y: 0.0, z: 0.0 }, R).unwrap();
        // Yaw +90° sends local +X to -Z.
        assert!((p.x - base.x).abs() < 1e-6 && (p.z - (base.z - 1.0)).abs() < 1e-6, "{p:?}");
        let d = object_dir_to_planet(&w, "garage", Vec3 { x: 0.0, y: 0.0, z: -1.0 }, R).unwrap();
        assert!((d.x + 1.0).abs() < 1e-6, "forward {d:?}");
    }

    #[test]
    fn reads_the_vehicle_state() {
        let w = WorldView::new("me".into());
        enter(&w, "truck", "vehicle", json!({
            "pilot_uuid": "me",
            "velocity_local": {"x": 0.0, "y": 0.0, "z": -5.0},
            "steering": 0.12,
            "engine": true,
            "doors": {"front_l_door": true},
            "seats": {"seat_driver": "me", "seat_passenger": ""},
        }));
        let o = w.objects.get("truck").unwrap();
        let s = VehicleState::read(&o);
        assert_eq!(s.forward_mps, 5.0);
        assert_eq!(s.steering_rad, Some(0.12));
        assert!(s.engine && !s.handbrake);
        assert!(door_open(&o, "front_l_door") && !door_open(&o, "front_r_door"));
        assert_eq!(seat_occupant(&o, "seat_driver").as_deref(), Some("me"));
    }
}
