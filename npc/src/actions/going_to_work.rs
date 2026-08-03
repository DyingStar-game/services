use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::info;

use super::{Action, ActionContext};
use crate::{error::Result, npc::spec::PlaceWhereWork, npc::state::NpcStatus};

/// Walks an NPC from its habitation to its `place_where_work` and completes the
/// trip when the server reports arrival.
///
/// Lifecycle, driven by [`NpcStatus`]:
/// - `AtHome` (set at spawn for a working NPC): dispatch the go-to-position
///   command and advance to `GoingToWork`.
/// - `GoingToWork`: wait until the server sets the `npc_arrived` player
///   property, then acknowledge it and advance to `AtWork`.
///
/// State is rebuilt per Horizon session, so a reconnected NPC (respawned back
/// in its habitation as `AtHome`) is sent to work again.
pub struct GoingToWork {
    place: Option<PlaceWhereWork>,
}

impl GoingToWork {
    pub fn new(place: Option<PlaceWhereWork>) -> Self {
        Self { place }
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
            // Ready to leave home.
            NpcStatus::AtHome => true,
            // En route: only act once the server says we've arrived. Otherwise
            // yield the slot so other actions can run during the trip.
            NpcStatus::GoingToWork => ctx.world.npc_arrived(),
            _ => false,
        }
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        let Some(place) = &self.place else {
            return Ok(());
        };
        match ctx.state.status() {
            NpcStatus::AtHome => {
                info!(npc = %ctx.npc_name, place = %place.uuid, "going to work");
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
                // Now travelling; `whereami` is effectively "travel to work".
                ctx.state.set_status(NpcStatus::GoingToWork);
            }
            NpcStatus::GoingToWork if ctx.world.npc_arrived() => {
                info!(npc = %ctx.npc_name, place = %place.uuid, "arrived at work");
                ctx.send
                    .send("player", "client_action", arrived_ack_data(place))
                    .await?;
                // Drop the local flag so a stale echo cannot re-ack, and settle
                // into the workplace.
                ctx.world.clear_arrived();
                ctx.state.set_status(NpcStatus::AtWork);
            }
            // Nothing to do (still travelling, or already at work).
            _ => {}
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
    fn dispatches_only_from_at_home() {
        let (ctx, _rx) = dummy_ctx();
        let a = GoingToWork::new(Some(place()));
        // Idle at construction: nothing to do until spawned at home.
        assert!(!a.can_run(&ctx));
        ctx.state.set_status(NpcStatus::AtHome);
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
}
