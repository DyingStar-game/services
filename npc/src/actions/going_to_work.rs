use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{info, warn};

use super::{Action, ActionContext, trip::TripWatch};
use crate::{error::Result, npc::spec::PlaceWhereWork, npc::state::NpcStatus};

/// Walks an NPC from its habitation to its `place_where_work` and completes the
/// trip when the server reports arrival.
///
/// Lifecycle, driven by [`NpcStatus`]:
/// - any settled status other than `AtWork`/`GoingToWork` (`AtHome` at spawn,
///   or `AtMeal`/`DoingMandatory`/`GoingHome` when a work window follows a
///   break): dispatch the go-to-position command and advance to `GoingToWork`.
/// - `GoingToWork`: wait until the server sets the `npc_arrived` player
///   property, then acknowledge it and advance to `AtWork`.
/// - `AtWork`: nothing to do.
///
/// Because it resumes work from *any* non-work status, it is the universal
/// "return to work" behaviour — the shift schedule only makes it *eligible*
/// during `T`/`t` windows, so it never fights the break behaviours.
///
/// State is rebuilt per Horizon session, so a reconnected NPC (respawned back
/// in its habitation as `AtHome`) is sent to work again.
pub struct GoingToWork {
    place: Option<PlaceWhereWork>,
    /// Watchdog over the walk in flight: the server may never answer with
    /// `npc_arrived` (a dropped order, or no path out of the habitation the NPC
    /// spawned in), and without this the NPC would wait for it all shift.
    trip: TripWatch,
}

impl GoingToWork {
    pub fn new(place: Option<PlaceWhereWork>) -> Self {
        Self {
            place,
            trip: TripWatch::new(),
        }
    }

    /// Send the NPC to `place` and start the arrival watchdog.
    async fn dispatch(&self, ctx: &mut ActionContext, place: &PlaceWhereWork) -> Result<()> {
        // A stale `npc_arrived: true` persisted from a previous session
        // replicates with the spawn data and would complete this trip
        // instantly, leaving the NPC standing at home while believing
        // it is at work. Reset it on the server (the ack clears and
        // re-replicates it) and locally before walking.
        ctx.send
            .send("player", "client_action", arrived_ack_data(place))
            .await?;
        ctx.world.clear_arrived();
        ctx.send
            .send("player", "client_action", go_to_work_data(place))
            .await?;
        self.trip.dispatched();
        // Now travelling; `whereami` is effectively "travel to work".
        ctx.state.set_status(NpcStatus::GoingToWork);
        Ok(())
    }
}

#[async_trait]
impl Action for GoingToWork {
    fn name(&self) -> &'static str {
        "going_to_work"
    }

    fn priority(&self) -> i32 {
        // Above every other action: dispatching to work and acknowledging
        // arrival take precedence. While merely travelling (see `can_run`) this
        // action stands down, so other behaviours still run en route.
        200
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        if self.place.is_none() {
            return false;
        }
        match ctx.state.status() {
            // Already at work: nothing to do.
            NpcStatus::AtWork => false,
            // En route: act once the server says we've arrived — or once it has
            // stayed silent long enough that the trip is presumed lost, so it
            // can be re-dispatched. Otherwise yield the slot so other actions
            // still run during the trip.
            NpcStatus::GoingToWork => ctx.world.npc_arrived() || self.trip.timed_out(),
            // Any other settled status (home, meal, mandatory, …): go to work.
            _ => true,
        }
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        let Some(place) = &self.place else {
            return Ok(());
        };
        match ctx.state.status() {
            // Already at work: nothing to do.
            NpcStatus::AtWork => {}
            // Travelling: complete the trip once the server reports arrival.
            NpcStatus::GoingToWork => {
                if ctx.world.npc_arrived() {
                    info!(npc = %ctx.npc_name, place = %place.uuid, "arrived at work");
                    ctx.send
                        .send("player", "client_action", arrived_ack_data(place))
                        .await?;
                    // Drop the local flag so a stale echo cannot re-ack, and
                    // settle into the workplace.
                    ctx.world.clear_arrived();
                    self.trip.arrived();
                    ctx.state.set_status(NpcStatus::AtWork);
                } else {
                    warn!(
                        npc = %ctx.npc_name,
                        place = %place.uuid,
                        waiting_s = self.trip.waiting_s(),
                        my_parent = ?ctx.world.my_parent(),
                        "no arrival notice; asking to walk to work again"
                    );
                    self.dispatch(ctx, place).await?;
                }
            }
            // Any other settled status: dispatch to work.
            _ => {
                info!(npc = %ctx.npc_name, place = %place.uuid, "going to work");
                self.dispatch(ctx, place).await?;
            }
        }
        Ok(())
    }
}

/// Payload of the `player/client_action` frame that walks an NPC to its place
/// of work.
fn go_to_work_data(place: &PlaceWhereWork) -> Value {
    json!({
        "action": "npc_go_to_position",
        "uuid": place.uuid,
        "position": place.position,
    })
}

/// Payload of the `player/client_action` frame that acknowledges arrival at the
/// place of work, so the server can clear its `npc_arrived` flag.
fn arrived_ack_data(place: &PlaceWhereWork) -> Value {
    json!({
        "action": "npc_arrived_ack",
        "uuid": place.uuid,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::{SeedableRng, rngs::StdRng};

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind, Vec3},
        npc::state::NpcState,
        world::view::WorldView,
    };

    fn place() -> PlaceWhereWork {
        PlaceWhereWork {
            uuid: "ed20bda3-f6f3-4053-b9de-968f73ebc44c".into(),
            planet: None,
            position: Vec3 {
                x: 50.858,
                y: 1.0,
                z: -37.942,
            },
        }
    }

    fn dummy_ctx() -> (ActionContext, tokio::sync::mpsc::Receiver<Value>) {
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

    /// Feed a self frame carrying `npc_arrived: true` so the world reports the
    /// NPC as having arrived, the way Horizon does at the workplace.
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

    #[test]
    fn builds_go_to_position_payload() {
        assert_eq!(
            go_to_work_data(&place()),
            json!({
                "action": "npc_go_to_position",
                "uuid": "ed20bda3-f6f3-4053-b9de-968f73ebc44c",
                "position": {"x": 50.858, "y": 1.0, "z": -37.942},
            })
        );
    }

    #[test]
    fn builds_arrived_ack_payload() {
        assert_eq!(
            arrived_ack_data(&place()),
            json!({
                "action": "npc_arrived_ack",
                "uuid": "ed20bda3-f6f3-4053-b9de-968f73ebc44c",
            })
        );
    }

    #[test]
    fn cannot_run_without_a_place_of_work() {
        let (ctx, _rx) = dummy_ctx();
        ctx.state.set_status(NpcStatus::AtHome);
        assert!(!GoingToWork::new(None).can_run(&ctx));
    }

    #[test]
    fn dispatches_from_any_non_work_status() {
        let (ctx, _rx) = dummy_ctx();
        let a = GoingToWork::new(Some(place()));
        // Dispatches from home…
        ctx.state.set_status(NpcStatus::AtHome);
        assert!(a.can_run(&ctx));
        // …and when resuming work after a break (meal / mandatory).
        ctx.state.set_status(NpcStatus::AtMeal);
        assert!(a.can_run(&ctx));
        ctx.state.set_status(NpcStatus::DoingMandatory);
        assert!(a.can_run(&ctx));
    }

    #[test]
    fn stands_down_while_travelling_until_arrival() {
        let (ctx, _rx) = dummy_ctx();
        let a = GoingToWork::new(Some(place()));
        ctx.state.set_status(NpcStatus::GoingToWork);
        // Travelling, no arrival yet: yield the slot to other actions.
        assert!(!a.can_run(&ctx));
        mark_arrived(&ctx);
        assert!(a.can_run(&ctx));
    }

    #[test]
    fn done_once_at_work() {
        let (ctx, _rx) = dummy_ctx();
        let a = GoingToWork::new(Some(place()));
        ctx.state.set_status(NpcStatus::AtWork);
        // Even a lingering arrival flag must not restart the trip.
        mark_arrived(&ctx);
        assert!(!a.can_run(&ctx));
    }

    #[tokio::test]
    async fn dispatch_sends_go_to_position_and_advances_to_going_to_work() {
        let (mut ctx, mut rx) = dummy_ctx();
        ctx.state.set_status(NpcStatus::AtHome);
        let a = GoingToWork::new(Some(place()));

        a.run(&mut ctx).await.unwrap();

        // The dispatch first resets any stale server-side arrival flag…
        assert_eq!(
            rx.try_recv().unwrap()["data"]["action"],
            json!("npc_arrived_ack")
        );
        // …then orders the walk.
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({
                "namespace": "player",
                "event": "client_action",
                "data": {
                    "action": "npc_go_to_position",
                    "uuid": "ed20bda3-f6f3-4053-b9de-968f73ebc44c",
                    "position": {"x": 50.858, "y": 1.0, "z": -37.942},
                },
            })
        );
        assert_eq!(ctx.state.status(), NpcStatus::GoingToWork);
    }

    #[tokio::test]
    async fn stale_persisted_arrival_flag_does_not_complete_the_trip_instantly() {
        let (mut ctx, _rx) = dummy_ctx();
        ctx.state.set_status(NpcStatus::AtHome);
        // A previous session's `npc_arrived: true` replicated with the spawn.
        mark_arrived(&ctx);
        let a = GoingToWork::new(Some(place()));

        a.run(&mut ctx).await.unwrap();

        assert_eq!(ctx.state.status(), NpcStatus::GoingToWork);
        assert!(
            !ctx.world.npc_arrived(),
            "stale flag must be cleared by the dispatch"
        );
        assert!(!a.can_run(&ctx), "must actually travel, not arrive instantly");
    }

    #[tokio::test]
    async fn arrival_acks_advances_to_at_work_and_clears_flag() {
        let (mut ctx, mut rx) = dummy_ctx();
        ctx.state.set_status(NpcStatus::GoingToWork);
        mark_arrived(&ctx);
        let a = GoingToWork::new(Some(place()));

        a.run(&mut ctx).await.unwrap();

        assert_eq!(
            rx.try_recv().unwrap(),
            json!({
                "namespace": "player",
                "event": "client_action",
                "data": {
                    "action": "npc_arrived_ack",
                    "uuid": "ed20bda3-f6f3-4053-b9de-968f73ebc44c",
                },
            })
        );
        assert_eq!(ctx.state.status(), NpcStatus::AtWork);
        assert!(!ctx.world.npc_arrived(), "arrival flag cleared after ack");

        // Already at work: further runs are no-ops.
        a.run(&mut ctx).await.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn an_unanswered_trip_is_dispatched_again() {
        // `npc_go_to_position` is fire-and-forget: when the server never answers
        // with `npc_arrived` (a dropped order, or no path out of the habitation
        // the NPC spawned in), the NPC used to wait for it for the whole shift.
        let (mut ctx, mut rx) = dummy_ctx();
        ctx.state.set_status(NpcStatus::AtHome);
        let a = GoingToWork::new(Some(place()));

        a.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.state.status(), NpcStatus::GoingToWork);
        while rx.try_recv().is_ok() {}

        // Mid-trip with no arrival notice: stand down, the walk is under way.
        assert!(!a.can_run(&ctx));

        // Still nothing after the timeout: walk to work again rather than wait
        // on a flag that is not coming.
        a.trip.age_by(crate::actions::trip::ARRIVAL_TIMEOUT);
        assert!(a.can_run(&ctx));
        a.run(&mut ctx).await.unwrap();
        let ack = rx.try_recv().unwrap();
        assert_eq!(ack["data"]["action"], json!("npc_arrived_ack"));
        let go = rx.try_recv().unwrap();
        assert_eq!(go["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(ctx.state.status(), NpcStatus::GoingToWork);

        // The retry restarts the clock: no second re-dispatch straight away.
        assert!(!a.can_run(&ctx));

        // And a real arrival still wins over the watchdog.
        mark_arrived(&ctx);
        assert!(a.can_run(&ctx));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.state.status(), NpcStatus::AtWork);
    }
}
