//! Slot layouts for storage furniture (shelves, racks…), loaded from JSON
//! files exported by the Godot editor — NOT replicated over the network: the
//! server only replicates the per-shelf `shelf_slots_occupied` occupancy;
//! the slot geometry is static per scene and lives here.
//!
//! The files sit under `SLOT_LAYOUTS_DIR` (default `config/slot_layouts`) and
//! mirror the Godot scene tree: the layout for
//! `res://scenes/_universe/props/furniture/metal_shelf.tscn` is read from
//! `config/slot_layouts/scenes/_universe/props/furniture/metal_shelf.json`.
//! The relative path (with `.json` swapped back to `.tscn`) IS the lookup
//! key; a `scenename` field inside the file is only informative and warned
//! about when it disagrees with the path.
//!
//! File template (positions local to the scene root node, metres,
//! index-matched with the replicated `shelf_slots_occupied`, ordered like the
//! Godot slot markers — bottom row first, left-to-right facing the front):
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "scenename": "scenes/_universe/props/furniture/metal_shelf.tscn",
//!   "object_type": "shelf",
//!   "front": [0.0, 0.0, 1.0],
//!   "slots": [[-1.25, 0.35, 0.42], [-0.75, 0.35, 0.42]]
//! }
//! ```

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};

use serde::Deserialize;
use tracing::{info, warn};

use crate::horizon::messages::Vec3;

/// Slot geometry of one storage scene, as exported from Godot.
#[derive(Debug, Clone, Deserialize)]
pub struct SlotLayout {
    #[serde(default)]
    pub schema_version: u32,
    /// Informative copy of the scene path; the file's location is the key.
    #[serde(default)]
    pub scenename: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    /// Direction of the access face in the scene's local frame — the way an
    /// NPC approaches from. Defaults to +Z. Its LENGTH scales the standing
    /// distance: the NPC stands at `slot + front × stand_distance_m`, so a
    /// unit vector keeps the per-NPC distance as-is while e.g. `[2.5, 0, 0]`
    /// makes this scene demand 2.5× the clearance.
    #[serde(default = "default_front")]
    pub front: [f64; 3],
    /// Centre of each slot, local to the scene root node, metres,
    /// index-matched with the replicated occupancy array.
    pub slots: Vec<[f64; 3]>,
}

fn default_front() -> [f64; 3] {
    [0.0, 0.0, 1.0]
}

impl SlotLayout {
    pub fn slot(&self, idx: usize) -> Option<Vec3> {
        self.slots.get(idx).map(|p| Vec3 { x: p[0], y: p[1], z: p[2] })
    }

    pub fn front_vec(&self) -> Vec3 {
        Vec3 { x: self.front[0], y: self.front[1], z: self.front[2] }
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }
}

/// Registry of every slot layout found under the layouts directory, indexed
/// by scene path (e.g. `scenes/_universe/props/furniture/metal_shelf.tscn`).
#[derive(Debug, Default)]
pub struct SlotLayouts {
    by_scene: HashMap<String, SlotLayout>,
}

impl SlotLayouts {
    /// The process-wide registry, loaded once from `SLOT_LAYOUTS_DIR`
    /// (default `config/slot_layouts`, like the other config paths).
    pub fn shared() -> Arc<SlotLayouts> {
        static SHARED: OnceLock<Arc<SlotLayouts>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| {
            let dir = std::env::var("SLOT_LAYOUTS_DIR")
                .unwrap_or_else(|_| "config/slot_layouts".to_string());
            Arc::new(SlotLayouts::load(Path::new(&dir)))
        }))
    }

    /// Load every `*.json` under `dir` (recursively). A missing directory or
    /// an unparseable file is warned about and skipped — a shelf whose layout
    /// is absent simply reads as "not usable" to the actions.
    pub fn load(dir: &Path) -> Self {
        let mut by_scene = HashMap::new();
        if dir.is_dir() {
            Self::walk(dir, dir, &mut by_scene);
        } else {
            warn!(dir = %dir.display(), "slot layouts directory not found; no storage scene will be usable");
        }
        info!(count = by_scene.len(), dir = %dir.display(), "slot layouts loaded");
        Self { by_scene }
    }

    fn walk(dir: &Path, root: &Path, by_scene: &mut HashMap<String, SlotLayout>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(dir = %dir.display(), error = %e, "cannot read slot layouts directory");
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
            let Some(key) = Self::derive_key(&path, root) else {
                continue;
            };
            let layout: SlotLayout = match fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            {
                Ok(l) => l,
                Err(e) => {
                    warn!(file = %path.display(), error = %e, "unparseable slot layout; skipping");
                    continue;
                }
            };
            if let Some(inner) = layout.scenename.as_deref() {
                if normalize(inner) != key {
                    warn!(
                        file = %path.display(),
                        inner = %inner,
                        key = %key,
                        "slot layout scenename disagrees with its path; the path wins"
                    );
                }
            }
            by_scene.insert(key, layout);
        }
    }

    /// Lookup key for a layout file: its path relative to the layouts root,
    /// with `.json` swapped back to `.tscn` and forward slashes.
    fn derive_key(path: &Path, root: &Path) -> Option<String> {
        let rel = path.strip_prefix(root).ok()?;
        Some(rel.with_extension("tscn").to_string_lossy().replace('\\', "/"))
    }

    /// Layout for a replicated `scenename` (a leading `res://` is tolerated).
    pub fn get(&self, scenename: &str) -> Option<&SlotLayout> {
        self.by_scene.get(normalize(scenename))
    }

    pub fn len(&self) -> usize {
        self.by_scene.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_scene.is_empty()
    }

    /// Build a registry from in-memory entries (tests).
    pub fn from_map(by_scene: HashMap<String, SlotLayout>) -> Self {
        Self { by_scene }
    }
}

/// The replicated `scenename` may carry Godot's `res://` prefix; the layout
/// keys never do.
fn normalize(scenename: &str) -> &str {
    scenename.strip_prefix("res://").unwrap_or(scenename)
}

#[cfg(test)]
mod tests {
    use super::*;

    const METAL_SHELF: &str = "scenes/_universe/props/furniture/metal_shelf.tscn";

    /// The layouts shipped in the repo must load — this also validates the
    /// committed JSON files themselves.
    #[test]
    fn loads_the_shipped_layouts_by_mirrored_path() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/slot_layouts");
        let layouts = SlotLayouts::load(&dir);
        let shelf = layouts.get(METAL_SHELF).expect("metal_shelf layout shipped");
        assert_eq!(shelf.slot_count(), 24);
        // Don't pin the exported values themselves — just require a usable
        // access-face vector (non-zero; its length scales the stand
        // distance, so any positive magnitude is legitimate).
        let f = shelf.front_vec();
        let norm = (f.x * f.x + f.y * f.y + f.z * f.z).sqrt();
        assert!(norm > 1e-6, "front must be non-zero, got {f:?}");
        assert!(shelf.slot(0).is_some());
        assert!(shelf.slot(24).is_none());
    }

    #[test]
    fn res_prefix_is_tolerated_on_lookup() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/slot_layouts");
        let layouts = SlotLayouts::load(&dir);
        assert!(layouts.get(&format!("res://{METAL_SHELF}")).is_some());
        assert!(layouts.get("res://scenes/unknown.tscn").is_none());
    }

    #[test]
    fn missing_directory_yields_an_empty_registry() {
        let layouts = SlotLayouts::load(Path::new("/nonexistent/slot_layouts"));
        assert!(layouts.is_empty());
    }

    #[test]
    fn front_defaults_to_plus_z() {
        let layout: SlotLayout =
            serde_json::from_str(r#"{"slots": [[1.0, 2.0, 3.0]]}"#).unwrap();
        assert_eq!(layout.front, [0.0, 0.0, 1.0]);
        let s = layout.slot(0).unwrap();
        assert_eq!((s.x, s.y, s.z), (1.0, 2.0, 3.0));
    }
}
