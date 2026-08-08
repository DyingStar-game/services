use std::{sync::RwLock, time::Instant};

use dashmap::DashMap;

use crate::{
    horizon::messages::{GorcFrame, GorcKind, Vec3},
    world::state::{ObjectState, PlayerState, parse_vec3},
};

/// Live view of everything Horizon is replicating into the NPC's GORC zones.
/// Players and items are kept in separate `DashMap`s keyed by UUID.
pub struct WorldView {
    /// UUID assigned to this NPC by `init_ack`.
    pub my_uuid: String,
    /// Players (other than self) currently inside any of this NPC's zones.
    pub players: DashMap<String, PlayerState>,
    /// Non-player objects (props, ships, planets, items, …).
    pub objects: DashMap<String, ObjectState>,
    /// Latest position Horizon echoed back for this NPC. `None` until the first
    /// self frame carrying a position arrives. Horizon replicates the NPC as a
    /// player-type object, so it never lands in `objects`.
    my_position: RwLock<Option<Vec3>>,
    /// Parent this NPC is attached to, if Horizon reports one. Positions are
    /// only comparable between entities sharing a parent.
    my_parent: RwLock<Option<String>>,
    /// Latest values of the NPC's own player properties, as echoed back on self
    /// frames. Server-driven flags such as `npc_arrived` land here; position,
    /// rotation and parent keep their typed fields above.
    my_props: DashMap<String, serde_json::Value>,
}

impl WorldView {
    pub fn new(my_uuid: String) -> Self {
        Self {
            my_uuid,
            players: DashMap::new(),
            objects: DashMap::new(),
            my_position: RwLock::new(None),
            my_parent: RwLock::new(None),
            my_props: DashMap::new(),
        }
    }

    /// True iff at least one other player is present.
    pub fn has_other_player(&self) -> bool {
        !self.players.is_empty()
    }

    /// This NPC's own position, or `None` if Horizon has not echoed one back
    /// yet. Callers must not substitute the origin: world coordinates are far
    /// from `(0,0,0)`, so a wrong reference point silently breaks distances.
    pub fn my_position(&self) -> Option<Vec3> {
        *self.my_position.read().unwrap()
    }

    /// Parent this NPC is attached to, if Horizon reports one.
    pub fn my_parent(&self) -> Option<String> {
        self.my_parent.read().unwrap().clone()
    }

    /// Resolve a position expressed in `parent_id`'s frame into the root frame
    /// by summing ancestor positions, the way Horizon does internally.
    ///
    /// An absent or empty `parent_id` already means the root frame. Returns
    /// `None` when an ancestor is not in the view, since a partial sum would be
    /// a plausible-looking wrong answer rather than an obvious failure.
    ///
    /// Like Horizon, this is translation-only: it ignores parent rotation, so
    /// entities on a rotating parent drift from the client's true transform.
    pub fn resolve_global(&self, local: Vec3, parent_id: Option<&str>) -> Option<Vec3> {
        /// Depth cap: guards against a parent cycle in malformed data.
        const MAX_DEPTH: usize = 16;

        let mut acc = local;
        let mut cursor = parent_id.map(str::to_string);
        for _ in 0..MAX_DEPTH {
            let Some(pid) = cursor.filter(|p| !p.is_empty()) else {
                return Some(acc);
            };
            let parent = self.objects.get(&pid)?;
            acc.x += parent.position.x;
            acc.y += parent.position.y;
            acc.z += parent.position.z;
            cursor = parent.parent_id.clone();
        }
        None
    }

    /// This NPC's position in the root frame, or `None` if it or any ancestor
    /// is not yet known.
    pub fn my_global_position(&self) -> Option<Vec3> {
        let local = self.my_position()?;
        self.resolve_global(local, self.my_parent().as_deref())
    }

    /// This NPC's position expressed in `frame`'s coordinate system (e.g. the
    /// depot the NPC works at, whose zone points are all depot-local).
    ///
    /// Prefers walking the NPC's own parent chain up to `frame`, which stays
    /// exact even when `frame`'s ancestors (its planet, …) never replicated
    /// into view — the situation that makes [`my_global_position`] fail. Only
    /// when `frame` is not an ancestor does it fall back to the difference of
    /// root-frame positions. Translation-only, like [`resolve_global`].
    ///
    /// [`my_global_position`]: Self::my_global_position
    pub fn my_position_in(&self, frame: &str) -> Option<Vec3> {
        let local = self.my_position()?;
        let parent = self.my_parent();
        if let Some(v) = self.resolve_up_to(local, parent.as_deref(), frame) {
            return Some(v);
        }
        let npc = self.resolve_global(local, parent.as_deref())?;
        let origin = {
            let (pos, pid) = {
                let f = self.objects.get(frame)?;
                (f.position, f.parent_id.clone())
            };
            self.resolve_global(pos, pid.as_deref())?
        };
        Some(Vec3 {
            x: npc.x - origin.x,
            y: npc.y - origin.y,
            z: npc.z - origin.z,
        })
    }

    /// `local` (expressed in `parent_id`'s frame) re-expressed in `frame`'s
    /// coordinates, when `frame` is `parent_id` or one of its ancestors.
    /// `None` when the chain does not pass through `frame` (or breaks first).
    fn resolve_up_to(&self, local: Vec3, parent_id: Option<&str>, frame: &str) -> Option<Vec3> {
        const MAX_DEPTH: usize = 16;

        let mut acc = local;
        let mut cursor = parent_id.map(str::to_string);
        for _ in 0..MAX_DEPTH {
            let pid = cursor.filter(|p| !p.is_empty())?;
            if pid == frame {
                return Some(acc);
            }
            let parent = self.objects.get(&pid)?;
            acc.x += parent.position.x;
            acc.y += parent.position.y;
            acc.z += parent.position.z;
            cursor = parent.parent_id.clone();
        }
        None
    }

    /// Record position, parent and any other player properties from a frame
    /// about self, if it carries them.
    fn remember_self(&self, frame: &GorcFrame) {
        if let Some(p) = parse_vec3(&frame.data, "position") {
            *self.my_position.write().unwrap() = Some(p);
        }
        if let Some(parent) = frame.data.get("parent_id").and_then(|v| v.as_str()) {
            *self.my_parent.write().unwrap() = if parent.is_empty() {
                None
            } else {
                Some(parent.to_string())
            };
        }
        // Keep every other replicated property (e.g. server-driven flags like
        // `npc_arrived`) so actions can react to them. Only frames that carry a
        // key update it, mirroring how name/parent are handled for players.
        if let Some(obj) = frame.data.as_object() {
            for (k, v) in obj {
                self.my_props.insert(k.clone(), v.clone());
            }
        }
    }

    /// Read one of the NPC's own replicated player properties, or `None` if it
    /// has never arrived on a self frame.
    pub fn my_property(&self, key: &str) -> Option<serde_json::Value> {
        self.my_props.get(key).map(|v| v.clone())
    }

    /// Whether the server has flagged this NPC as having arrived at its target
    /// (the `npc_arrived` player property). Cleared with [`clear_arrived`].
    ///
    /// [`clear_arrived`]: Self::clear_arrived
    pub fn npc_arrived(&self) -> bool {
        self.my_property("npc_arrived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Forget a locally cached arrival flag once it has been acknowledged, so a
    /// stale `true` cannot re-trigger the acknowledgement.
    pub fn clear_arrived(&self) {
        self.my_props.remove("npc_arrived");
    }

    /// Apply a GORC frame to the view. Returns a descriptor of what changed
    /// (for the action scheduler) or `None` if the frame was about self.
    pub fn apply(&self, frame: &GorcFrame) -> Option<WorldEvent> {
        // Frames about self are observed but not stored as another player.
        let about_self = frame.object_id == self.my_uuid;

        match frame.kind {
            GorcKind::ZoneEnter => {
                if frame.object_type == "player" || frame.object_type == "GorcPlayer" {
                    if about_self {
                        self.remember_self(frame);
                        return None;
                    }
                    let entry = self
                        .players
                        .entry(frame.object_id.clone())
                        .or_insert_with(|| PlayerState::new(frame.object_id.clone()));
                    let mut ps = entry;
                    apply_player_data(&mut ps, frame);
                    Some(WorldEvent::PlayerEntered(frame.object_id.clone()))
                } else {
                    let mut o = self
                        .objects
                        .entry(frame.object_id.clone())
                        .or_insert_with(|| {
                            ObjectState::new(frame.object_id.clone(), frame.object_type.clone())
                        });
                    apply_object_data(&mut o, frame);
                    Some(WorldEvent::ObjectEntered(frame.object_id.clone()))
                }
            }
            GorcKind::ZoneExit => {
                // GDScript: deletion only on channel 0 for players, 6 for props.
                if frame.object_type == "player" || frame.object_type == "GorcPlayer" {
                    if frame.channel == 0 {
                        self.players.remove(&frame.object_id);
                        return Some(WorldEvent::PlayerLeft(frame.object_id.clone()));
                    }
                } else if frame.channel == 6 {
                    self.objects.remove(&frame.object_id);
                    return Some(WorldEvent::ObjectLeft(frame.object_id.clone()));
                }
                None
            }
            GorcKind::Update => {
                if frame.object_type == "player" {
                    if about_self {
                        self.remember_self(frame);
                        return None;
                    }
                    if let Some(mut ps) = self.players.get_mut(&frame.object_id) {
                        apply_player_data(&mut ps, frame);
                        return Some(WorldEvent::PlayerMoved(frame.object_id.clone()));
                    }
                }
                None
            }
            GorcKind::UpdateProperty => {
                // Player property updates (server-driven flags like
                // `npc_arrived`, dialog state, …) travel as update_property
                // frames too — route them like player updates, or they are
                // silently lost.
                if frame.object_type == "player" || frame.object_type == "GorcPlayer" {
                    if about_self {
                        self.remember_self(frame);
                        return None;
                    }
                    if let Some(mut ps) = self.players.get_mut(&frame.object_id) {
                        apply_player_data(&mut ps, frame);
                        return Some(WorldEvent::PlayerMoved(frame.object_id.clone()));
                    }
                    return None;
                }
                if let Some(mut o) = self.objects.get_mut(&frame.object_id) {
                    apply_object_data(&mut o, frame);
                    return Some(WorldEvent::ObjectUpdated(frame.object_id.clone()));
                }
                None
            }
        }
    }

    /// Find the nearest other player to `from`. Returns `(uuid, distance²)`.
    pub fn nearest_player(&self, from: Vec3) -> Option<(String, f64)> {
        let mut best: Option<(String, f64)> = None;
        for entry in self.players.iter() {
            let p = entry.value();
            let dx = p.position.x - from.x;
            let dy = p.position.y - from.y;
            let dz = p.position.z - from.z;
            let d2 = dx * dx + dy * dy + dz * dz;
            if best.as_ref().is_none_or(|(_, bd)| d2 < *bd) {
                best = Some((p.uuid.clone(), d2));
            }
        }
        best
    }
}

fn apply_player_data(ps: &mut PlayerState, frame: &GorcFrame) {
    // Like `parent_id`, `name` only travels on frames that carry it, so an
    // absent key must leave the known value alone rather than clear it.
    if let Some(name) = frame.data.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            ps.name = Some(name.to_string());
        }
    }
    if let Some(parent) = frame.data.get("parent_id").and_then(|v| v.as_str()) {
        ps.parent_id = if parent.is_empty() {
            None
        } else {
            Some(parent.to_string())
        };
    }
    if let Some(p) = parse_vec3(&frame.data, "position") {
        ps.position = p;
    }
    if let Some(r) = parse_vec3(&frame.data, "rotation") {
        ps.rotation = r;
    }
    ps.last_seen = Instant::now();
}

fn apply_object_data(o: &mut ObjectState, frame: &GorcFrame) {
    if let Some(parent) = frame.data.get("parent_id").and_then(|v| v.as_str()) {
        o.parent_id = if parent.is_empty() {
            None
        } else {
            Some(parent.to_string())
        };
    }
    if let Some(p) = parse_vec3(&frame.data, "position") {
        o.position = p;
    }
    if let Some(r) = parse_vec3(&frame.data, "rotation") {
        o.rotation = r;
    }
    if let Some(s) = frame.data.get("scenename").and_then(|v| v.as_str()) {
        o.scenename = Some(s.to_string());
    }
    // Merge, don't replace: a frame overwrites only the keys it carries, and
    // the newest frame wins for each key regardless of its channel.
    if let Some(obj) = frame.data.as_object() {
        for (k, v) in obj {
            o.props.insert(k.clone(), v.clone());
        }
    }
    o.last_seen = Instant::now();
}

#[derive(Debug, Clone)]
pub enum WorldEvent {
    PlayerEntered(String),
    PlayerLeft(String),
    PlayerMoved(String),
    ObjectEntered(String),
    ObjectLeft(String),
    ObjectUpdated(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enter_player(uuid: &str, pos: (f64, f64, f64)) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: uuid.into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "parent_id": "",
                "position": {"x": pos.0, "y": pos.1, "z": pos.2},
                "rotation": {"x": 0, "y": 0, "z": 0},
            }),
        }
    }

    /// A non-player entity (city, building, …) entering a zone, optionally
    /// parented to another entity.
    fn enter_object(uuid: &str, pos: (f64, f64, f64), parent: Option<&str>) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: uuid.into(),
            object_type: "building".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "parent_id": parent.unwrap_or(""),
                "position": {"x": pos.0, "y": pos.1, "z": pos.2},
            }),
        }
    }

    #[test]
    fn enters_and_leaves_player() {
        let v = WorldView::new("self".into());
        assert!(!v.has_other_player());
        v.apply(&enter_player("p1", (1.0, 2.0, 3.0)));
        assert!(v.has_other_player());
        let exit = GorcFrame {
            kind: GorcKind::ZoneExit,
            channel: 0,
            object_id: "p1".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({}),
        };
        v.apply(&exit);
        assert!(!v.has_other_player());
    }

    #[test]
    fn ignores_self_zone_enter() {
        let v = WorldView::new("me".into());
        v.apply(&enter_player("me", (0.0, 0.0, 0.0)));
        assert!(!v.has_other_player());
    }

    #[test]
    fn my_position_in_a_frame_survives_missing_frame_ancestors() {
        let v = WorldView::new("me".into());
        // The depot is parented to a planet that never replicated into view,
        // so a root-frame resolution of anything inside it must fail…
        v.apply(&enter_object("depot", (100.0, 0.0, 50.0), Some("planet-never-seen")));
        let mut me = enter_player("me", (1.0, 0.0, 2.0));
        me.data["parent_id"] = json!("depot");
        v.apply(&me);
        assert_eq!(v.my_global_position(), None);
        // …but the NPC's position IN the depot frame is exact regardless.
        let p = v.my_position_in("depot").unwrap();
        assert_eq!((p.x, p.y, p.z), (1.0, 0.0, 2.0));
    }

    #[test]
    fn my_position_in_a_frame_falls_back_to_root_difference() {
        let v = WorldView::new("me".into());
        // NPC and depot are siblings under the same (known) planet: the
        // depot is not an ancestor, so the root-difference fallback applies.
        v.apply(&enter_object("planet", (1000.0, 0.0, 0.0), None));
        v.apply(&enter_object("depot", (100.0, 0.0, 50.0), Some("planet")));
        let mut me = enter_player("me", (103.0, 0.0, 51.0));
        me.data["parent_id"] = json!("planet");
        v.apply(&me);
        let p = v.my_position_in("depot").unwrap();
        assert_eq!((p.x, p.y, p.z), (3.0, 0.0, 1.0));
    }

    #[test]
    fn self_position_unknown_until_a_self_frame_arrives() {
        let v = WorldView::new("me".into());
        assert_eq!(v.my_position(), None);
        // A frame about someone else must not be mistaken for our own position.
        v.apply(&enter_player("other", (7.0, 0.0, 0.0)));
        assert_eq!(v.my_position(), None);
    }

    #[test]
    fn records_player_name_and_keeps_it_across_nameless_frames() {
        let v = WorldView::new("me".into());
        let mut enter = enter_player("other", (1.0, 0.0, 1.0));
        enter.data["name"] = json!("Bob");
        v.apply(&enter);
        assert_eq!(v.players.get("other").unwrap().name.as_deref(), Some("Bob"));

        // A "move" frame carries no name; it must not wipe the known one.
        v.apply(&GorcFrame {
            kind: GorcKind::from_str("move").unwrap(),
            channel: 0,
            object_id: "other".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({"position": {"x": 2.0, "y": 0.0, "z": 2.0}}),
        });
        assert_eq!(v.players.get("other").unwrap().name.as_deref(), Some("Bob"));
    }

    #[test]
    fn move_frame_updates_a_player_position() {
        let v = WorldView::new("me".into());
        v.apply(&enter_player("other", (1.0, 0.0, 1.0)));
        // Horizon sends "move", not "update", for player transforms.
        v.apply(&GorcFrame {
            kind: GorcKind::from_str("move").expect("\"move\" must map to a GORC kind"),
            channel: 0,
            object_id: "other".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({"position": {"x": 9.0, "y": 0.0, "z": 9.0}}),
        });
        let p = v.players.get("other").unwrap();
        assert_eq!((p.position.x, p.position.z), (9.0, 9.0));
    }

    #[test]
    fn resolves_position_through_the_parent_chain() {
        let v = WorldView::new("me".into());
        // city at (1000,0,1000) -> building at (10,0,0) under it.
        v.apply(&enter_object("city", (1000.0, 0.0, 1000.0), None));
        v.apply(&enter_object("building", (10.0, 0.0, 0.0), Some("city")));
        // A player 2m inside the building resolves to 1012,0,1000.
        let g = v
            .resolve_global(Vec3 { x: 2.0, y: 0.0, z: 0.0 }, Some("building"))
            .expect("chain is fully known");
        assert_eq!((g.x, g.y, g.z), (1012.0, 0.0, 1000.0));
    }

    #[test]
    fn root_frame_position_resolves_to_itself() {
        let v = WorldView::new("me".into());
        let local = Vec3 { x: 3.0, y: 0.0, z: 4.0 };
        // Both "no parent" and the empty string mean the root frame.
        assert_eq!(v.resolve_global(local, None), Some(local));
        assert_eq!(v.resolve_global(local, Some("")), Some(local));
    }

    #[test]
    fn unknown_ancestor_is_unresolvable_rather_than_partially_summed() {
        let v = WorldView::new("me".into());
        v.apply(&enter_object("building", (10.0, 0.0, 0.0), Some("city")));
        // "city" was never seen, so the sum would be wrong, not merely partial.
        assert_eq!(
            v.resolve_global(Vec3 { x: 2.0, y: 0.0, z: 0.0 }, Some("building")),
            None
        );
    }

    #[test]
    fn parent_cycle_does_not_hang() {
        let v = WorldView::new("me".into());
        v.apply(&enter_object("a", (1.0, 0.0, 0.0), Some("b")));
        v.apply(&enter_object("b", (1.0, 0.0, 0.0), Some("a")));
        assert_eq!(v.resolve_global(Vec3::default(), Some("a")), None);
    }

    #[test]
    fn records_self_position_from_self_frame() {
        let v = WorldView::new("me".into());
        v.apply(&enter_player("me", (1000.0, 2.0, 1000.0)));
        let p = v.my_position().expect("self position recorded");
        assert_eq!((p.x, p.y, p.z), (1000.0, 2.0, 1000.0));
        // Still not a player in the view.
        assert!(!v.has_other_player());
    }

    #[test]
    fn finds_nearest_player() {
        let v = WorldView::new("self".into());
        v.apply(&enter_player("a", (10.0, 0.0, 0.0)));
        v.apply(&enter_player("b", (2.0, 0.0, 0.0)));
        let (id, d2) = v.nearest_player(Vec3::default()).unwrap();
        assert_eq!(id, "b");
        assert!((d2 - 4.0).abs() < 1e-9);
    }

    #[test]
    fn self_property_update_frame_sets_server_flags() {
        let v = WorldView::new("me".into());
        // The live `npc_arrived` notice arrives as an update_property frame
        // about self, not as a zone_enter or a move.
        v.apply(&GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 0,
            object_id: "me".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "npc_arrived": true }),
        });
        assert!(v.npc_arrived(), "self update_property must reach my_props");
        assert!(!v.has_other_player(), "self must not appear as another player");
    }

    #[test]
    fn other_player_property_update_applies_to_that_player() {
        let v = WorldView::new("me".into());
        v.apply(&enter_player("other", (1.0, 0.0, 1.0)));
        v.apply(&GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 0,
            object_id: "other".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "name": "Bob" }),
        });
        assert_eq!(v.players.get("other").unwrap().name.as_deref(), Some("Bob"));
    }

    #[test]
    fn merges_object_channels() {
        let v = WorldView::new("self".into());
        let ch6 = GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 6,
            object_id: "o".into(),
            object_type: "ship".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({"scenename":"res://s.tscn"}),
        };
        v.apply(&ch6);
        let ch0 = GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: "o".into(),
            object_type: "ship".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({"position":{"x":5.0,"y":0.0,"z":0.0}}),
        };
        v.apply(&ch0);
        let o = v.objects.get("o").unwrap();
        assert_eq!(o.scenename.as_deref(), Some("res://s.tscn"));
        assert_eq!(o.position.x, 5.0);
        assert!(o.property("scenename").is_some(), "ch6 keys kept after a ch0 frame");
    }

    #[test]
    fn newest_frame_wins_for_a_property_regardless_of_channel() {
        // The zone-enter snapshot (channel 6) carries a stale value; a later
        // update on ANOTHER channel must overwrite it — and stay visible. The
        // old per-channel storage made this lookup a coin flip: the stale
        // snapshot could shadow the fresh value forever.
        let v = WorldView::new("self".into());
        v.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 6,
            object_id: "depot".into(),
            object_type: "cargo_depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({"pnj_box_temp_zones": {"zone_0": ["stale-box"]}}),
        });
        v.apply(&GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 2,
            object_id: "depot".into(),
            object_type: "cargo_depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({"pnj_box_temp_zones": {}}),
        });
        let o = v.objects.get("depot").unwrap();
        assert_eq!(o.property("pnj_box_temp_zones"), Some(&json!({})));
    }
}
