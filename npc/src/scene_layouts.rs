//! Interaction points of scenes (kiosks, seats, door handles, NPC markers),
//! loaded from JSON files exported by the Godot tool
//! `DyingStar/tools/npc/npc_layout_export.gd` — NOT replicated: they are static
//! per scene, like [`slot_layouts`](crate::slot_layouts).
//!
//! The files sit under `SCENE_LAYOUTS_DIR` (default `config/scene_layouts`)
//! and mirror the Godot scene tree: the layout of
//! `res://scenes/_universe/structures/industrial/garage.tscn` is read from
//! `config/scene_layouts/scenes/_universe/structures/industrial/garage.json`.
//!
//! Every position is in the scene root's frame — the frame of the replicated
//! object built from that scene — so it can be sent as is in
//! `npc_go_to_position {uuid: <object>, position}`.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};

use serde::Deserialize;
use tracing::{info, warn};

use crate::horizon::messages::Vec3;

/// Format version this service understands. A file with another version is
/// skipped (and warned about) rather than half-read.
pub const SCHEMA_VERSION: u32 = 1;

fn v3(a: [f64; 3]) -> Vec3 {
    Vec3 { x: a[0], y: a[1], z: a[2] }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pose {
    pub position: [f64; 3],
    #[serde(default = "default_forward")]
    pub forward: [f64; 3],
}

impl Pose {
    pub fn position(&self) -> Vec3 {
        v3(self.position)
    }
    pub fn forward(&self) -> Vec3 {
        v3(self.forward)
    }
}

fn default_forward() -> [f64; 3] {
    [0.0, 0.0, -1.0]
}

#[derive(Debug, Clone, Deserialize)]
pub struct KioskLayout {
    /// Path of the kiosk node inside the building — sent as `kiosk` in the
    /// `kiosk_spawn` action.
    pub path: String,
    #[serde(default)]
    pub text: String,
    pub position: [f64; 3],
    /// Where the NPC stands to press it.
    pub approach: [f64; 3],
    #[serde(default = "default_use_range")]
    pub use_range_m: f64,
    #[serde(default)]
    pub cooldown_s: f64,
    /// Where the spawned prop appears (its forward = the prop's forward).
    #[serde(default)]
    pub spawn: Option<Pose>,
}

fn default_use_range() -> f64 {
    4.0
}

impl KioskLayout {
    pub fn position(&self) -> Vec3 {
        v3(self.position)
    }
    pub fn approach(&self) -> Vec3 {
        v3(self.approach)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeatLayout {
    /// Seat key as the server expects it in `enter_vehicle` (`seat_driver`).
    pub key: String,
    #[serde(default)]
    pub driver: bool,
    #[serde(default)]
    pub door_id: String,
    pub position: [f64; 3],
}

#[derive(Debug, Clone, Deserialize)]
pub struct DoorLayout {
    pub door_id: String,
    pub outdoor: [f64; 3],
    #[serde(default)]
    pub indoor: Option<[f64; 3]>,
    /// Where the NPC stands to open it and board.
    pub approach: [f64; 3],
}

impl DoorLayout {
    pub fn outdoor(&self) -> Vec3 {
        v3(self.outdoor)
    }
    pub fn approach(&self) -> Vec3 {
        v3(self.approach)
    }
}

/// Driving geometry of a vehicle scene, for the controller.
#[derive(Debug, Clone, Deserialize)]
pub struct VehicleLayout {
    #[serde(default = "default_forward")]
    pub forward: [f64; 3],
    pub wheelbase_m: f64,
    pub max_steer_deg: f64,
    /// Fraction of the max steer still available at/above
    /// `steer_speed_falloff_kmh` (the vehicle narrows the lock with speed).
    #[serde(default = "one")]
    pub steer_min_ratio: f64,
    #[serde(default)]
    pub steer_speed_falloff_kmh: f64,
    /// Wheel turn rate out from centre and back to it, rad/s at full steer
    /// (`Vehicle.steer_speed` / `steer_return_speed`; the truck's are 0.2 and
    /// 1.0). `None` in layouts exported before they were.
    #[serde(default)]
    pub steer_speed: Option<f64>,
    #[serde(default)]
    pub steer_return_speed: Option<f64>,
}

fn one() -> f64 {
    1.0
}

impl VehicleLayout {
    pub fn forward(&self) -> Vec3 {
        v3(self.forward)
    }
}

/// A garage instanced in a village layout: where it sits in the village.
#[derive(Debug, Clone, Deserialize)]
pub struct GarageRef {
    pub name: String,
    #[serde(default)]
    pub path: String,
    pub position: [f64; 3],
    /// Rotation about the village's up axis (Godot convention), radians.
    #[serde(default)]
    pub yaw: f64,
}

impl GarageRef {
    /// A point local to the garage, in the village layout's frame.
    pub fn to_village(&self, local: Vec3) -> Vec3 {
        let r = crate::actions::depot_common::rotate_y(local, self.yaw);
        Vec3 { x: self.position[0] + r.x, y: self.position[1] + r.y, z: self.position[2] + r.z }
    }
}

/// Ground footprint of a solid collision shape: the four corners `[x, z]` of
/// its box seen from above (scene root frame), and its height range.
#[derive(Debug, Clone, Deserialize)]
pub struct ObstacleLayout {
    pub corners: Vec<[f64; 2]>,
    #[serde(default)]
    pub y_min: f64,
    #[serde(default)]
    pub y_max: f64,
}

impl ObstacleLayout {
    /// The corners as points of the scene root frame (on its ground plane).
    pub fn corner_points(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.corners.iter().map(|c| Vec3 { x: c[0], y: 0.0, z: c[1] })
    }
}

/// Everything exported for one scene.
#[derive(Debug, Clone, Deserialize)]
pub struct SceneLayout {
    pub schema_version: u32,
    #[serde(default)]
    pub scenename: Option<String>,
    #[serde(default)]
    pub kiosks: Vec<KioskLayout>,
    /// `Npc*` Marker3D nodes by name (e.g. `NpcParking`).
    #[serde(default)]
    pub markers: HashMap<String, Pose>,
    #[serde(default)]
    pub seats: Vec<SeatLayout>,
    #[serde(default)]
    pub doors: Vec<DoorLayout>,
    #[serde(default)]
    pub vehicle: Option<VehicleLayout>,
    #[serde(default)]
    pub garages: Vec<GarageRef>,
    /// Solid footprints a vehicle must drive around (village layouts).
    #[serde(default)]
    pub obstacles: Vec<ObstacleLayout>,
    /// Bounding box of the scene's own collision shapes (vehicles, props):
    /// what it takes up on a road (ADR services/0012).
    #[serde(default)]
    pub footprint: Option<Footprint>,
}

/// Axis-aligned box of a scene's collision shapes, scene root frame.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Footprint {
    #[serde(default)]
    pub center: [f64; 3],
    pub size: [f64; 3],
}

impl SceneLayout {
    pub fn driver_seat(&self) -> Option<&SeatLayout> {
        self.seats.iter().find(|s| s.driver)
    }

    pub fn door(&self, door_id: &str) -> Option<&DoorLayout> {
        self.doors.iter().find(|d| d.door_id == door_id)
    }

    pub fn marker(&self, name: &str) -> Option<&Pose> {
        self.markers.get(name)
    }
}

/// Registry of every scene layout under the layouts directory, keyed by scene
/// path (`scenes/…/garage.tscn`).
#[derive(Debug, Default)]
pub struct SceneLayouts {
    by_scene: HashMap<String, SceneLayout>,
}

impl SceneLayouts {
    /// The process-wide registry, loaded once from `SCENE_LAYOUTS_DIR`
    /// (default `config/scene_layouts`).
    pub fn shared() -> Arc<SceneLayouts> {
        static SHARED: OnceLock<Arc<SceneLayouts>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| {
            let dir = std::env::var("SCENE_LAYOUTS_DIR")
                .unwrap_or_else(|_| "config/scene_layouts".to_string());
            Arc::new(SceneLayouts::load(Path::new(&dir)))
        }))
    }

    /// Load every `*.json` under `dir` (recursively). A missing directory or a
    /// bad file is warned about and skipped: an action needing that scene then
    /// simply cannot run.
    pub fn load(dir: &Path) -> Self {
        let mut by_scene = HashMap::new();
        if dir.is_dir() {
            Self::walk(dir, dir, &mut by_scene);
        } else {
            warn!(dir = %dir.display(), "scene layouts directory not found");
        }
        info!(count = by_scene.len(), dir = %dir.display(), "scene layouts loaded");
        Self { by_scene }
    }

    fn walk(dir: &Path, root: &Path, by_scene: &mut HashMap<String, SceneLayout>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(dir = %dir.display(), error = %e, "cannot read scene layouts directory");
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, root, by_scene);
                continue;
            }
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(key) = path
                .strip_prefix(root)
                .ok()
                .map(|rel| rel.with_extension("tscn").to_string_lossy().replace('\\', "/"))
            else {
                continue;
            };
            let layout: SceneLayout = match fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            {
                Ok(l) => l,
                Err(e) => {
                    warn!(file = %path.display(), error = %e, "unparseable scene layout; skipping");
                    continue;
                }
            };
            if layout.schema_version != SCHEMA_VERSION {
                warn!(
                    file = %path.display(),
                    version = layout.schema_version,
                    expected = SCHEMA_VERSION,
                    "scene layout schema version not understood; skipping"
                );
                continue;
            }
            by_scene.insert(key, layout);
        }
    }

    /// Layout for a replicated `scenename` (a leading `res://` is tolerated).
    pub fn get(&self, scenename: &str) -> Option<&SceneLayout> {
        self.by_scene.get(normalize(scenename))
    }

    pub fn len(&self) -> usize {
        self.by_scene.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_scene.is_empty()
    }

    /// Build a registry from in-memory entries (tests).
    pub fn from_map(by_scene: HashMap<String, SceneLayout>) -> Self {
        Self { by_scene }
    }
}

pub fn normalize(scenename: &str) -> &str {
    scenename.strip_prefix("res://").unwrap_or(scenename)
}

/// Scene of the garage building (kiosk + truck spawn).
pub const GARAGE_SCENE: &str = "scenes/_universe/structures/industrial/garage.tscn";

#[cfg(test)]
mod tests {
    use super::*;

    const TRUCK: &str = "scenes/_universe/vehicles/ground/trucks/truck.tscn";

    #[test]
    fn loads_the_shipped_layouts() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/scene_layouts");
        let layouts = SceneLayouts::load(&dir);
        let garage = layouts.get(GARAGE_SCENE).expect("garage layout shipped");
        let kiosk = garage.kiosks.first().expect("garage has a kiosk");
        assert_eq!(kiosk.path, "SpawnKiosk");
        assert!(kiosk.spawn.is_some());
        let truck = layouts.get(&format!("res://{TRUCK}")).expect("truck layout shipped");
        let seat = truck.driver_seat().expect("truck has a driver seat");
        assert_eq!(seat.key, "seat_driver");
        assert!(truck.door(&seat.door_id).is_some(), "driver door exported");
        assert!(truck.vehicle.as_ref().is_some_and(|v| v.wheelbase_m > 0.0));
    }

    #[test]
    fn unknown_schema_version_is_skipped() {
        let dir = std::env::temp_dir().join(format!("scene_layouts_test_{}", std::process::id()));
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::write(dir.join("scenes/x.json"), r#"{"schema_version": 99}"#).unwrap();
        let layouts = SceneLayouts::load(&dir);
        assert!(layouts.get("scenes/x.tscn").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
