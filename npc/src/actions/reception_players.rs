use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use super::{Action, ActionContext, ActionTiming};
use crate::{error::Result, npc::state::NpcStatus};

/// How often the NPC checks its depot's reception while at work.
const CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// A player who leaves the reception and comes back within this window is not
/// greeted again.
const RETURN_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// What the NPC tells players waiting at the reception.
const RECEPTION_GREETING: &str = "Posez la boite sur le comptoir";

/// While the NPC is [`AtWork`](NpcStatus::AtWork), watches the depot it works at
/// — the object whose UUID is the NPC's configured `place_where_work`. When a
/// player newly appears in that depot's `reception_players` property, the NPC
/// tells them to drop their box on the counter — once: a player who stays
/// listed is not greeted again, and one who leaves and returns within
/// [`RETURN_COOLDOWN`] is not either. Polls every 2 seconds.
pub struct ReceptionPlayers {
    /// UUID of the depot the NPC works at, taken from `place_where_work`. `None`
    /// means the NPC has no workplace, so this action never runs.
    work_uuid: Option<String>,
    /// How long a player must have been away before being greeted again.
    cooldown: Duration,
    /// When each player was last seen at the reception. Refreshed on every
    /// poll while the player stays listed, so after they leave the entry holds
    /// (roughly) the moment of departure. A player is greeted only when they
    /// have no entry younger than `cooldown`.
    last_seen: Mutex<HashMap<String, Instant>>,
}

impl ReceptionPlayers {
    pub fn new(work_uuid: Option<String>) -> Self {
        Self {
            work_uuid,
            cooldown: RETURN_COOLDOWN,
            last_seen: Mutex::new(HashMap::new()),
        }
    }

    /// Players currently at the reception who deserve a greeting: never seen
    /// before, or away for longer than `cooldown`. Also refreshes `last_seen`
    /// for every listed player and drops expired entries.
    fn newly_arrived(&self, ctx: &ActionContext) -> Vec<String> {
        let now = Instant::now();
        let mut seen = self.last_seen.lock().unwrap();
        // An entry older than the cooldown no longer suppresses a greeting,
        // so it can go — this also keeps the map from growing forever.
        seen.retain(|_, t| now.duration_since(*t) < self.cooldown);
        self.waiting_players(ctx)
            .into_iter()
            .filter(|p| seen.insert(p.clone(), now).is_none())
            .collect()
    }

    /// Player UUIDs currently listed in the depot's `reception_players` array.
    /// Empty when the NPC has no workplace, the depot is not in view, or the
    /// property is absent or not an array.
    fn waiting_players(&self, ctx: &ActionContext) -> Vec<String> {
        let Some(uuid) = &self.work_uuid else {
            return Vec::new();
        };
        let Some(depot) = ctx.world.objects.get(uuid) else {
            return Vec::new();
        };
        depot
            .property("reception_players")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
}

#[async_trait]
impl Action for ReceptionPlayers {
    fn name(&self) -> &'static str {
        "reception_players"
    }

    fn priority(&self) -> i32 {
        90
    }

    fn timing(&self) -> ActionTiming {
        // Poll the reception at ~0.5 Hz rather than on every scheduler tick.
        ActionTiming {
            min_interval: Some(CHECK_INTERVAL),
            ..Default::default()
        }
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        // Only while at the workplace, and only when at least one player is
        // waiting at the reception. Deliberately does NOT check whether anyone
        // would actually be greeted: `run` must keep firing while players
        // linger so their `last_seen` stays fresh — otherwise a player who
        // simply waits longer than the cooldown would be greeted twice.
        ctx.state.status() == NpcStatus::AtWork && !self.waiting_players(ctx).is_empty()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        for player in self.newly_arrived(ctx) {
            info!(
                npc = %ctx.npc_name,
                player = %player,
                text = %RECEPTION_GREETING,
                "telling reception player to drop the box on the counter"
            );
            ctx.send
                .send(
                    "player",
                    "client_action",
                    json!({
                        "action": "dialog",
                        "to": player,
                        "text": RECEPTION_GREETING,
                        "uuid": ctx.world.my_uuid,
                    }),
                )
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::{SeedableRng, rngs::StdRng};
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind},
        npc::state::NpcState,
        world::view::WorldView,
    };

    const DEPOT: &str = "a9340169-767c-4b02-a2f3-67f2786375b3";

    fn dummy_ctx(world: Arc<WorldView>) -> (ActionContext, tokio::sync::mpsc::Receiver<Value>) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let ctx = ActionContext {
            npc_name: "Test".into(),
            world,
            state: Arc::new(NpcState::new()),
            send: HorizonSender::__test_new(tx),
            rng: StdRng::seed_from_u64(0),
            allowed_actions: None,
            depot_role: crate::depot::DepotRole::Uncoordinated,
        };
        (ctx, rx)
    }

    /// A depot object entering a zone with the given `reception_players` value.
    fn depot_frame(reception_players: Value) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 6,
            object_id: DEPOT.into(),
            object_type: "depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "position": {"x": 0.0, "y": 0.0, "z": 0.0},
                "reception_players": reception_players,
            }),
        }
    }

    /// The action wired to the depot above, i.e. an NPC that works there.
    fn action() -> ReceptionPlayers {
        ReceptionPlayers::new(Some(DEPOT.to_string()))
    }

    #[test]
    fn cannot_run_when_not_at_work() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (ctx, _rx) = dummy_ctx(world);
        // Status defaults to Idle: even a full reception must not fire.
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn cannot_run_without_a_workplace() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!ReceptionPlayers::new(None).can_run(&ctx));
    }

    #[test]
    fn cannot_run_when_depot_not_in_view() {
        let world = Arc::new(WorldView::new("self".into()));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn cannot_run_when_reception_empty() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&depot_frame(json!([])));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn can_run_when_at_work_and_players_waiting() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(action().can_run(&ctx));
    }

    #[tokio::test]
    async fn run_greets_each_waiting_player() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!(["p1", "p2"])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        assert_eq!(
            rx.try_recv().unwrap(),
            json!({
                "namespace": "player",
                "event": "client_action",
                "data": {
                    "action": "dialog",
                    "to": "p1",
                    "text": "Posez la boite sur le comptoir",
                    "uuid": "me",
                },
            })
        );
        assert_eq!(rx.try_recv().unwrap()["data"]["to"], json!("p2"));
        assert!(rx.try_recv().is_err(), "one dialog per waiting player");
    }

    #[tokio::test]
    async fn does_not_greet_a_player_who_stays_at_the_reception() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = action();

        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["to"], json!("p1"));

        // Next poll: p1 is still listed — no second greeting.
        a.run(&mut ctx).await.unwrap();
        assert!(rx.try_recv().is_err(), "a lingering player is greeted once");
    }

    #[tokio::test]
    async fn greets_only_the_newcomer_when_reception_grows() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = action();

        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["to"], json!("p1"));

        ctx.world.apply(&depot_frame(json!(["p1", "p2"])));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["to"], json!("p2"));
        assert!(rx.try_recv().is_err(), "p1 must not be greeted again");
    }

    #[tokio::test]
    async fn does_not_greet_a_player_returning_within_the_cooldown() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = action();

        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["to"], json!("p1"));

        // p1 leaves, then comes back before the 10-minute cooldown elapsed.
        ctx.world.apply(&depot_frame(json!([])));
        ctx.world.apply(&depot_frame(json!(["p1"])));
        a.run(&mut ctx).await.unwrap();
        assert!(rx.try_recv().is_err(), "quick return must stay silent");
    }

    #[tokio::test]
    async fn greets_again_once_the_cooldown_has_expired() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!(["p1"])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let mut a = action();
        a.cooldown = Duration::from_millis(20);

        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["to"], json!("p1"));

        // p1 leaves and only returns after the cooldown has fully elapsed.
        ctx.world.apply(&depot_frame(json!([])));
        tokio::time::sleep(Duration::from_millis(40)).await;
        ctx.world.apply(&depot_frame(json!(["p1"])));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(
            rx.try_recv().unwrap()["data"]["to"],
            json!("p1"),
            "a player away longer than the cooldown is greeted anew"
        );
    }
}
