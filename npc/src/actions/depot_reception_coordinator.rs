//! Drives the visible, server-arbitrated reception negotiation between the
//! magasiniers at a cargo depot.
//!
//! The authoritative decision is the depot's replicated `reception_owner`
//! property (granted first-come by the Godot game server). This action makes the
//! NPC *pursue* and *voice* that decision:
//!
//! * counter free (`reception_owner == ""`) → say "Réception libre ?" and send a
//!   `claim_reception`; the server grants it to the first claimant;
//! * we won it (`reception_owner == me`) → say "Je prends la réception.";
//! * someone else owns it → once we HEAR them claim it (their replicated
//!   `conversation` line), say "Ok, je m'occupe des rayons." and go stock;
//! * leaving work → send `release_reception` so the role never sticks (the game
//!   server also clears it on disconnect).
//!
//! The role split itself is enforced by [`ActionContext::depot_role`] in the two
//! depot actions; this action only claims/releases and speaks. It is
//! edge-triggered — `can_run` is true only when there is a discrete claim, line,
//! or release owed — so it never starves the real work.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use super::{Action, ActionContext, ActionTiming};
use crate::{depot::DepotRole, error::Result, npc::state::NpcStatus};

/// Line said when the counter looks free, just before claiming it.
const OPEN_LINE: &str = "Réception libre ?";
/// Line said once we own the reception counter.
const TAKE_LINE: &str = "Je prends la réception.";
/// Line said once another magasinier owns reception and we heard them take it.
const STOCK_LINE: &str = "Ok, je m'occupe des rayons.";

/// Blackboard: the `reception_owner` value we last acted on ("" = free). A change
/// means a fresh situation, so the "already said / already claimed" flags reset.
const LAST_OWNER_KEY: &str = "depot_coord_last_owner";
/// Blackboard: which line we last spoke for the current owner (`open`/`take`/`stock`).
const SPOKEN_KEY: &str = "depot_coord_spoken";
/// Blackboard: we have an outstanding claim/ownership to release.
const CLAIM_SENT_KEY: &str = "depot_coord_claim_sent";
/// Blackboard: the `reception_owner` value we last sent a `release_reception`
/// for. Keeps the release to one send per situation instead of one per tick
/// while the server catches up.
const RELEASED_KEY: &str = "depot_coord_released";

/// One discrete thing the coordinator may do this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Counter free: open the exchange and send `claim_reception`.
    Claim,
    /// We own it: announce it.
    SayTake,
    /// Someone else owns it and we heard them: announce we'll stock.
    SayStock,
    /// We stopped working: hand the role back.
    Release,
}

/// Coordinates the reception hand-off at the NPC's depot.
pub struct DepotReceptionCoordinator {
    /// UUID of the depot the NPC works at. `None` = no workplace, so this action
    /// never runs.
    work_uuid: Option<String>,
}

impl DepotReceptionCoordinator {
    pub fn new(work_uuid: Option<String>) -> Self {
        Self { work_uuid }
    }

    /// Current `reception_owner` string ("" when free), or `None` when the depot
    /// or the property is not in view (older game server → no claim flow, the
    /// deterministic fallback role is used instead).
    fn owner(&self, ctx: &ActionContext) -> Option<String> {
        let depot = self.work_uuid.as_ref()?;
        let d = ctx.world.objects.get(depot)?;
        let v = d.property("reception_owner")?;
        Some(v.as_str().unwrap_or("").to_string())
    }

    fn spoken(ctx: &ActionContext) -> Option<String> {
        ctx.state.get(SPOKEN_KEY).and_then(|v| v.as_str().map(str::to_string))
    }

    fn claim_sent(ctx: &ActionContext) -> bool {
        ctx.state.get(CLAIM_SENT_KEY).is_some()
    }

    /// The one discrete step owed this tick, if any. Shared by `can_run` and
    /// `run` so they never disagree.
    fn next_step(&self, ctx: &ActionContext) -> Option<Step> {
        self.work_uuid.as_ref()?;

        let my_uuid = &ctx.world.my_uuid;

        // Off the clock: hand the role back. Visible ownership counts as much as
        // our own claim flag — the game server can grant the counter on its own
        // (e.g. to the first NPC crossing the reception area), and a role we
        // never claimed is exactly the one we would otherwise never release,
        // leaving the co-workers still on shift unable to take a deposit.
        if ctx.state.status() != NpcStatus::AtWork {
            let owned = self.owner(ctx).as_deref() == Some(my_uuid.as_str());
            if !owned && !Self::claim_sent(ctx) {
                return None;
            }
            // Already released for this exact situation: wait for the server to
            // clear the property rather than re-sending every tick.
            let released = ctx.state.get(RELEASED_KEY).and_then(|v| v.as_str().map(str::to_string));
            if owned && released.as_deref() == Some(my_uuid.as_str()) {
                return None;
            }
            return Some(Step::Release);
        }

        let owner = self.owner(ctx)?;
        // A changed owner is a fresh situation: the per-owner flags are stale.
        let fresh = ctx.state.get(LAST_OWNER_KEY).and_then(|v| v.as_str().map(str::to_string))
            != Some(owner.clone());
        let spoken = if fresh { None } else { Self::spoken(ctx) };
        let claimed = !fresh && Self::claim_sent(ctx);

        if owner == *my_uuid {
            (spoken.as_deref() != Some("take")).then_some(Step::SayTake)
        } else if !owner.is_empty() && ctx.depot_role != DepotRole::Reception {
            if spoken.as_deref() == Some("stock") {
                None
            } else if ctx.world.heard_from(&owner).is_some() {
                // We actually heard the owner claim it — reply and go stock.
                Some(Step::SayStock)
            } else {
                None
            }
        } else if claimed {
            None
        } else {
            Some(Step::Claim)
        }
    }

    /// Say `text` near the depot (the server shows it to nearby players and
    /// replicates it as our `conversation`, so co-workers hear it). `to` is
    /// ignored by the server's dialog handler, so any recipient works.
    async fn say(&self, ctx: &mut ActionContext, to: &str, text: &str) -> Result<()> {
        info!(npc = %ctx.npc_name, %text, "depot reception negotiation");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "dialog",
                    "to": to,
                    "text": text,
                    "uuid": ctx.world.my_uuid,
                }),
            )
            .await
    }

    async fn send_claim(&self, ctx: &mut ActionContext, action: &str, depot: &str) -> Result<()> {
        ctx.send
            .send(
                "player",
                "client_action",
                json!({ "action": action, "uuid": depot }),
            )
            .await
    }
}

#[async_trait]
impl Action for DepotReceptionCoordinator {
    fn name(&self) -> &'static str {
        "depot_reception_coordinator"
    }

    fn priority(&self) -> i32 {
        // Above the depot work actions (95/94) and the break travels (120) so a
        // claim/line/release happens promptly, but below going_to_work (200).
        // Edge-triggered `can_run` keeps it from starving anything.
        130
    }

    fn timing(&self) -> ActionTiming {
        ActionTiming {
            min_interval: Some(Duration::from_secs(1)),
            ..Default::default()
        }
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        self.next_step(ctx).is_some()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        let Some(step) = self.next_step(ctx) else {
            return Ok(());
        };
        let Some(depot) = self.work_uuid.clone() else {
            return Ok(());
        };
        let my_uuid = ctx.world.my_uuid.clone();

        match step {
            Step::Claim => {
                // Record the owner we saw, not a bare "": the claim may be
                // taking a STALE counter over (an off-shift owner the runtime
                // told us to replace), and remembering "" there would read as a
                // fresh situation on the next tick and re-claim every second.
                let owner = self.owner(ctx).unwrap_or_default();
                self.say(ctx, &my_uuid, OPEN_LINE).await?;
                self.send_claim(ctx, "claim_reception", &depot).await?;
                ctx.state.set(LAST_OWNER_KEY, json!(owner));
                ctx.state.set(SPOKEN_KEY, json!("open"));
                ctx.state.set(CLAIM_SENT_KEY, json!(true));
                ctx.state.remove(RELEASED_KEY);
            }
            Step::SayTake => {
                self.say(ctx, &my_uuid, TAKE_LINE).await?;
                ctx.state.set(LAST_OWNER_KEY, json!(my_uuid));
                ctx.state.set(SPOKEN_KEY, json!("take"));
                ctx.state.set(CLAIM_SENT_KEY, json!(true));
                ctx.state.remove(RELEASED_KEY);
            }
            Step::SayStock => {
                let owner = self.owner(ctx).unwrap_or_default();
                let to = if owner.is_empty() { my_uuid.clone() } else { owner.clone() };
                self.say(ctx, &to, STOCK_LINE).await?;
                ctx.state.set(LAST_OWNER_KEY, json!(owner));
                ctx.state.set(SPOKEN_KEY, json!("stock"));
                // We don't own reception: nothing to release later.
                ctx.state.remove(CLAIM_SENT_KEY);
                ctx.state.remove(RELEASED_KEY);
            }
            Step::Release => {
                let owner = self.owner(ctx).unwrap_or_default();
                info!(npc = %ctx.npc_name, "handing the reception counter back");
                self.send_claim(ctx, "release_reception", &depot).await?;
                ctx.state.remove(LAST_OWNER_KEY);
                ctx.state.remove(SPOKEN_KEY);
                ctx.state.remove(CLAIM_SENT_KEY);
                ctx.state.set(RELEASED_KEY, json!(owner));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::{SeedableRng, rngs::StdRng};
    use serde_json::{Value, json};
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind},
        npc::state::{NpcState, NpcStatus},
        world::view::WorldView,
    };

    const DEPOT: &str = "depot-1";
    const ME: &str = "me";
    const OTHER: &str = "other";

    fn ctx_for(world: Arc<WorldView>) -> (ActionContext, mpsc::Receiver<Value>) {
        let (tx, rx) = mpsc::channel(16);
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

    fn world_with_owner(owner: Option<&str>) -> Arc<WorldView> {
        let world = Arc::new(WorldView::new(ME.into()));
        let mut data = json!({ "position": {"x":0.0,"y":0.0,"z":0.0} });
        if let Some(o) = owner {
            data["reception_owner"] = json!(o);
        }
        world.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: DEPOT.into(),
            object_type: "cargo_depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data,
        });
        world
    }

    #[tokio::test]
    async fn free_counter_claims_and_opens() {
        let world = world_with_owner(Some(""));
        let (mut ctx, mut rx) = ctx_for(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = DepotReceptionCoordinator::new(Some(DEPOT.into()));
        assert_eq!(a.next_step(&ctx), Some(Step::Claim));
        a.run(&mut ctx).await.unwrap();
        // An opening dialog line, then a claim_reception.
        let m1 = rx.try_recv().unwrap();
        assert_eq!(m1["data"]["action"], json!("dialog"));
        assert_eq!(m1["data"]["text"], json!(OPEN_LINE));
        let m2 = rx.try_recv().unwrap();
        assert_eq!(m2["data"]["action"], json!("claim_reception"));
        assert_eq!(m2["data"]["uuid"], json!(DEPOT));
        // Claim recorded: not eligible again while still free.
        assert_eq!(a.next_step(&ctx), None);
    }

    #[tokio::test]
    async fn owning_says_take_once() {
        let world = world_with_owner(Some(ME));
        let (mut ctx, mut rx) = ctx_for(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = DepotReceptionCoordinator::new(Some(DEPOT.into()));
        assert_eq!(a.next_step(&ctx), Some(Step::SayTake));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["text"], json!(TAKE_LINE));
        assert_eq!(a.next_step(&ctx), None);
    }

    #[tokio::test]
    async fn other_owner_waits_until_heard_then_says_stock() {
        let world = world_with_owner(Some(OTHER));
        let (mut ctx, mut rx) = ctx_for(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = DepotReceptionCoordinator::new(Some(DEPOT.into()));
        // Not heard yet -> no reply.
        assert_eq!(a.next_step(&ctx), None);
        // The owner is now heard saying something (their claim line).
        ctx.world.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: OTHER.into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "position": {"x":1.0,"y":0.0,"z":0.0}, "conversation": TAKE_LINE }),
        });
        assert_eq!(a.next_step(&ctx), Some(Step::SayStock));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["text"], json!(STOCK_LINE));
        assert_eq!(a.next_step(&ctx), None);
    }

    #[tokio::test]
    async fn leaving_work_releases_a_role_we_never_claimed() {
        // The game server granted us the counter without a claim of ours, so
        // `claim_sent` is unset — the release must happen anyway, or the
        // co-workers still on shift stay locked out of the reception.
        let world = world_with_owner(Some(ME));
        let (mut ctx, mut rx) = ctx_for(world);
        ctx.state.set_status(NpcStatus::GoingToMandatory);
        let a = DepotReceptionCoordinator::new(Some(DEPOT.into()));
        assert!(!DepotReceptionCoordinator::claim_sent(&ctx));
        assert_eq!(a.next_step(&ctx), Some(Step::Release));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("release_reception"));
        // One send per situation: no re-release while the server catches up.
        assert_eq!(a.next_step(&ctx), None);
    }

    #[tokio::test]
    async fn elected_reception_claims_over_a_stale_owner() {
        // The runtime decided the replicated owner is stale (off shift) and
        // elected us instead: claim the counter rather than go stock.
        let world = world_with_owner(Some(OTHER));
        let (mut ctx, mut rx) = ctx_for(world);
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.depot_role = DepotRole::Reception;
        let a = DepotReceptionCoordinator::new(Some(DEPOT.into()));
        assert_eq!(a.next_step(&ctx), Some(Step::Claim));
        a.run(&mut ctx).await.unwrap();
        let _open = rx.try_recv().unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("claim_reception"));
        // The observed owner is remembered, so the claim is not re-sent every
        // tick while the server still reports the stale owner.
        assert_eq!(a.next_step(&ctx), None);
    }

    #[tokio::test]
    async fn leaving_work_releases_if_claimed() {
        let world = world_with_owner(Some(ME));
        let (mut ctx, mut rx) = ctx_for(world);
        ctx.state.set_status(NpcStatus::AtWork);
        let a = DepotReceptionCoordinator::new(Some(DEPOT.into()));
        a.run(&mut ctx).await.unwrap(); // says take, marks claim_sent
        let _ = rx.try_recv();
        // Off to a meal: should release.
        ctx.state.set_status(NpcStatus::GoingToMeal);
        assert_eq!(a.next_step(&ctx), Some(Step::Release));
        a.run(&mut ctx).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("release_reception"));
        assert_eq!(a.next_step(&ctx), None);
    }
}
