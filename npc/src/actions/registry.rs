use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use super::{Action, ActionContext, ActionTiming};
use crate::npc::spec::NpcSpec;

/// Per-session scheduling bookkeeping for one action. Reset whenever the
/// registry is rebuilt (i.e. every Horizon session).
#[derive(Default)]
struct SchedState {
    last_started: Option<Instant>,
    last_finished: Option<Instant>,
    run_count: u32,
}

/// One catalogue entry: the action, its effective (default ⊕ per-NPC override)
/// timing, and its live schedule state.
struct Entry {
    action: Arc<dyn Action>,
    timing: ActionTiming,
    sched: Mutex<SchedState>,
}

/// Holds the set of actions an NPC is allowed to run, filtered from a
/// catalogue by `NpcSpec.action_profile`, together with each action's timing
/// and per-session schedule state. The scheduler enforces timing here so
/// individual actions don't hand-roll cooldowns or once-per-session flags.
pub struct ActionRegistry {
    entries: Vec<Entry>,
}

impl ActionRegistry {
    /// Build the catalogue of every known action for one NPC. Actions that are
    /// configured per NPC read what they need from `spec`.
    pub fn catalogue(spec: &NpcSpec) -> Vec<Arc<dyn Action>> {
        vec![
            Arc::new(super::detect_player_and_greet::DetectPlayerAndGreet::default()),
            Arc::new(super::go_to_point::GoToPoint::default()),
            Arc::new(super::reception_players::ReceptionPlayers::new(
                spec.place_where_work.as_ref().map(|p| p.uuid.clone()),
            )),
            Arc::new(super::going_to_work::GoingToWork::new(
                spec.place_where_work.clone(),
            )),
            Arc::new(
                super::depot_store_reception_to_temp_area::DepotStoreReceptionToTempArea::new(
                    spec.place_where_work.as_ref().map(|p| p.uuid.clone()),
                    spec.depot_store_reception_to_temp_area.clone(),
                ),
            ),
            Arc::new(super::drive_truck::DriveTruck),
            Arc::new(super::check_vehicle_components::CheckVehicleComponents),
            Arc::new(super::go_sell_object::GoSellObject),
        ]
    }

    /// Keep only catalogue entries named in the NPC's `action_profile`, and
    /// resolve each one's effective timing = code default merged with any
    /// per-NPC override in `spec.action_timing`.
    pub fn from_spec(spec: &NpcSpec) -> Self {
        let entries = Self::catalogue(spec)
            .into_iter()
            .filter(|a| spec.action_profile.iter().any(|n| n == a.name()))
            .map(|action| {
                let timing = match spec.action_timing.get(action.name()) {
                    Some(ov) => ov.apply(action.timing()),
                    None => action.timing(),
                };
                Entry {
                    action,
                    timing,
                    sched: Mutex::new(SchedState::default()),
                }
            })
            .collect();
        Self { entries }
    }

    /// Whether `entry` is past every timing gate (once / cooldown / interval)
    /// as of `now`. Cheap; does not consider `can_run`.
    fn timing_ok(entry: &Entry, now: Instant) -> bool {
        let s = entry.sched.lock().unwrap();
        if entry.timing.once && s.run_count > 0 {
            return false;
        }
        if let (Some(cd), Some(fin)) = (entry.timing.cooldown, s.last_finished) {
            if now.duration_since(fin) < cd {
                return false;
            }
        }
        if let (Some(iv), Some(start)) = (entry.timing.min_interval, s.last_started) {
            if now.duration_since(start) < iv {
                return false;
            }
        }
        true
    }

    /// Choose the highest-priority action that is both past its timing gates
    /// and whose `can_run` returns true.
    pub fn pick(&self, ctx: &ActionContext) -> Option<Arc<dyn Action>> {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|e| Self::timing_ok(e, now))
            .filter(|e| e.action.can_run(ctx))
            .max_by_key(|e| e.action.priority())
            .map(|e| Arc::clone(&e.action))
    }

    /// Effective timing for an action, e.g. so the scheduler can apply its run
    /// timeout. `None` if the name is unknown to this registry.
    pub fn timing_of(&self, name: &str) -> Option<ActionTiming> {
        self.entries
            .iter()
            .find(|e| e.action.name() == name)
            .map(|e| e.timing.clone())
    }

    /// Record that an action's run has started: stamps `last_started` and bumps
    /// `run_count` (which drives the `once` gate).
    pub fn mark_started(&self, name: &str) {
        if let Some(e) = self.entries.iter().find(|e| e.action.name() == name) {
            let mut s = e.sched.lock().unwrap();
            s.last_started = Some(Instant::now());
            s.run_count += 1;
        }
    }

    /// Record that an action's run has finished: stamps `last_finished`, which
    /// drives the `cooldown` gate.
    pub fn mark_finished(&self, name: &str) {
        if let Some(e) = self.entries.iter().find(|e| e.action.name() == name) {
            e.sched.lock().unwrap().last_finished = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use async_trait::async_trait;
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;
    use crate::{
        error::Result, horizon::client::HorizonSender, npc::state::NpcState,
        world::view::WorldView,
    };

    /// Always-eligible action with configurable name/priority/timing.
    struct Dummy {
        name: &'static str,
        timing: ActionTiming,
    }

    #[async_trait]
    impl Action for Dummy {
        fn name(&self) -> &'static str {
            self.name
        }
        fn timing(&self) -> ActionTiming {
            self.timing.clone()
        }
        fn can_run(&self, _ctx: &ActionContext) -> bool {
            true
        }
        async fn run(&self, _ctx: &mut ActionContext) -> Result<()> {
            Ok(())
        }
    }

    fn registry_with(action: Dummy) -> ActionRegistry {
        ActionRegistry {
            entries: vec![Entry {
                timing: action.timing(),
                action: Arc::new(action),
                sched: Mutex::new(SchedState::default()),
            }],
        }
    }

    fn dummy_ctx() -> ActionContext {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        ActionContext {
            npc_name: "Test".into(),
            world: Arc::new(WorldView::new("self".into())),
            state: Arc::new(NpcState::new()),
            send: HorizonSender::__test_new(tx),
            rng: StdRng::seed_from_u64(0),
        }
    }

    #[test]
    fn once_action_is_picked_only_before_it_has_run() {
        let reg = registry_with(Dummy {
            name: "once",
            timing: ActionTiming {
                once: true,
                ..Default::default()
            },
        });
        let ctx = dummy_ctx();
        assert!(reg.pick(&ctx).is_some());
        reg.mark_started("once");
        assert!(reg.pick(&ctx).is_none(), "once action must not be re-picked");
    }

    #[test]
    fn cooldown_blocks_until_elapsed() {
        let reg = registry_with(Dummy {
            name: "cd",
            timing: ActionTiming {
                cooldown: Some(Duration::from_secs(3600)),
                ..Default::default()
            },
        });
        let ctx = dummy_ctx();
        reg.mark_started("cd");
        reg.mark_finished("cd");
        assert!(
            reg.pick(&ctx).is_none(),
            "still within cooldown of last finish"
        );
    }

    #[test]
    fn unrestricted_action_is_always_eligible() {
        let reg = registry_with(Dummy {
            name: "free",
            timing: ActionTiming::default(),
        });
        let ctx = dummy_ctx();
        assert!(reg.pick(&ctx).is_some());
        reg.mark_started("free");
        reg.mark_finished("free");
        assert!(reg.pick(&ctx).is_some(), "no timing => re-eligible at once");
    }
}
