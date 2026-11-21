//! What a driving NPC sees on and around the road (ADR services/0012): the
//! replicated vehicles, people and props near its truck, as boxes in the planet
//! frame with a velocity.
//!
//! Nothing replicates a size, so a box comes from the scene's exported
//! `footprint` (`tools/npc/npc_layout_export.gd`) when there is one, else from a
//! default per object type. Nothing but vehicles replicates a velocity either,
//! so a [`Tracker`] estimates it from successive positions.

use std::collections::HashMap;

use serde_json::Value;

use super::object_point_to_planet;
use crate::{
    horizon::messages::Vec3,
    scene_layouts::SceneLayouts,
    world::view::WorldView,
    world_data::geo,
};

/// What kind of thing is in the way: it decides how to deal with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Vehicle,
    Person,
    Prop,
}

/// A thing near the truck, planet frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Obstacle {
    pub id: String,
    pub kind: Kind,
    /// Centre of its box.
    pub pos: Vec3,
    /// Its forward direction (unit, roughly horizontal).
    pub axis: Vec3,
    /// Half its width (across `axis`) and half its length (along it), metres.
    pub half_w: f64,
    pub half_l: f64,
    /// Velocity, m/s.
    pub vel: Vec3,
    /// Someone is there to move it: a person, or a vehicle with a pilot or a
    /// passenger aboard.
    pub occupied: bool,
}

/// Object types that can stand on a road. Buildings, depots, villages, lifts
/// and the like are not: the village footprints already route around them.
const ROAD_TYPES: &[(&str, Kind, f64, f64)] = &[
    // (object_type, kind, default width, default length), for scenes without
    // an exported footprint — the NPC truck's is 2.19 × 3.65 m.
    ("vehicle", Kind::Vehicle, 2.2, 3.7),
    ("box", Kind::Prop, 0.8, 0.8),
    ("crate_container", Kind::Prop, 1.3, 2.5),
    ("miningrock", Kind::Prop, 1.6, 1.6),
    ("food", Kind::Prop, 0.8, 0.8),
    ("tool", Kind::Prop, 0.8, 0.8),
    ("vehicle_component", Kind::Prop, 0.8, 0.8),
];

/// A person (player or NPC) on foot.
const PERSON_SIZE_M: f64 = 0.6;

/// Velocity estimates from successive positions, per object.
#[derive(Debug, Default)]
pub struct Tracker {
    last: HashMap<String, (f64, Vec3)>,
    vel: HashMap<String, Vec3>,
}

impl Tracker {
    /// Feed a position seen at time `t` (seconds); returns the smoothed
    /// velocity. Samples closer than 0.2 s are skipped (the replicated position
    /// would be the same frame); jumps (> 60 m/s) reset the estimate.
    pub fn update(&mut self, id: &str, t: f64, pos: Vec3) -> Vec3 {
        let v = match self.last.get(id) {
            Some(&(t0, p0)) if t - t0 >= 0.2 => {
                let raw = geo::scale(geo::sub(pos, p0), 1.0 / (t - t0));
                let v = if geo::length(raw) > 60.0 {
                    Vec3::default()
                } else {
                    let prev = self.vel.get(id).copied().unwrap_or(raw);
                    geo::add(geo::scale(prev, 0.5), geo::scale(raw, 0.5))
                };
                self.last.insert(id.to_string(), (t, pos));
                v
            }
            Some(_) => return self.vel.get(id).copied().unwrap_or_default(),
            None => {
                // First sight: nothing to estimate from yet.
                self.last.insert(id.to_string(), (t, pos));
                return Vec3::default();
            }
        };
        self.vel.insert(id.to_string(), v);
        v
    }

    /// Forget objects not seen since `t - 10 s`.
    pub fn prune(&mut self, t: f64) {
        self.last.retain(|_, (t0, _)| t - *t0 < 10.0);
        let last = &self.last;
        self.vel.retain(|id, _| last.contains_key(id));
    }
}

/// Where a position replicated local to `parent` sits in the planet frame (it
/// may already be: things on the ground are parented to the planet).
fn to_planet(world: &WorldView, parent: Option<&str>, local: Vec3, radius: f64) -> Option<Vec3> {
    let r = geo::length(local);
    if r > radius * 0.98 && r < radius * 1.02 {
        return Some(local);
    }
    object_point_to_planet(world, parent?, local, radius)
}

/// Everything that may be in the truck's way within `range_m` of `around`:
/// vehicles, people on foot and props — not the truck itself, nor what rides
/// in or on it. `t` is the time in seconds, for the velocity tracker.
pub fn snapshot(
    world: &WorldView,
    truck: &str,
    around: Vec3,
    range_m: f64,
    radius: f64,
    tracker: &mut Tracker,
    t: f64,
) -> Vec<Obstacle> {
    let layouts = SceneLayouts::shared();
    let fwd = Vec3 { x: 0.0, y: 0.0, z: -1.0 };
    let mut out = Vec::new();

    // Objects.
    let objects: Vec<(String, String, Option<String>, Option<String>, Option<Value>, bool)> = world
        .objects
        .iter()
        .filter(|o| o.uuid != truck && o.parent_id.as_deref() != Some(truck))
        .filter(|o| ROAD_TYPES.iter().any(|(ty, ..)| *ty == o.object_type))
        .map(|o| {
            (o.uuid.clone(), o.object_type.clone(), o.scenename.clone(), o.parent_id.clone(), o.property("velocity_local").cloned(), occupied(&o))
        })
        .collect();
    for (id, ty, scene, parent, vel_local, occupied) in objects {
        // Something carried inside another vehicle travels with it.
        if parent.as_deref().is_some_and(|p| world.objects.get(p).is_some_and(|po| po.object_type == "vehicle")) {
            continue;
        }
        let &(_, kind, w, l) = ROAD_TYPES.iter().find(|(t, ..)| *t == ty).unwrap();
        let footprint = scene.as_deref().and_then(|s| layouts.get(s)).and_then(|l| l.footprint);
        let centre_local = footprint.map(|f| Vec3 { x: f.center[0], y: f.center[1], z: f.center[2] }).unwrap_or_default();
        let (half_w, half_l) = footprint.map(|f| (f.size[0] / 2.0, f.size[2] / 2.0)).unwrap_or((w / 2.0, l / 2.0));
        let Some(pos) = object_point_to_planet(world, &id, centre_local, radius) else { continue };
        if geo::arc_m(pos, around, radius) > range_m {
            continue;
        }
        let origin = object_point_to_planet(world, &id, Vec3::default(), radius).unwrap_or(pos);
        let axis = object_point_to_planet(world, &id, fwd, radius)
            .map(|p| geo::normalized(geo::sub(p, origin)))
            .unwrap_or(fwd);
        let vel = match vel_local.as_ref().and_then(parse_vec3) {
            // Body-frame velocity, turned into the planet frame.
            Some(vl) if geo::length(vl) > 1e-3 => {
                let tip = object_point_to_planet(world, &id, vl, radius).unwrap_or(origin);
                let v = geo::sub(tip, origin);
                tracker.update(&id, t, pos);
                v
            }
            Some(_) => {
                tracker.update(&id, t, pos);
                Vec3::default()
            }
            None => tracker.update(&id, t, pos),
        };
        out.push(Obstacle { id, kind, pos, axis, half_w, half_l, vel, occupied });
    }

    // People on foot (anyone seated rides with a vehicle).
    let me = world.my_uuid.clone();
    let people: Vec<(String, Option<String>, Vec3)> = world
        .players
        .iter()
        .filter(|p| p.uuid != me)
        .map(|p| (p.uuid.clone(), p.parent_id.clone(), p.position))
        .collect();
    for (id, parent, local) in people {
        if parent.as_deref().is_some_and(|p| p == truck || world.objects.get(p).is_some_and(|o| o.object_type == "vehicle")) {
            continue;
        }
        let Some(pos) = to_planet(world, parent.as_deref(), local, radius) else { continue };
        if geo::arc_m(pos, around, radius) > range_m {
            continue;
        }
        let vel = tracker.update(&id, t, pos);
        let axis = if geo::length(vel) > 0.1 { geo::normalized(vel) } else { geo::normalized(around) };
        out.push(Obstacle {
            id,
            kind: Kind::Person,
            pos,
            axis,
            half_w: PERSON_SIZE_M / 2.0,
            half_l: PERSON_SIZE_M / 2.0,
            vel,
            occupied: true,
        });
    }
    tracker.prune(t);
    out
}

/// Whether a vehicle has someone aboard (pilot, or any seat taken).
fn occupied(o: &crate::world::state::ObjectState) -> bool {
    let pilot = o.property("pilot_uuid").and_then(Value::as_str).is_some_and(|p| !p.is_empty());
    let seated = o
        .property("seats")
        .and_then(Value::as_object)
        .is_some_and(|m| m.values().any(|v| v.as_str().is_some_and(|s| !s.is_empty())));
    pilot || seated
}

fn parse_vec3(v: &Value) -> Option<Vec3> {
    Some(Vec3 { x: v.get("x")?.as_f64()?, y: v.get("y")?.as_f64()?, z: v.get("z")?.as_f64()? })
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

    fn at(east: f64, north: f64) -> Vec3 {
        let o = geo::scale(geo::lonlat_to_unit(0.0, 0.0), R);
        geo::Tangent::at(o).unproject(east, north)
    }

    fn pos(p: Vec3) -> Value {
        json!({"x": p.x, "y": p.y, "z": p.z})
    }

    #[test]
    fn sees_vehicles_people_and_props_but_not_its_own_truck_or_cargo() {
        let world = WorldView::new("me".into());
        enter(&world, "truck", "vehicle", json!({"position": pos(at(0.0, 0.0))}));
        enter(&world, "parked", "vehicle", json!({"position": pos(at(30.0, 0.0)), "seats": {"seat_driver": "", "seat_passenger": "bob"}}));
        enter(&world, "crate", "crate_container", json!({"position": pos(at(50.0, 1.0))}));
        enter(&world, "cargo", "box", json!({"parent_id": "truck", "position": {"x": 0.0, "y": 0.5, "z": 1.0}}));
        enter(&world, "depot", "cargo_depot", json!({"position": pos(at(20.0, 5.0))}));
        enter(&world, "far", "box", json!({"position": pos(at(900.0, 0.0))}));
        enter(&world, "walker", "player", json!({"position": pos(at(40.0, -2.0))}));
        let mut tr = Tracker::default();
        let seen = snapshot(&world, "truck", at(0.0, 0.0), 200.0, R, &mut tr, 0.0);
        let mut ids: Vec<&str> = seen.iter().map(|o| o.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["crate", "parked", "walker"]);
        let parked = seen.iter().find(|o| o.id == "parked").unwrap();
        assert_eq!(parked.kind, Kind::Vehicle);
        assert!((parked.half_w - 1.1).abs() < 1e-9 && (parked.half_l - 1.85).abs() < 1e-9);
        assert_eq!(seen.iter().find(|o| o.id == "walker").unwrap().kind, Kind::Person);
        assert!(parked.occupied, "a passenger aboard");
        assert!(!seen.iter().find(|o| o.id == "crate").unwrap().occupied);
    }

    #[test]
    fn a_vehicle_velocity_comes_from_velocity_local() {
        let world = WorldView::new("me".into());
        // Yaw 0: the cab faces -Z, so -Z forward at 10 m/s.
        enter(&world, "car", "vehicle", json!({"position": pos(at(30.0, 0.0)), "velocity_local": {"x": 0.0, "y": 0.0, "z": -10.0}}));
        let mut tr = Tracker::default();
        let seen = snapshot(&world, "truck", at(0.0, 0.0), 200.0, R, &mut tr, 0.0);
        let car = &seen[0];
        assert!((geo::length(car.vel) - 10.0).abs() < 0.05, "{:?}", car.vel);
        assert!(geo::dot(geo::normalized(car.vel), car.axis) > 0.99, "moving along its axis");
    }

    #[test]
    fn tracks_the_velocity_of_things_that_do_not_replicate_it() {
        let mut tr = Tracker::default();
        assert_eq!(tr.update("p", 0.0, at(0.0, 0.0)), Vec3::default());
        tr.update("p", 0.5, at(0.75, 0.0));
        let v = tr.update("p", 1.0, at(1.5, 0.0));
        assert!((geo::length(v) - 1.5).abs() < 0.05, "{v:?}");
    }
}
