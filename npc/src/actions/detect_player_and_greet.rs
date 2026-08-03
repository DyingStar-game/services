use std::{
    sync::{Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use tracing::{debug, warn};

use super::{
    Action, ActionContext, ActionTiming, conversations::Conversation, conversations::greet::Greet,
};
use crate::error::Result;

/// Players further than this from the NPC are never greeted.
const MAX_GREET_DISTANCE: f64 = 5.0;

/// A single greeting must not hold the NPC's only action slot indefinitely.
const GREET_TIMEOUT: Duration = Duration::from_secs(1200);

/// Minimum gap between the start of one greet check and the next. Throttles
/// this action to ~2 Hz even though the scheduler ticks faster.
const GREET_INTERVAL: Duration = Duration::from_millis(500);

/// When at least one player is within [`MAX_GREET_DISTANCE`] of the NPC, pick
/// the nearest one and run a greeting conversation. Cooldown prevents spamming
/// the same player.
pub struct DetectPlayerAndGreet {
    cooldown: Duration,
    last_greet: Mutex<std::collections::HashMap<String, Instant>>,
    /// Set once the "no self position" warning has been emitted, so the tick
    /// loop cannot spam it every 200ms.
    warned_no_self_position: AtomicBool,
}

impl Default for DetectPlayerAndGreet {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(60),
            last_greet: Mutex::new(std::collections::HashMap::new()),
            warned_no_self_position: AtomicBool::new(false),
        }
    }
}

impl DetectPlayerAndGreet {
    /// Nearest player that is both within [`MAX_GREET_DISTANCE`] of the NPC and
    /// past its greet cooldown.
    ///
    /// Without a self position there is no reference point for the distance
    /// check, so this greets nobody rather than measuring from the origin.
    fn nearest_greetable(&self, ctx: &ActionContext) -> Option<String> {
        let Some(here) = ctx.world.my_global_position() else {
            if !self.warned_no_self_position.swap(true, Ordering::Relaxed) {
                warn!(
                    npc = %ctx.npc_name,
                    my_uuid = %ctx.world.my_uuid,
                    my_parent = ?ctx.world.my_parent(),
                    "own position not resolvable to the root frame; cannot greet anyone"
                );
            }
            return None;
        };
        let max_d2 = MAX_GREET_DISTANCE * MAX_GREET_DISTANCE;
        let map = self.last_greet.lock().unwrap();
        let now = Instant::now();

        let mut best: Option<(String, f64)> = None;
        for p in ctx.world.players.iter() {
            let on_cd = map
                .get(p.key())
                .map(|t| now.duration_since(*t) < self.cooldown)
                .unwrap_or(false);
            if on_cd {
                continue;
            }
            // Positions are local to each entity's parent, so both sides must
            // be lifted into the root frame before they can be subtracted.
            let Some(there) = ctx
                .world
                .resolve_global(p.position, p.parent_id.as_deref())
            else {
                debug!(
                    npc = %ctx.npc_name,
                    player = %p.key(),
                    player_parent = ?p.parent_id,
                    "player position not resolvable to the root frame; skipping"
                );
                continue;
            };
            let dx = there.x - here.x;
            let dy = there.y - here.y;
            let dz = there.z - here.z;
            let d2 = dx * dx + dy * dy + dz * dz;
            debug!(
                npc = %ctx.npc_name,
                player = %p.key(),
                distance = %d2.sqrt(),
                "considering for greeting"
            );
            if d2 > max_d2 {
                continue;
            }
            if best.as_ref().is_none_or(|(_, bd)| d2 < *bd) {
                best = Some((p.key().clone(), d2));
            }
        }
        best.map(|(uuid, _)| uuid)
    }
}

#[async_trait]
impl Action for DetectPlayerAndGreet {
    fn name(&self) -> &'static str {
        "detect_player_and_greet"
    }

    fn priority(&self) -> i32 {
        100
    }

    fn timing(&self) -> ActionTiming {
        // Poll at ~2 Hz (500ms) rather than on every 200ms scheduler tick.
        // The 60s greet throttle is *per player* (`last_greet` below), which
        // the scheduler's per-action `cooldown` cannot express, so that stays
        // hand-rolled.
        ActionTiming {
            min_interval: Some(GREET_INTERVAL),
            timeout: Some(GREET_TIMEOUT),
            ..Default::default()
        }
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        self.nearest_greetable(ctx).is_some()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        let target = self.nearest_greetable(ctx);
        if let Some(uuid) = target {
            // Clone the name out before awaiting: holding a DashMap guard
            // across an await point can deadlock the world view.
            let name = ctx.world.players.get(&uuid).and_then(|p| p.name.clone());
            Greet.run(ctx, &uuid, name.as_deref()).await?;
            self.last_greet
                .lock()
                .unwrap()
                .insert(uuid, Instant::now());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::{SeedableRng, rngs::StdRng};

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind},
        world::view::WorldView,
    };

    fn dummy_ctx(world: Arc<WorldView>) -> ActionContext {
        // Create a sender backed by an mpsc whose receiver we drop. Sends will
        // succeed until the buffer fills (capacity 64 in the real client; we
        // make our own here).
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        ActionContext {
            npc_name: "Test".into(),
            world,
            state: Arc::new(crate::npc::state::NpcState::new()),
            send: HorizonSender::__test_new(tx),
            rng: StdRng::seed_from_u64(0),
        }
    }

    /// Place an entity in the world at `pos`. `object_type` decides whether it
    /// lands in `players` or `objects`.
    fn place(world: &WorldView, id: &str, object_type: &str, pos: (f64, f64, f64)) {
        world.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: id.into(),
            object_type: object_type.into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: serde_json::json!({"position":{"x":pos.0,"y":pos.1,"z":pos.2}}),
        });
    }

    /// World where the NPC itself sits at `pos`, far from the origin so that a
    /// distance accidentally measured from `(0,0,0)` cannot pass the gate.
    /// Horizon echoes the NPC back as a player-type frame about self, which is
    /// the only way its own position ever reaches the view.
    fn world_with_npc_at(pos: (f64, f64, f64)) -> Arc<WorldView> {
        let world = Arc::new(WorldView::new("self".into()));
        place(&world, "self", "player", pos);
        world
    }

    #[test]
    fn cannot_run_when_no_players() {
        let ctx = dummy_ctx(world_with_npc_at((1000.0, 0.0, 1000.0)));
        let a = DetectPlayerAndGreet::default();
        assert!(!a.can_run(&ctx));
    }

    #[test]
    fn can_run_when_player_within_range() {
        let world = world_with_npc_at((1000.0, 0.0, 1000.0));
        // 3m away from the NPC, ~1414m from the origin.
        place(&world, "p1", "player", (1003.0, 0.0, 1000.0));
        let ctx = dummy_ctx(world);
        let a = DetectPlayerAndGreet::default();
        assert!(a.can_run(&ctx));
    }

    #[test]
    fn cannot_run_when_player_beyond_range() {
        let world = world_with_npc_at((1000.0, 0.0, 1000.0));
        // 6m away: inside the GORC zone but outside greeting range.
        place(&world, "p1", "player", (1006.0, 0.0, 1000.0));
        let ctx = dummy_ctx(world);
        let a = DetectPlayerAndGreet::default();
        assert!(!a.can_run(&ctx));
    }

    #[test]
    fn cannot_run_when_self_position_unknown() {
        // No frame about self, so the NPC has no reference point: fail closed
        // rather than measuring from the origin.
        let world = Arc::new(WorldView::new("self".into()));
        place(&world, "p1", "player", (1.0, 0.0, 0.0));
        let ctx = dummy_ctx(world);
        let a = DetectPlayerAndGreet::default();
        assert!(!a.can_run(&ctx));
    }

    #[test]
    fn picks_nearest_player_relative_to_npc_not_origin() {
        let world = world_with_npc_at((1000.0, 0.0, 1000.0));
        // p_near is nearer the NPC; p_far is nearer the world origin. The old
        // origin-based math would have picked p_far.
        place(&world, "p_far", "player", (996.0, 0.0, 1000.0));
        place(&world, "p_near", "player", (1001.0, 0.0, 1000.0));
        let ctx = dummy_ctx(world);
        let a = DetectPlayerAndGreet::default();
        assert_eq!(a.nearest_greetable(&ctx).as_deref(), Some("p_near"));
    }
}
