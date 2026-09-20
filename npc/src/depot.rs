//! Cargo-depot coordination between the magasinier NPCs.
//!
//! Two magasiniers must never both work the reception counter. The authoritative
//! answer is the depot's replicated `reception_owner` property (granted by the
//! Godot game server to the first NPC that claims it — see [`role_from_owner`]).
//! When that property is not replicated at all (an older game server), NPCs fall
//! back to a deterministic, no-communication election over the shared shift
//! schedule ([`DepotWorkforce::role_for`]): the co-worker who has been on shift
//! the longest (earliest arrival) takes reception, everyone else stocks.

use std::{collections::HashMap, sync::RwLock};

use serde_json::Value;

use crate::{clock::PlanetTime, npc::spec::NpcSpec, schedule::Schedule};

/// What a magasinier does at the depot this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepotRole {
    /// No coordination applies (not a depot worker, off shift, or no roster
    /// entry): the depot actions behave exactly as they did before the roles
    /// existed.
    Uncoordinated,
    /// Owns the reception counter: runs `depot_store_reception_to_temp_area`,
    /// and only helps stock when reception is idle.
    Reception,
    /// Stocks the shelves: runs `depot_store_item`, never reception.
    Stocker,
}

/// Role implied by the depot's replicated `reception_owner` property.
///
/// * `Some(Reception)` — the property names *this* NPC.
/// * `Some(Stocker)` — the property is present but names someone else, or is
///   empty (nobody has claimed yet, so this NPC must not receive until it wins
///   the claim; the coordinator action drives the claim meanwhile).
/// * `None` — the property is absent entirely (older server): the caller should
///   fall back to [`DepotWorkforce::role_for`].
pub fn role_from_owner(owner_prop: Option<&Value>, my_uuid: &str) -> Option<DepotRole> {
    let owner = owner_prop?.as_str()?;
    if owner == my_uuid {
        Some(DepotRole::Reception)
    } else {
        Some(DepotRole::Stocker)
    }
}

/// One magasinier as far as depot coordination cares.
#[derive(Debug, Clone)]
struct Worker {
    uuid: String,
    /// Schedule pattern name, or `None` when the NPC has no schedule (then it is
    /// never counted as a working co-worker for the fallback election).
    pattern: Option<String>,
}

/// Roster index: which NPCs work at which depot. Built by the manager from the
/// NPC specs and shared read-only with every runtime. Only consulted for the
/// deterministic fallback election.
#[derive(Default)]
pub struct DepotWorkforce {
    by_depot: RwLock<HashMap<String, Vec<Worker>>>,
}

impl DepotWorkforce {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or refresh) an NPC's depot membership. NPCs without a workplace
    /// are ignored. A re-inserted uuid replaces its previous entry so a respawn
    /// never double-lists.
    pub fn insert_spec(&self, spec: &NpcSpec) {
        let Some(place) = spec.place_where_work.as_ref() else {
            return;
        };
        let mut map = self.by_depot.write().unwrap();
        let workers = map.entry(place.uuid.clone()).or_default();
        workers.retain(|w| w.uuid != spec.uuid);
        workers.push(Worker {
            uuid: spec.uuid.clone(),
            pattern: spec.schedule.clone(),
        });
    }

    /// Drop an NPC from every depot bucket (dropping empty buckets).
    pub fn remove_uuid(&self, uuid: &str) {
        let mut map = self.by_depot.write().unwrap();
        map.retain(|_, workers| {
            workers.retain(|w| w.uuid != uuid);
            !workers.is_empty()
        });
    }

    /// Whether `owner_uuid` is a magasinier of `depot_uuid` that is NOT on shift
    /// right now — i.e. its `reception_owner` claim is stale and must not be
    /// allowed to keep the counter shut.
    ///
    /// This happens whenever an NPC holds the counter while walking off to a
    /// meal, a mandatory activity or home (the game server can also grant the
    /// role on its own, so the owner may never have claimed — and therefore
    /// never releases). Every co-worker then reads itself as
    /// [`Stocker`](DepotRole::Stocker) and a box deposited at the reception sits
    /// there forever.
    ///
    /// `false` for anyone the roster does not know (a player, an NPC from
    /// another process) or for a worker with no schedule: their claim is
    /// honoured as-is rather than stolen on a guess.
    pub fn is_off_shift_worker(
        &self,
        depot_uuid: &str,
        owner_uuid: &str,
        schedule: &Schedule,
        now: PlanetTime,
    ) -> bool {
        let map = self.by_depot.read().unwrap();
        let Some(workers) = map.get(depot_uuid) else {
            return false;
        };
        let Some(worker) = workers.iter().find(|w| w.uuid == owner_uuid) else {
            return false;
        };
        match worker.pattern.as_deref() {
            Some(pattern) => !schedule.is_working(pattern, now),
            None => false,
        }
    }

    /// Deterministic fallback role for `my_uuid` at `depot_uuid` and planetary
    /// time `now`, when the depot does not replicate `reception_owner`.
    ///
    /// Among co-workers currently on shift (Work/Transmission), the one on shift
    /// the longest wins reception (earliest arrival); ties break on the lowest
    /// uuid so every NPC computes the same winner with no communication. An NPC
    /// not on shift is `Uncoordinated` (its depot actions are schedule-gated off
    /// anyway).
    pub fn role_for(
        &self,
        depot_uuid: &str,
        my_uuid: &str,
        schedule: &Schedule,
        now: PlanetTime,
    ) -> DepotRole {
        let map = self.by_depot.read().unwrap();
        let Some(workers) = map.get(depot_uuid) else {
            return DepotRole::Uncoordinated;
        };
        // (run length, uuid) of the best reception candidate so far.
        let mut best: Option<(u32, &str)> = None;
        let mut me_working = false;
        for w in workers {
            let Some(pattern) = w.pattern.as_deref() else {
                continue;
            };
            if !schedule.is_working(pattern, now) {
                continue;
            }
            if w.uuid == my_uuid {
                me_working = true;
            }
            let run = schedule.work_run_length(pattern, now);
            let better = match best {
                None => true,
                Some((brun, buuid)) => run > brun || (run == brun && w.uuid.as_str() < buuid),
            };
            if better {
                best = Some((run, w.uuid.as_str()));
            }
        }
        if !me_working {
            return DepotRole::Uncoordinated;
        }
        if best.map(|(_, u)| u) == Some(my_uuid) {
            DepotRole::Reception
        } else {
            DepotRole::Stocker
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        horizon::messages::Vec3,
        npc::spec::{NpcSpec, PlaceWhereWork},
        schedule::Schedule,
    };

    const DEPOT: &str = "depot-1";

    fn pt(day: u32, hour: u32) -> PlanetTime {
        PlanetTime {
            day,
            hour,
            hour_f: hour as f64,
        }
    }

    fn spec(uuid: &str, pattern: Option<&str>) -> NpcSpec {
        NpcSpec {
            name: Some(uuid.to_string()),
            uuid: uuid.to_string(),
            action_profile: vec![],
            place_where_work: Some(PlaceWhereWork {
                uuid: DEPOT.to_string(),
                planet: None,
                position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            }),
            schedule: pattern.map(str::to_string),
            home: None,
            meal_place: None,
            mandatory_place: None,
            action_timing: Default::default(),
            depot_store_reception_to_temp_area: None,
            depot_store_item: None,
            take_a_walk: None,
        }
    }

    // Two patterns: A works hours 0-9 of day 0; B starts later, hours 5-14.
    // Both parse to a single 25-char day (rest off).
    fn schedule() -> Schedule {
        let yaml = r#"
schedules:
  a:
    days:
      - "TTTTTTTTTT..............."
  b:
    days:
      - ".....TTTTTTTTTT.........."
"#;
        Schedule::from_yaml(yaml).unwrap()
    }

    #[test]
    fn owner_property_decides_role() {
        assert_eq!(
            role_from_owner(Some(&json!("me")), "me"),
            Some(DepotRole::Reception)
        );
        assert_eq!(
            role_from_owner(Some(&json!("other")), "me"),
            Some(DepotRole::Stocker)
        );
        // Empty = claimed by nobody yet -> stock until we win the claim.
        assert_eq!(
            role_from_owner(Some(&json!("")), "me"),
            Some(DepotRole::Stocker)
        );
        // Absent entirely -> fall back to the deterministic election.
        assert_eq!(role_from_owner(None, "me"), None);
    }

    #[test]
    fn lone_worker_is_reception() {
        let wf = DepotWorkforce::new();
        wf.insert_spec(&spec("a", Some("a")));
        let s = schedule();
        // Hour 2: only A is on shift.
        assert_eq!(wf.role_for(DEPOT, "a", &s, pt(0, 2)), DepotRole::Reception);
    }

    #[test]
    fn earliest_arrival_takes_reception() {
        let wf = DepotWorkforce::new();
        wf.insert_spec(&spec("a", Some("a")));
        wf.insert_spec(&spec("b", Some("b")));
        let s = schedule();
        // Hour 7: both on shift, but A started at hour 0 (run 8) vs B at hour 5
        // (run 3): A arrived first -> Reception, B -> Stocker.
        assert_eq!(wf.role_for(DEPOT, "a", &s, pt(0, 7)), DepotRole::Reception);
        assert_eq!(wf.role_for(DEPOT, "b", &s, pt(0, 7)), DepotRole::Stocker);
    }

    #[test]
    fn off_shift_worker_is_uncoordinated_and_excluded() {
        let wf = DepotWorkforce::new();
        wf.insert_spec(&spec("a", Some("a")));
        wf.insert_spec(&spec("b", Some("b")));
        let s = schedule();
        // Hour 12: A is off (0-9 only), B is on. B is alone -> Reception.
        assert_eq!(wf.role_for(DEPOT, "a", &s, pt(0, 12)), DepotRole::Uncoordinated);
        assert_eq!(wf.role_for(DEPOT, "b", &s, pt(0, 12)), DepotRole::Reception);
    }

    #[test]
    fn only_an_off_shift_co_worker_counts_as_a_stale_owner() {
        let wf = DepotWorkforce::new();
        wf.insert_spec(&spec("a", Some("a")));
        wf.insert_spec(&spec("b", Some("b")));
        let s = schedule();
        // Hour 12: A is off shift (0-9 only) -> its claim is stale; B is on.
        assert!(wf.is_off_shift_worker(DEPOT, "a", &s, pt(0, 12)));
        assert!(!wf.is_off_shift_worker(DEPOT, "b", &s, pt(0, 12)));
        // Someone the roster does not know keeps their claim.
        assert!(!wf.is_off_shift_worker(DEPOT, "a-player", &s, pt(0, 12)));
        assert!(!wf.is_off_shift_worker("nope", "a", &s, pt(0, 12)));
    }

    #[test]
    fn unknown_depot_or_removed_is_uncoordinated() {
        let wf = DepotWorkforce::new();
        wf.insert_spec(&spec("a", Some("a")));
        let s = schedule();
        assert_eq!(wf.role_for("nope", "a", &s, pt(0, 2)), DepotRole::Uncoordinated);
        wf.remove_uuid("a");
        assert_eq!(wf.role_for(DEPOT, "a", &s, pt(0, 2)), DepotRole::Uncoordinated);
    }
}
