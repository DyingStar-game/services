use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use rand::Rng;
use serde_json::{Value, json};
use tracing::{info, warn};

use super::{
    Action, ActionContext,
    go_to_place::{HOME_PARENT_KEY, HOME_POSITION_KEY},
    trip::TripWatch,
};
use crate::{
    error::Result, horizon::messages::Vec3, npc::spec::TakeAWalkConfig, npc::state::NpcStatus,
};

/// Radius of the disc the next point is drawn from, centred on the NPC.
pub const DEFAULT_RADIUS_M: f64 = 100.0;

/// How long the NPC stands at a reached point before setting off again.
pub const DEFAULT_PAUSE: Duration = Duration::from_secs(30);

/// Ambient strolling: draw a random point within `radius_m` of wherever the NPC
/// stands, ask the server to walk there, pause `pause` on arrival, then draw the
/// next point — indefinitely.
///
/// The choreography mirrors [`GoToPlace`](super::go_to_place::GoToPlace) — the
/// action is polled by the scheduler and yields the slot between steps rather
/// than looping inside `run`, so ambient behaviours (greeting) still run while
/// the NPC is en route:
///
/// - any status but the two below: draw a point and dispatch → `Walking`;
/// - `Walking`: on the server's `npc_arrived`, acknowledge it, start the pause
///   and settle into `WalkPause`. A trip left unanswered for
///   [`ARRIVAL_TIMEOUT`](super::trip::ARRIVAL_TIMEOUT) is dispatched again
///   (toward a freshly drawn point, since the last one may be unreachable);
/// - `WalkPause`: idle until the pause elapses, then draw the next point.
///
/// Each point is drawn around the NPC's *current* position, so a stroll is a
/// random walk that drifts. `max_distance_m` puts a leash on that drift: no
/// point is ever drawn further than that from the NPC's **spawn position** (the
/// one the runtime remembers as home), and an NPC that already stands beyond
/// the leash is walked back inside it. `None` = no limit, so the NPC may wander
/// arbitrarily far.
pub struct TakeAWalk {
    radius_m: f64,
    /// Metres from the spawn position the NPC may not stray beyond. `None` =
    /// unleashed.
    max_distance_m: Option<f64>,
    pause: Duration,
    /// Watchdog over the walk in flight — see [`TripWatch`].
    trip: TripWatch,
    /// When the pause at the point just reached ends. `None` = not pausing.
    pause_until: Mutex<Option<Instant>>,
}

impl TakeAWalk {
    /// Build from the roster config; `None` (or unset fields) keeps the code
    /// defaults.
    pub fn new(cfg: Option<TakeAWalkConfig>) -> Self {
        let radius_m = cfg
            .as_ref()
            .and_then(|c| c.radius_m)
            .filter(|r| *r > 0.0)
            .unwrap_or(DEFAULT_RADIUS_M);
        let max_distance_m = cfg
            .as_ref()
            .and_then(|c| c.max_distance_m)
            .filter(|d| *d > 0.0);
        let pause = cfg
            .as_ref()
            .and_then(|c| c.pause_s)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_PAUSE);
        Self {
            radius_m,
            max_distance_m,
            pause,
            trip: TripWatch::new(),
            pause_until: Mutex::new(None),
        }
    }

    /// Whether the pause at the last point reached is over. `true` when no pause
    /// is running, so a fresh NPC sets off at once.
    fn pause_elapsed(&self) -> bool {
        self.pause_until
            .lock()
            .unwrap()
            .is_none_or(|t| Instant::now() >= t)
    }

    /// Walk the NPC to `position` (expressed in `uuid`'s frame) and start the
    /// arrival watchdog.
    async fn dispatch(&self, ctx: &mut ActionContext, uuid: &str, position: &Value) -> Result<()> {
        // Reset any stale server-side arrival flag before walking (same hazard
        // as the other travel actions: a persisted `npc_arrived` would complete
        // the trip instantly).
        ctx.send
            .send("player", "client_action", arrived_ack_data(uuid))
            .await?;
        ctx.world.clear_arrived();
        ctx.send
            .send("player", "client_action", go_to_position_data(uuid, position))
            .await?;
        self.trip.dispatched();
        *self.pause_until.lock().unwrap() = None;
        ctx.state.set_status(NpcStatus::Walking);
        Ok(())
    }

    /// The next point to walk to: drawn around `here`, then held to
    /// `max_distance_m` of `anchor` (the spawn position) when both are set.
    ///
    /// Rejection-samples a few draws so the point keeps a natural spread, and
    /// falls back to pulling the last candidate back onto the boundary circle —
    /// which also walks an NPC that somehow ended up outside the leash back in.
    fn draw_point<R: Rng>(&self, here: Vec3, anchor: Option<Vec3>, rng: &mut R) -> Vec3 {
        /// Draws before giving up on landing inside the leash by chance.
        const MAX_TRIES: usize = 8;

        let (Some(max), Some(anchor)) = (self.max_distance_m, anchor) else {
            return random_point_around(here, self.radius_m, rng);
        };
        let mut candidate = random_point_around(here, self.radius_m, rng);
        for _ in 0..MAX_TRIES {
            if flat_distance(candidate, anchor) <= max {
                return candidate;
            }
            candidate = random_point_around(here, self.radius_m, rng);
        }
        clamp_to(candidate, anchor, max)
    }

    /// Backdate the pause deadline so a test can reach it without sleeping.
    #[cfg(test)]
    fn age_pause_by(&self, age: Duration) {
        let mut slot = self.pause_until.lock().unwrap();
        if let Some(t) = *slot {
            *slot = Some(t - age);
        }
    }
}

#[async_trait]
impl Action for TakeAWalk {
    fn name(&self) -> &'static str {
        "take_a_walk"
    }

    fn priority(&self) -> i32 {
        // Ambient roaming: below greeting (100) and every scheduled behaviour,
        // above plain `go_to_point` (10).
        20
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        match ctx.state.status() {
            // En route: finalise once the server says we've arrived — or
            // re-dispatch once the trip is presumed lost.
            NpcStatus::Walking => ctx.world.npc_arrived() || self.trip.timed_out(),
            // Standing at the point just reached: wait out the pause. Points are
            // drawn around the NPC, so its position must be known first.
            NpcStatus::WalkPause => self.pause_elapsed() && ctx.world.my_position().is_some(),
            _ => ctx.world.my_position().is_some(),
        }
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        if ctx.state.status() == NpcStatus::Walking {
            if ctx.world.npc_arrived() {
                let uuid = ctx.world.my_parent().unwrap_or_default();
                info!(npc = %ctx.npc_name, pause_s = self.pause.as_secs(), "walk point reached; pausing");
                ctx.send
                    .send("player", "client_action", arrived_ack_data(&uuid))
                    .await?;
                ctx.world.clear_arrived();
                self.trip.arrived();
                *self.pause_until.lock().unwrap() = Some(Instant::now() + self.pause);
                ctx.state.set_status(NpcStatus::WalkPause);
                return Ok(());
            }
            warn!(
                npc = %ctx.npc_name,
                waiting_s = self.trip.waiting_s(),
                my_parent = ?ctx.world.my_parent(),
                "no arrival notice; strolling to another point"
            );
            // Fall through: draw a fresh point rather than insisting on one the
            // server may not be able to path to.
        }

        // Points are drawn around where the NPC stands, in its own frame, so
        // the walk order is expressed in that same frame (its parent, empty =
        // root) — exactly like the position Horizon echoes back.
        let Some(here) = ctx.world.my_position() else {
            return Ok(());
        };
        // Where the NPC spawned, in the frame it is walking in — the centre of
        // the leash. `None` when there is no leash to enforce, or when the two
        // frames cannot be reconciled (warned about in `spawn_anchor`).
        let mut anchor = None;
        if self.max_distance_m.is_some() {
            anchor = spawn_anchor(ctx);
            if anchor.is_none() {
                warn!(
                    npc = %ctx.npc_name,
                    "spawn position unknown in the current frame; walking without a distance limit"
                );
            }
        }
        let target = self.draw_point(here, anchor, &mut ctx.rng);
        let uuid = ctx.world.my_parent().unwrap_or_default();
        info!(
            npc = %ctx.npc_name,
            x = target.x, y = target.y, z = target.z,
            radius_m = self.radius_m,
            max_distance_m = ?self.max_distance_m,
            "taking a walk to a random point"
        );
        self.dispatch(ctx, &uuid, &json!(target)).await
    }
}

/// A point drawn uniformly from the horizontal disc of radius `radius_m`
/// centred on `center`. The altitude is left untouched: the NPC walks the
/// ground it already stands on.
fn random_point_around<R: Rng>(center: Vec3, radius_m: f64, rng: &mut R) -> Vec3 {
    let theta = rng.gen_range(0.0..std::f64::consts::TAU);
    // sqrt of a uniform draw, or points would bunch up around the centre.
    let r = radius_m * rng.gen_range(0.0f64..1.0).sqrt();
    Vec3 {
        x: center.x + r * theta.cos(),
        y: center.y,
        z: center.z + r * theta.sin(),
    }
}

/// Horizontal distance between two points; the leash ignores altitude, like the
/// draw itself.
fn flat_distance(a: Vec3, b: Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// `p` pulled back onto the circle of radius `max` around `anchor`, keeping its
/// bearing. Returns `anchor` itself when `p` sits exactly on it.
fn clamp_to(p: Vec3, anchor: Vec3, max: f64) -> Vec3 {
    let d = flat_distance(p, anchor);
    if d <= max || d == 0.0 {
        return p;
    }
    let k = max / d;
    Vec3 {
        x: anchor.x + (p.x - anchor.x) * k,
        y: p.y,
        z: anchor.z + (p.z - anchor.z) * k,
    }
}

/// The NPC's spawn position — remembered by the runtime under
/// [`HOME_POSITION_KEY`] — expressed in the frame the NPC is currently walking
/// in, so the two are comparable.
///
/// Usually both are the same frame and the stored value is returned as is. When
/// the NPC has since been re-parented, the positions are reconciled through the
/// root frame; `None` when that chain is not fully in view, since a partial sum
/// would silently leash the NPC to the wrong spot.
fn spawn_anchor(ctx: &ActionContext) -> Option<Vec3> {
    let stored = ctx.state.get(HOME_POSITION_KEY)?;
    let spawn: Vec3 = serde_json::from_value(stored).ok()?;
    let spawn_parent = ctx
        .state
        .get(HOME_PARENT_KEY)
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let here_parent = ctx.world.my_parent().unwrap_or_default();
    if spawn_parent == here_parent {
        return Some(spawn);
    }
    // Re-parented since spawn: go through the root frame and back down into the
    // current one (its origin = the NPC's global position minus its local one).
    let here_local = ctx.world.my_position()?;
    let here_global = ctx.world.resolve_global(here_local, Some(&here_parent))?;
    let spawn_global = ctx.world.resolve_global(spawn, Some(&spawn_parent))?;
    Some(Vec3 {
        x: spawn_global.x - here_global.x + here_local.x,
        y: spawn_global.y - here_global.y + here_local.y,
        z: spawn_global.z - here_global.z + here_local.z,
    })
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

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind},
        npc::state::NpcState,
        world::view::WorldView,
    };

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

    /// Feed a frame about self, the way Horizon echoes the NPC's own transform.
    fn self_frame(ctx: &ActionContext, data: Value) {
        ctx.world.apply(&GorcFrame {
            kind: GorcKind::Update,
            channel: 0,
            object_id: "self".into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data,
        });
    }

    fn stand_at(ctx: &ActionContext, x: f64, y: f64, z: f64, parent: &str) {
        self_frame(
            ctx,
            json!({ "position": {"x": x, "y": y, "z": z}, "parent_id": parent }),
        );
    }

    fn mark_arrived(ctx: &ActionContext) {
        self_frame(ctx, json!({ "npc_arrived": true }));
    }

    fn walk() -> TakeAWalk {
        TakeAWalk::new(None)
    }

    #[test]
    fn stands_down_until_its_own_position_is_known() {
        let (c, _rx) = ctx();
        assert!(
            !walk().can_run(&c),
            "no reference point => no point to draw around"
        );
        stand_at(&c, 10.0, 0.0, 20.0, "depot");
        assert!(walk().can_run(&c));
    }

    #[test]
    fn drawn_points_stay_inside_the_radius() {
        let mut rng = StdRng::seed_from_u64(7);
        let center = Vec3 { x: 100.0, y: 3.5, z: -50.0 };
        for _ in 0..1_000 {
            let p = random_point_around(center, DEFAULT_RADIUS_M, &mut rng);
            let d = ((p.x - center.x).powi(2) + (p.z - center.z).powi(2)).sqrt();
            assert!(d <= DEFAULT_RADIUS_M, "drew a point {d} m away");
            assert_eq!(p.y, center.y, "altitude must be left alone");
        }
    }

    #[tokio::test]
    async fn walks_pauses_then_walks_again() {
        let (mut c, mut rx) = ctx();
        stand_at(&c, 10.0, 0.0, 20.0, "depot");
        let a = walk();

        // First hop: stale arrival reset, then the walk order, in our own frame.
        a.run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        let order = rx.try_recv().unwrap();
        assert_eq!(order["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(order["data"]["uuid"], json!("depot"));
        let (x, z) = (
            order["data"]["position"]["x"].as_f64().unwrap(),
            order["data"]["position"]["z"].as_f64().unwrap(),
        );
        assert!(((x - 10.0).powi(2) + (z - 20.0).powi(2)).sqrt() <= DEFAULT_RADIUS_M);
        assert_eq!(c.state.status(), NpcStatus::Walking);

        // En route: yields the slot.
        assert!(!a.can_run(&c));

        // Arrival: ack, then stand still for the pause.
        mark_arrived(&c);
        assert!(a.can_run(&c));
        a.run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(c.state.status(), NpcStatus::WalkPause);
        assert!(!c.world.npc_arrived());
        assert!(!a.can_run(&c), "must not set off again during the pause");

        // Pause over: off to the next point.
        a.age_pause_by(DEFAULT_PAUSE);
        assert!(a.can_run(&c));
        a.run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(c.state.status(), NpcStatus::Walking);
    }

    #[tokio::test]
    async fn an_unanswered_walk_is_set_off_again() {
        let (mut c, mut rx) = ctx();
        stand_at(&c, 0.0, 0.0, 0.0, "");
        let a = walk();

        a.run(&mut c).await.unwrap();
        while rx.try_recv().is_ok() {}
        assert!(!a.can_run(&c));

        // The server never answered: draw another point instead of standing
        // still for the rest of the window.
        a.trip.age_by(crate::actions::trip::ARRIVAL_TIMEOUT);
        assert!(a.can_run(&c));
        a.run(&mut c).await.unwrap();
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        let order = rx.try_recv().unwrap();
        assert_eq!(order["data"]["action"], json!("npc_go_to_position"));
        // No parent reported => the root frame.
        assert_eq!(order["data"]["uuid"], json!(""));
        assert_eq!(c.state.status(), NpcStatus::Walking);
    }

    /// Remember a spawn position the way the runtime does on the first tick.
    fn spawned_at(ctx: &ActionContext, x: f64, y: f64, z: f64, parent: &str) {
        ctx.state
            .set(HOME_POSITION_KEY, json!({ "x": x, "y": y, "z": z }));
        ctx.state.set(HOME_PARENT_KEY, json!(parent));
    }

    fn leashed(max_distance_m: f64) -> TakeAWalk {
        TakeAWalk::new(Some(TakeAWalkConfig {
            radius_m: None,
            pause_s: None,
            max_distance_m: Some(max_distance_m),
        }))
    }

    #[test]
    fn every_drawn_point_stays_within_the_leash() {
        let mut rng = StdRng::seed_from_u64(11);
        let a = leashed(150.0);
        let spawn = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        // Walk from the far edge of the leash: most raw draws land outside, so
        // both the rejection sampling and the clamp fallback are exercised.
        let here = Vec3 { x: 150.0, y: 0.0, z: 0.0 };
        for _ in 0..1_000 {
            let p = a.draw_point(here, Some(spawn), &mut rng);
            assert!(
                flat_distance(p, spawn) <= 150.0 + 1e-9,
                "drew a point {} m from spawn",
                flat_distance(p, spawn)
            );
        }
    }

    #[test]
    fn an_npc_outside_the_leash_is_walked_back_in() {
        let mut rng = StdRng::seed_from_u64(3);
        let a = leashed(50.0);
        let spawn = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        // 400 m out with a 100 m stride: no draw can land inside, so the clamp
        // must pull the point back onto the boundary rather than let it drift.
        let here = Vec3 { x: 400.0, y: 0.0, z: 0.0 };
        let p = a.draw_point(here, Some(spawn), &mut rng);
        assert!((flat_distance(p, spawn) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn no_leash_configured_leaves_the_draw_alone() {
        let mut rng = StdRng::seed_from_u64(5);
        let spawn = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let here = Vec3 { x: 1_000.0, y: 0.0, z: 0.0 };
        let p = walk().draw_point(here, Some(spawn), &mut rng);
        assert!(flat_distance(p, here) <= DEFAULT_RADIUS_M);
        assert!(flat_distance(p, spawn) > 800.0, "unleashed: no pull home");
    }

    #[tokio::test]
    async fn the_leash_is_centred_on_the_spawn_position() {
        let (mut c, mut rx) = ctx();
        spawned_at(&c, 0.0, 0.0, 0.0, "depot");
        // Already 90 m from spawn with a 100 m stride and a 100 m leash: the
        // next point must come back toward spawn, not carry on drifting.
        stand_at(&c, 90.0, 0.0, 0.0, "depot");
        leashed(100.0).run(&mut c).await.unwrap();
        rx.try_recv().unwrap();
        let pos = &rx.try_recv().unwrap()["data"]["position"];
        let (x, z) = (
            pos["x"].as_f64().unwrap(),
            pos["z"].as_f64().unwrap(),
        );
        assert!((x * x + z * z).sqrt() <= 100.0 + 1e-9);
    }

    #[test]
    fn spawn_anchor_is_re_expressed_after_a_re_parent() {
        let (c, _rx) = ctx();
        // Spawn frame and current frame are siblings under a known root: the
        // anchor must be converted, not compared raw.
        c.world.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: "hab".into(),
            object_type: "building".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "parent_id": "", "position": {"x": 1000.0, "y": 0.0, "z": 0.0} }),
        });
        c.world.apply(&GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: "depot".into(),
            object_type: "building".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "parent_id": "", "position": {"x": 1100.0, "y": 0.0, "z": 0.0} }),
        });
        spawned_at(&c, 5.0, 0.0, 0.0, "hab"); // = 1005 in the root frame
        stand_at(&c, 0.0, 0.0, 0.0, "depot"); // depot origin = 1100
        let anchor = spawn_anchor(&c).expect("both frames are in view");
        assert!((anchor.x - (-95.0)).abs() < 1e-9, "got {}", anchor.x);
    }

    #[test]
    fn an_unresolvable_spawn_frame_yields_no_anchor() {
        let (c, _rx) = ctx();
        // Spawned under a frame that never replicated into view: reconciling
        // would be a guess, so there is no anchor (and no leash) rather than a
        // plausible wrong one.
        spawned_at(&c, 5.0, 0.0, 0.0, "hab-never-seen");
        stand_at(&c, 0.0, 0.0, 0.0, "");
        assert!(spawn_anchor(&c).is_none());
    }

    #[tokio::test]
    async fn roster_config_overrides_radius_and_pause() {
        let (mut c, mut rx) = ctx();
        stand_at(&c, 0.0, 0.0, 0.0, "");
        let a = TakeAWalk::new(Some(TakeAWalkConfig {
            radius_m: Some(5.0),
            pause_s: Some(1),
            max_distance_m: None,
        }));
        assert_eq!(a.pause, Duration::from_secs(1));

        a.run(&mut c).await.unwrap();
        rx.try_recv().unwrap();
        let order = rx.try_recv().unwrap();
        let (x, z) = (
            order["data"]["position"]["x"].as_f64().unwrap(),
            order["data"]["position"]["z"].as_f64().unwrap(),
        );
        assert!((x * x + z * z).sqrt() <= 5.0);
    }
}
