//! Shared, mutable per-NPC state: a typed lifecycle status plus a generic
//! key/value "blackboard" for ad-hoc data (box counts, transient flags, …).
//!
//! One `NpcState` is built per Horizon session alongside the [`WorldView`], so
//! a reconnect (which respawns the NPC) starts from a clean slate. The types
//! are serializable so this state can be persisted across sessions later
//! without touching the actions that read and write it.
//!
//! [`WorldView`]: crate::world::view::WorldView

use std::sync::RwLock;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where the NPC is in its work lifecycle. Actions gate their triggers on this
/// (`can_run`) and advance it as they complete (`run`), forming a lightweight
/// finite state machine. Extend as new behaviours are implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcStatus {
    /// Freshly spawned; not yet doing anything in particular.
    Idle,
    /// In its habitation (home); the spawn location for an NPC that works.
    AtHome,
    /// On the way to its place of work.
    GoingToWork,
    /// At the workplace, available for work actions.
    AtWork,
    /// Taking a break from work.
    OnBreak,
    /// Driving a vehicle.
    Driving,
    /// Heading home.
    GoingHome,
    /// Walking to the common zone for a meal.
    GoingToMeal,
    /// Eating in the common zone (schedule activity `r`).
    AtMeal,
    /// Walking to the mandatory-activities area.
    GoingToMandatory,
    /// Doing mandatory activities — medical / RH / cleaning / chores (`a`).
    DoingMandatory,
    /// Strolling toward a random point picked nearby (`take_a_walk`).
    Walking,
    /// Standing at the point a stroll reached, before setting off again.
    WalkPause,
}

impl Default for NpcStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// Shared per-NPC state. Interior mutability (like [`WorldView`]) so actions
/// hold it behind an `Arc` and mutate without a `&mut`.
///
/// [`WorldView`]: crate::world::view::WorldView
#[derive(Default)]
pub struct NpcState {
    status: RwLock<NpcStatus>,
    /// Ad-hoc named values any action may read or write. Use for data that does
    /// not (yet) warrant a typed field: counters, per-job flags, scratch state.
    board: DashMap<String, Value>,
}

impl NpcState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current lifecycle status.
    pub fn status(&self) -> NpcStatus {
        *self.status.read().unwrap()
    }

    /// Advance the lifecycle status.
    pub fn set_status(&self, status: NpcStatus) {
        *self.status.write().unwrap() = status;
    }

    /// Read a blackboard value, or `None` if unset.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.board.get(key).map(|v| v.clone())
    }

    /// Set a blackboard value.
    pub fn set(&self, key: impl Into<String>, value: Value) {
        self.board.insert(key.into(), value);
    }

    /// Remove a blackboard value, returning the previous one if any.
    pub fn remove(&self, key: &str) -> Option<Value> {
        self.board.remove(key).map(|(_, v)| v)
    }

    /// Serializable snapshot of the whole state. The building block for future
    /// persistence; nothing consumes it yet.
    pub fn snapshot(&self) -> NpcStateSnapshot {
        NpcStateSnapshot {
            status: self.status(),
            board: self
                .board
                .iter()
                .map(|e| (e.key().clone(), e.value().clone()))
                .collect(),
        }
    }
}

/// Point-in-time, serializable copy of an [`NpcState`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcStateSnapshot {
    pub status: NpcStatus,
    pub board: std::collections::HashMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_to_idle() {
        assert_eq!(NpcState::new().status(), NpcStatus::Idle);
    }

    #[test]
    fn transitions_status() {
        let s = NpcState::new();
        s.set_status(NpcStatus::GoingToWork);
        assert_eq!(s.status(), NpcStatus::GoingToWork);
        s.set_status(NpcStatus::AtWork);
        assert_eq!(s.status(), NpcStatus::AtWork);
    }

    #[test]
    fn blackboard_get_set_remove() {
        let s = NpcState::new();
        assert!(s.get("boxes").is_none());
        s.set("boxes", json!(3));
        assert_eq!(s.get("boxes"), Some(json!(3)));
        assert_eq!(s.remove("boxes"), Some(json!(3)));
        assert!(s.get("boxes").is_none());
    }

    #[test]
    fn snapshot_captures_status_and_board() {
        let s = NpcState::new();
        s.set_status(NpcStatus::AtWork);
        s.set("zone_0", json!(1));
        let snap = s.snapshot();
        assert_eq!(snap.status, NpcStatus::AtWork);
        assert_eq!(snap.board.get("zone_0"), Some(&json!(1)));
    }
}
