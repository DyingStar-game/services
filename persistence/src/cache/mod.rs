use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use dashmap::DashMap;

use crate::websocket::messages::Item;

/// Dual-buffer cache for high-throughput writes.
///
/// Two `DashMap` instances act as ping-pong buffers:
/// - One is the **active** map: all incoming `update_item` messages land here.
/// - The other is the **idle** map: being flushed to ScyllaDB in the background.
///
/// The `active` flag indicates which map is currently active:
/// - `false` → `map_a` is active, `map_b` is flushing.
/// - `true`  → `map_b` is active, `map_a` is flushing.
///
/// The swap is atomic and lock-free from the writer perspective:
/// 1. Atomically flip `active`.
/// 2. Drain the now-idle map → return its contents for flushing.
/// 3. Callers continue inserting into the other map immediately.
#[derive(Clone)]
pub struct DualCache {
    map_a: Arc<DashMap<String, Item>>,
    map_b: Arc<DashMap<String, Item>>,
    /// `false` → map_a is active; `true` → map_b is active.
    active: Arc<AtomicBool>,
}

impl DualCache {
    pub fn new() -> Self {
        Self {
            map_a: Arc::new(DashMap::new()),
            map_b: Arc::new(DashMap::new()),
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Insert or overwrite an item in the active map.
    pub fn insert(&self, item: Item) {
        self.active_map().insert(item.uuid.clone(), item);
    }

    /// Remove an item by uuid from both buffers.
    /// Needed when a DELETE arrives so the item is not re-written on the next flush.
    pub fn remove(&self, uuid: &str) {
        self.map_a.remove(uuid);
        self.map_b.remove(uuid);
    }

    /// Atomically swap buffers and return all items that were in the now-idle
    /// (previously active) map so they can be flushed to the database.
    ///
    /// After this call:
    /// - The new active map is empty (ready for fresh writes).
    /// - The returned `Vec<Item>` should be persisted to ScyllaDB.
    pub fn swap_and_drain(&self) -> Vec<Item> {
        // Flip the active flag.  Use AcqRel so that all prior writes to the
        // active map are visible after the swap.
        let was_b = self.active.fetch_xor(true, Ordering::AcqRel);

        // The map that was active is now idle — drain it.
        let idle = if was_b { &self.map_b } else { &self.map_a };
        let items: Vec<Item> = idle.iter().map(|r| r.value().clone()).collect();
        idle.clear();
        items
    }

    fn active_map(&self) -> &DashMap<String, Item> {
        if self.active.load(Ordering::Acquire) {
            &self.map_b
        } else {
            &self.map_a
        }
    }

    /// Look up an item by uuid. Checks the active map first, then the idle map.
    ///
    /// Checking the idle map covers the window between a `swap_and_drain` flip
    /// and the subsequent `idle.clear()`, where an item may not yet be in the
    /// new active map.
    pub fn get(&self, uuid: &str) -> Option<Item> {
        if let Some(item) = self.active_map().get(uuid) {
            return Some(item.clone());
        }
        // Idle map is whichever map is NOT currently active.
        let idle = if self.active.load(Ordering::Acquire) {
            &self.map_a
        } else {
            &self.map_b
        };
        idle.get(uuid).map(|r| r.value().clone())
    }

    /// Number of items currently waiting in the active map.
    pub fn pending_count(&self) -> usize {
        self.active_map().len()
    }
}

impl Default for DualCache {
    fn default() -> Self {
        Self::new()
    }
}
