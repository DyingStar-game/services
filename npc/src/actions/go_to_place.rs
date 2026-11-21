use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{info, warn};

use super::{Action, ActionContext, trip::TripWatch};
use crate::{error::Result, npc::spec::PlaceWhereWork, npc::state::NpcStatus};

/// Blackboard keys under which the NPC remembers its spawn position (its
/// habitation) as "home". Set once per session by the runtime before the NPC
/// first moves — an NPC spawns in its habitation, so its first known position
/// *is* home and needs no configuration.
pub const HOME_POSITION_KEY: &str = "home_position";
pub const HOME_PARENT_KEY: &str = "home_parent";

/// Where a [`GoToPlace`] sends the NPC.
pub enum PlaceTarget {
    /// A place from the roster. `None` disables the action.
    Configured(Option<PlaceWhereWork>),
    /// The NPC's spawn position, remembered at session start. Used by `go_home`
    /// when no `home` is configured.
    SpawnHome,
}

impl PlaceTarget {
    /// Resolve the current `(anchor uuid, position json)` to walk to, or `None`
    /// when the target is unknown (no configured place, or home not captured
    /// yet).
    fn resolve(&self, ctx: &ActionContext) -> Option<(String, Value)> {
        match self {
            PlaceTarget::Configured(Some(p)) => Some((p.uuid.clone(), json!(p.position))),
            PlaceTarget::Configured(None) => None,
            PlaceTarget::SpawnHome => {
                let position = ctx.state.get(HOME_POSITION_KEY)?;
                // The frame the spawn position was expressed in (empty = root).
                let uuid = ctx
                    .state
                    .get(HOME_PARENT_KEY)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                Some((uuid, position))
            }
        }
    }
}

/// Generic "walk to a place and settle there" action, driven by the schedule.
///
/// It backs the MVP activity behaviours — `go_home` (off-shift), `go_to_meal`
/// (`r`) and `mandatory_activity` (`a`) — which differ only in their target and
/// the [`NpcStatus`] they travel through / settle into. The choreography mirrors
/// [`GoingToWork`](super::going_to_work::GoingToWork):
///
/// - any status other than `traveling`/`terminal`: dispatch the walk, advancing
///   to `traveling`;
/// - `traveling`: once the server reports arrival, acknowledge it and settle into
///   `terminal`;
/// - `terminal`: nothing to do (idle for the rest of the window).
///
/// The schedule gate ensures this action is only *eligible* during its activity
/// window, so an NPC whose window has passed is switched to another behaviour
/// (e.g. `going_to_work` dispatches it back to work from any settled status).
pub struct GoToPlace {
    name: &'static str,
    target: PlaceTarget,
    traveling: NpcStatus,
    terminal: NpcStatus,
    priority: i32,
    /// Watchdog over the walk in flight — see [`GoingToWork`](super::going_to_work::GoingToWork).
    trip: TripWatch,
}

impl GoToPlace {
    pub fn new(
        name: &'static str,
        target: PlaceTarget,
        traveling: NpcStatus,
        terminal: NpcStatus,
        priority: i32,
    ) -> Self {
        Self {
            name,
            target,
            traveling,
            terminal,
            priority,
            trip: TripWatch::new(),
        }
    }

    /// Walk the NPC to `uuid`/`position` and start the arrival watchdog.
    async fn dispatch(&self, ctx: &mut ActionContext, uuid: &str, position: &Value) -> Result<()> {
        // Reset any stale server-side arrival flag before walking (same
        // hazard as GoingToWork: a persisted `npc_arrived` would complete
        // the trip instantly).
        ctx.send
            .send("player", "client_action", arrived_ack_data(uuid))
            .await?;
        ctx.world.clear_arrived();
        ctx.send
            .send("player", "client_action", go_to_position_data(uuid, position))
            .await?;
        self.trip.dispatched();
        ctx.state.set_status(self.traveling);
        Ok(())
    }
}

#[async_trait]
impl Action for GoToPlace {
    fn name(&self) -> &'static str {
        self.name
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        // Need a resolvable target (a configured place, or a remembered home).
        if self.target.resolve(ctx).is_none() {
            return false;
        }
        let status = ctx.state.status();
        if status == self.terminal {
            // Already settled — idle, let ambient actions (greeting) take over.
            false
        } else if status == self.traveling {
            // En route: finalise once the server says we've arrived — or
            // re-dispatch once it has stayed silent long enough that the trip
            // is presumed lost.
            ctx.world.npc_arrived() || self.trip.timed_out()
        } else {
            // Any other status: time to set off.
            true
        }
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        let Some((uuid, position)) = self.target.resolve(ctx) else {
            return Ok(());
        };
        let status = ctx.state.status();
        if status == self.terminal {
            // Nothing to do.
        } else if status == self.traveling {
            if ctx.world.npc_arrived() {
                info!(npc = %ctx.npc_name, action = self.name, place = %uuid, "arrived");
                ctx.send
                    .send("player", "client_action", arrived_ack_data(&uuid))
                    .await?;
                ctx.world.clear_arrived();
                self.trip.arrived();
                ctx.state.set_status(self.terminal);
            } else {
                warn!(
                    npc = %ctx.npc_name,
                    action = self.name,
                    place = %uuid,
                    waiting_s = self.trip.waiting_s(),
                    my_parent = ?ctx.world.my_parent(),
                    "no arrival notice; setting off again"
                );
                self.dispatch(ctx, &uuid, &position).await?;
            }
        } else {
            info!(npc = %ctx.npc_name, action = self.name, place = %uuid, "setting off");
            self.dispatch(ctx, &uuid, &position).await?;
        }
        Ok(())
    }
}

fn go_to_position_data(uuid: &str, position: &Value) -> Value {
    json!({
        "action": "npc_go_to_position",
        "uuid": uuid,
        "position": position,
    })
}

fn arrived_ack_data(uuid: &str) -> Value {
    json!({
        "action": "npc_arrived_ack",
        "uuid": uuid,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::{SeedableRng, rngs::StdRng};
    use serde_json::json;

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind, Vec3},
        npc::state::NpcState,
        world::view::WorldView,
    };

    fn place() -> PlaceWhereWork {
        PlaceWhereWork {
            uuid: "home-uuid".into(),
            planet: None,
            position: Vec3 { x: 1.0, y: 0.0, z: 2.0 },
        }
    }

    fn ctx() -> (ActionContext, tokio::sync::mpsc::Receiver<Value>) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let ctx = ActionContext {
            npc_name: "Test".into(),
            world: Arc::new(WorldView::new("self".into())),
            state: Arc::new(NpcState::new()),
            send: HorizonSender::__test_new(tx),
            rng: StdRng::seed_from_u64(0),
            allowed_actions: None,
            depot_role: crate::depot::DepotRole::Uncoordinated,
        };
        (ctx, rx)
    }

    fn mark_arrived(ctx: &ActionContext) {
        ctx.world.apply(&GorcFrame {
            kind: GorcKind::Update,
            channel: 0,
            object_id: "self".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "npc_arrived": true }),
        });
    }

    fn go_home_configured() -> GoToPlace {
        GoToPlace::new(
            "go_home",
            PlaceTarget::Configured(Some(place())),
            NpcStatus::GoingHome,
            NpcStatus::AtHome,
            120,
        )
    }

    fn go_home_spawn() -> GoToPlace {
        GoToPlace::new(
            "go_home",
            PlaceTarget::SpawnHome,
            NpcStatus::GoingHome,
            NpcStatus::AtHome,
            120,
        )
    }

    #[test]
    fn no_target_never_runs() {
        let (c, _rx) = ctx();
        c.state.set_status(NpcStatus::AtWork);
        // Configured with no place: disabled.
        let a = GoToPlace::new("go_home", PlaceTarget::Configured(None), NpcStatus::GoingHome, NpcStatus::AtHome, 120);
        assert!(!a.can_run(&c));
        // SpawnHome before the spawn position is remembered: disabled.
        assert!(!go_home_spawn().can_run(&c));
    }

    #[test]
    fn spawn_home_resolves_once_remembered() {
        let (c, _rx) = ctx();
        c.state.set_status(NpcStatus::AtWork);
        c.state
            .set(HOME_POSITION_KEY, json!({ "x": 3.0, "y": 0.5, "z": 4.0 }));
        c.state.set(HOME_PARENT_KEY, json!("depot-uuid"));
        assert!(go_home_spawn().can_run(&c));
    }

    #[test]
    fn dispatches_from_any_non_terminal_status() {
        let (c, _rx) = ctx();
        c.state.set_status(NpcStatus::AtWork);
        assert!(go_home_configured().can_run(&c));
        c.state.set_status(NpcStatus::AtMeal);
        assert!(go_home_configured().can_run(&c));
    }

    #[test]
    fn idle_once_terminal() {
        let (c, _rx) = ctx();
        c.state.set_status(NpcStatus::AtHome);
        mark_arrived(&c); // even a lingering flag must not restart it
        assert!(!go_home_configured().can_run(&c));
    }

    #[tokio::test]
    async fn full_trip_dispatch_then_arrival() {
        let (mut c, mut rx) = ctx();
        c.state.set_status(NpcStatus::AtWork);
        let a = go_home_configured();

        // Dispatch: resets stale arrival, then orders the walk, advances to traveling.
        a.run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        let walk = rx.try_recv().unwrap();
        assert_eq!(walk["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(walk["data"]["position"], json!({ "x": 1.0, "y": 0.0, "z": 2.0 }));
        assert_eq!(c.state.status(), NpcStatus::GoingHome);
        assert!(!c.world.npc_arrived());

        // Still travelling: stands down.
        assert!(!a.can_run(&c));

        // Arrival: acks and settles home.
        mark_arrived(&c);
        assert!(a.can_run(&c));
        a.run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(c.state.status(), NpcStatus::AtHome);
        assert!(!c.world.npc_arrived());
    }

    #[tokio::test]
    async fn spawn_home_walks_to_remembered_position() {
        let (mut c, mut rx) = ctx();
        c.state.set_status(NpcStatus::AtWork);
        c.state
            .set(HOME_POSITION_KEY, json!({ "x": 3.0, "y": 0.5, "z": 4.0 }));
        c.state.set(HOME_PARENT_KEY, json!("depot-uuid"));

        go_home_spawn().run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        let walk = rx.try_recv().unwrap();
        assert_eq!(walk["data"]["uuid"], json!("depot-uuid"));
        assert_eq!(walk["data"]["position"], json!({ "x": 3.0, "y": 0.5, "z": 4.0 }));
        assert_eq!(c.state.status(), NpcStatus::GoingHome);
    }

    #[tokio::test]
    async fn an_unanswered_trip_is_set_off_again() {
        let (mut ctx, mut rx) = ctx();
        let a = GoToPlace::new(
            "go_to_meal",
            PlaceTarget::Configured(Some(place())),
            NpcStatus::GoingToMeal,
            NpcStatus::AtMeal,
            120,
        );

        a.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.state.status(), NpcStatus::GoingToMeal);
        while rx.try_recv().is_ok() {}

        // En route, no arrival notice yet: stand down.
        assert!(!a.can_run(&ctx));

        // The server never answered: set off again instead of idling all window.
        a.trip.age_by(crate::actions::trip::ARRIVAL_TIMEOUT);
        assert!(a.can_run(&ctx));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(ctx.state.status(), NpcStatus::GoingToMeal);
        assert!(!a.can_run(&ctx));
    }
}
