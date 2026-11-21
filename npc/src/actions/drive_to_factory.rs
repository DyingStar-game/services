//! `drive_to_factory`: an NPC living in a Mining village walks to its garage,
//! presses the kiosk for a truck, opens the driver's door, boards, drives BY
//! THE ROAD to its Factory village, parks and gets out.
//!
//! Everything is decided here; the game server only does what it does for a
//! player pressing the same buttons (`kiosk_spawn`, `vehicle_door`,
//! `enter_vehicle`, `vehicle_input`, …) and validates them the same way
//! (range, line of sight, open door, pilot).
//!
//! Inputs:
//! - [`WorldData`]: villages + road graph (QGIS exports) → origin, destination
//!   and the route;
//! - [`SceneLayouts`]: where the kiosk, the spawn spot, the driver's door and
//!   seat are inside the garage / truck scenes (Godot export);
//! - the replicated world: the garage object, the spawned truck and its live
//!   state (position, rotation, speed, steering, doors, seats).
//!
//! The trip runs as one long `run` (like the depot actions), its phase kept on
//! the blackboard under [`STATE_KEY`] for `GET /npcs/world`. A failure before
//! boarding just ends the run (retried after the cooldown); once aboard, any
//! failure parks the truck where it is and gets the NPC out.

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{Value, json};
use tokio::time;
use tracing::{debug, info, warn};

use super::{
    Action, ActionContext, ActionTiming,
    driving::{
        self, VehicleState,
        controller::{Command, Controller, Observation, RoutePt, Step, Tuning, VehicleParams},
        obstacles::{self, Obstacle, Tracker},
        offroad,
        traffic::{self, BypassRequest, Me, Mode, Seen, Traffic},
    },
};
use crate::{
    error::Result,
    horizon::messages::Vec3,
    npc::{spec::DriveToFactoryConfig, state::NpcStatus},
    scene_layouts::{self, DoorLayout, GARAGE_SCENE, KioskLayout, SceneLayout, SceneLayouts},
    world_data::{WorldData, geo, poi::Village},
};

/// Scene of the truck the garage kiosk hands out.
pub const TRUCK_SCENE: &str = "scenes/_universe/vehicles/ground/trucks/truck.tscn";

/// Blackboard key holding the trip's progress (phase, villages, truck, metres).
pub const STATE_KEY: &str = "drive_to_factory";
/// Blackboard flag set once no further trip is due this session: `true` after
/// a completed trip, or a reason string when none can be made from here.
pub const DONE_KEY: &str = "drive_to_factory.done";

/// Marker (in the garage scene) where an arriving NPC parks. Without it, the
/// NPC parks [`GARAGE_EXIT_M`] in front of the garage's truck spawn spot.
const PARKING_MARKER: &str = "NpcParking";
/// The truck spawns facing the garage door: it first drives this far straight
/// ahead, out of the building, before the off-road planner takes over (the
/// door is barely wider than the truck, too tight for a grid). Also how far in
/// front of the destination garage it parks.
const GARAGE_EXIT_M: f64 = 15.0;
/// Village objects (`poi_village`, replicated up to 10 km) searched this far
/// around the truck / the destination's centre.
const VILLAGE_SEARCH_M: f64 = 3_000.0;
/// Off-road planner: clearance kept around buildings, grid cell.
const OFFROAD_CLEARANCE_M: f64 = 2.0;
const OFFROAD_CELL_M: f64 = 1.0;
/// How often to look again for the destination village while it is not in
/// view yet.
const DEST_RETRY: Duration = Duration::from_secs(2);

/// How far from the NPC its village garage may be.
const GARAGE_SEARCH_M: f64 = 1_500.0;
/// A truck this close to the kiosk's spawn spot is the one it handed out.
const SPAWN_MATCH_M: f64 = 6.0;
/// Walking legs (to the kiosk, to the door).
const WALK_TIMEOUT: Duration = Duration::from_secs(120);
/// Close enough to a walk target when the server never says `npc_arrived`.
const WALK_ARRIVED_M: f64 = 1.2;
/// How long the truck may take to appear after a press.
const SPAWN_WAIT: Duration = Duration::from_secs(10);
/// Kiosk presses before giving up.
const KIOSK_ATTEMPTS: u32 = 3;
/// Retries of each door / seat request.
const REQUEST_ATTEMPTS: u32 = 3;
/// How long to wait for a door, seat or engine change to replicate back.
const CONFIRM_WAIT: Duration = Duration::from_secs(5);
/// The NPC has driven this far from the garage: free the kiosk for others.
const RELEASE_GARAGE_AFTER_M: f64 = 30.0;
/// Vehicle resets tolerated on one trip before parking where it stands.
const MAX_RESETS: u32 = 3;
/// The truck may vanish from view this long before the trip is abandoned.
const TRUCK_LOST: Duration = Duration::from_secs(5);
/// Wheel turn rate at steer ±1 (`Vehicle.steer_speed`).
/// Wheel turn rates when the truck layout does not say (`truck.tscn`'s
/// `steer_speed` / `steer_return_speed`).
const TURN_SPEED_RAD_S: f64 = 0.2;
const RETURN_SPEED_RAD_S: f64 = 1.0;
/// How far ahead along the route the driver looks for traffic and obstacles.
const LOOK_AHEAD_M: f64 = 200.0;
/// How long one horn blast lasts (the game's horn sounds while held).
const HONK_HOLD: Duration = Duration::from_secs(1);
/// A stuck truck with something this close ahead is blocked, not stuck: no
/// reset (it would do nothing to an upright truck anyway).
const BLOCKED_AHEAD_M: f64 = 6.0;
/// Resend unchanged pilot inputs at least this often.
const INPUT_REFRESH: Duration = Duration::from_secs(1);

/// Control period of the driving loop (`DRIVE_TICK_MS`, default 100 ms).
fn drive_tick() -> Duration {
    static TICK: OnceLock<Duration> = OnceLock::new();
    *TICK.get_or_init(|| {
        Duration::from_millis(
            std::env::var("DRIVE_TICK_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &u64| *v >= 20)
                .unwrap_or(100),
        )
    })
}

/// One NPC at a time per garage kiosk: its cooldown and single spawn box would
/// otherwise make co-workers steal or block each other's truck. All NPCs run
/// in this process, so an in-process lock is enough.
fn garage_locks() -> &'static DashMap<String, String> {
    static LOCKS: OnceLock<DashMap<String, String>> = OnceLock::new();
    LOCKS.get_or_init(DashMap::new)
}

/// Held kiosk claim; released on drop (end of run, abort, timeout).
struct GarageLock {
    garage: String,
    npc: String,
}

impl GarageLock {
    fn try_take(garage: &str, npc: &str) -> Option<Self> {
        let locks = garage_locks();
        match locks.entry(garage.to_string()) {
            dashmap::Entry::Occupied(e) if e.get() != npc => return None,
            dashmap::Entry::Occupied(_) => {}
            dashmap::Entry::Vacant(v) => {
                v.insert(npc.to_string());
            }
        }
        Some(Self { garage: garage.to_string(), npc: npc.to_string() })
    }
}

impl Drop for GarageLock {
    fn drop(&mut self) {
        garage_locks().remove_if(&self.garage, |_, v| *v == self.npc);
    }
}

/// Everything resolved before the trip starts.
struct Plan {
    wd: std::sync::Arc<WorldData>,
    radius: f64,
    origin: Village,
    dest: Village,
    /// Road point nearest the Factory village: where the trip ends unless a
    /// parking spot is found there.
    dest_access: Vec3,
    garage: String,
    kiosk: KioskLayout,
    truck_layout: SceneLayout,
    door: DoorLayout,
    seat_key: String,
    params: VehicleParams,
    max_speed_kmh: f64,
}

pub struct DriveToFactory {
    cfg: DriveToFactoryConfig,
}

impl DriveToFactory {
    pub fn new(cfg: Option<DriveToFactoryConfig>) -> Self {
        Self { cfg: cfg.unwrap_or_default() }
    }

    fn set_phase(ctx: &ActionContext, phase: &str, extra: Value) {
        let mut v = ctx.state.get(STATE_KEY).unwrap_or_else(|| json!({}));
        if let Some(o) = v.as_object_mut() {
            o.insert("phase".into(), json!(phase));
            if let Some(e) = extra.as_object() {
                for (k, x) in e {
                    o.insert(k.clone(), x.clone());
                }
            }
        }
        ctx.state.set(STATE_KEY, v);
    }

    /// Resolve villages, route feasibility, garage and layouts. `None` (logged)
    /// when the trip cannot be made from here.
    fn plan(&self, ctx: &ActionContext) -> Option<Plan> {
        let npc = &ctx.npc_name;
        let wd = WorldData::shared()?;
        let layouts = SceneLayouts::shared();
        let radius = wd.radius_m();
        let (Some(garage_l), Some(truck_l)) = (layouts.get(GARAGE_SCENE), layouts.get(TRUCK_SCENE)) else {
            warn!(npc = %npc, "garage or truck scene layout missing; cannot drive");
            return None;
        };
        let kiosk = garage_l.kiosks.iter().find(|k| k.spawn.is_some())?.clone();
        let Some(seat) = truck_l.driver_seat() else {
            warn!(npc = %npc, "truck layout has no driver seat");
            return None;
        };
        let Some(door) = truck_l.door(&seat.door_id).cloned() else {
            warn!(npc = %npc, door = %seat.door_id, "truck layout lacks the driver's door");
            return None;
        };
        let Some(veh) = truck_l.vehicle.clone() else {
            warn!(npc = %npc, "truck layout has no vehicle geometry");
            return None;
        };

        let me = driving::my_planet_position(&ctx.world, radius)?;
        // The three checks below cannot change during a session (the NPC does
        // not move between villages unless it drives): give up for the session
        // instead of retrying — and warning — every cooldown.
        let Some(origin) = wd.home_mining_village(me).cloned() else {
            let (lon, lat) = geo::unit_to_lonlat(me);
            info!(npc = %npc, lon, lat, "not in a Mining village; no trip to the Factory this session");
            ctx.state.set(DONE_KEY, json!("not_in_mining_village"));
            return None;
        };
        let Some(dest) = wd.factory_destination(&origin.name, self.cfg.destination.as_deref()).cloned() else {
            warn!(npc = %npc, origin = %origin.name, "no Factory village reachable by road from here");
            ctx.state.set(DONE_KEY, json!("no_factory"));
            return None;
        };
        let access = wd.graph.locate(dest.unit)?;
        let dest_access = geo::scale(geo::lonlat_to_unit(access.lonlat.0, access.lonlat.1), radius);
        if wd.route(me, dest_access).is_none() {
            warn!(npc = %npc, origin = %origin.name, dest = %dest.name, "no road between the villages");
            ctx.state.set(DONE_KEY, json!("no_road"));
            return None;
        }

        let garage = nearest_object(ctx, GARAGE_SCENE, me, radius, GARAGE_SEARCH_M)?;
        Some(Plan {
            radius,
            origin,
            dest,
            dest_access,
            garage,
            kiosk,
            door,
            seat_key: seat.key.clone(),
            params: VehicleParams {
                wheelbase_m: veh.wheelbase_m,
                max_steer_rad: veh.max_steer_deg.to_radians(),
                steer_min_ratio: veh.steer_min_ratio,
                steer_falloff_kmh: veh.steer_speed_falloff_kmh,
                turn_speed_rad_s: veh.steer_speed.unwrap_or(TURN_SPEED_RAD_S),
                return_speed_rad_s: veh.steer_return_speed.unwrap_or(RETURN_SPEED_RAD_S),
            },
            max_speed_kmh: self.cfg.max_speed_kmh.unwrap_or(wd.config.max_speed_kmh),
            truck_layout: truck_l.clone(),
            wd,
        })
    }
}

#[async_trait]
impl Action for DriveToFactory {
    fn name(&self) -> &'static str {
        "drive_to_factory"
    }

    fn priority(&self) -> i32 {
        // A deliberate trip: above ambient greeting (100) and the schedule's
        // go-to-place behaviours (120), below going_to_work (200).
        130
    }

    fn timing(&self) -> ActionTiming {
        ActionTiming {
            // A trip that could not start (no garage in view yet, kiosk taken…)
            // is tried again a minute later.
            cooldown: Some(Duration::from_secs(60)),
            timeout: Some(Duration::from_secs(45 * 60)),
            ..Default::default()
        }
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        ctx.state.get(DONE_KEY).is_none()
            && ctx.world.my_position().is_some()
            && WorldData::shared().is_some()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        let Some(plan) = self.plan(ctx) else {
            return Ok(());
        };
        let npc = ctx.npc_name.clone();
        let Some(lock) = GarageLock::try_take(&plan.garage, &npc) else {
            debug!(npc = %npc, garage = %plan.garage, "garage kiosk busy; later");
            return Ok(());
        };
        info!(
            npc = %npc,
            origin = %plan.origin.name,
            dest = %plan.dest.name,
            garage = %plan.garage,
            "driving to the Factory village"
        );
        ctx.state.set(
            STATE_KEY,
            json!({"origin": plan.origin.name, "destination": plan.dest.name, "garage": plan.garage}),
        );
        ctx.state.set_status(NpcStatus::Driving);

        let outcome = trip(ctx, &plan, lock).await;
        match outcome {
            Ok(true) => {
                info!(npc = %npc, dest = %plan.dest.name, "arrived at the Factory village");
                Self::set_phase(ctx, "done", json!({}));
                ctx.state.set(DONE_KEY, json!(true));
            }
            Ok(false) => {
                Self::set_phase(ctx, "failed", json!({}));
            }
            Err(e) => {
                Self::set_phase(ctx, "failed", json!({"error": e.to_string()}));
                ctx.state.set_status(NpcStatus::Idle);
                return Err(e);
            }
        }
        ctx.state.set_status(NpcStatus::Idle);
        Ok(())
    }
}

/// The whole trip. `Ok(true)` = arrived and out of the truck; `Ok(false)` =
/// gave up (logged).
async fn trip(ctx: &mut ActionContext, plan: &Plan, lock: GarageLock) -> Result<bool> {
    let npc = ctx.npc_name.clone();

    // ── 1. A truck at the garage ──
    DriveToFactory::set_phase(ctx, "get_truck", json!({}));
    let Some(truck) = get_truck(ctx, plan).await? else {
        warn!(npc = %npc, garage = %plan.garage, "no truck from the kiosk; giving up for now");
        return Ok(false);
    };
    DriveToFactory::set_phase(ctx, "walk_to_door", json!({"truck": truck}));

    // ── 2. Walk to the driver's door, open it, board ──
    if !walk_to(ctx, &truck, plan.door.approach(), plan.radius).await? {
        warn!(npc = %npc, %truck, "could not reach the truck's door");
        return Ok(false);
    }
    face(ctx, &truck, plan.door.outdoor()).await?;
    time::sleep(Duration::from_millis(500)).await;
    DriveToFactory::set_phase(ctx, "board", json!({}));
    if !set_door(ctx, &truck, &plan.door.door_id, true, "outdoor").await? {
        warn!(npc = %npc, %truck, door = %plan.door.door_id, "door would not open");
        return Ok(false);
    }
    if !board(ctx, &truck, &plan.seat_key).await? {
        warn!(npc = %npc, %truck, "could not take the driver's seat");
        return Ok(false);
    }

    // ── 3. Aboard: from here on, any failure parks and gets out ──
    let arrived = drive(ctx, plan, &truck, lock).await;
    let out = park_and_exit(ctx, plan, &truck).await;
    let arrived = arrived?;
    if !out? {
        warn!(npc = %npc, %truck, "could not get out of the truck");
    }
    Ok(arrived)
}

/// The truck the kiosk handed out (or one already waiting free on its spawn
/// spot), pressing the kiosk when needed.
async fn get_truck(ctx: &mut ActionContext, plan: &Plan) -> Result<Option<String>> {
    let spawn = plan.kiosk.spawn.as_ref().map(|s| s.position()).unwrap_or_default();
    let Some(spawn_pos) = driving::object_point_to_planet(&ctx.world, &plan.garage, spawn, plan.radius)
    else {
        return Ok(None);
    };
    for attempt in 0..KIOSK_ATTEMPTS {
        if let Some(t) = free_truck_near(ctx, spawn_pos, plan.radius) {
            return Ok(Some(t));
        }
        if !walk_to(ctx, &plan.garage, plan.kiosk.approach(), plan.radius).await? {
            return Ok(None);
        }
        face(ctx, &plan.garage, plan.kiosk.position()).await?;
        time::sleep(Duration::from_millis(600)).await;
        info!(npc = %ctx.npc_name, attempt, "pressing the truck kiosk");
        send(ctx, json!({
            "action": "kiosk_spawn",
            "target_uuid": plan.garage,
            "kiosk": plan.kiosk.path,
        }))
        .await?;
        let deadline = Instant::now() + SPAWN_WAIT;
        while Instant::now() < deadline {
            if let Some(t) = free_truck_near(ctx, spawn_pos, plan.radius) {
                return Ok(Some(t));
            }
            time::sleep(Duration::from_millis(250)).await;
        }
        // Refused (cooldown, something in the spawn box): wait it out.
        let wait = Duration::from_secs_f64(plan.kiosk.cooldown_s.max(5.0) + 2.0);
        info!(npc = %ctx.npc_name, wait_s = wait.as_secs(), "no truck appeared; waiting out the kiosk cooldown");
        time::sleep(wait).await;
    }
    Ok(None)
}

/// A truck nobody drives standing on the kiosk's spawn spot.
fn free_truck_near(ctx: &ActionContext, spawn_pos: Vec3, radius: f64) -> Option<String> {
    let candidates: Vec<String> = ctx
        .world
        .objects
        .iter()
        .filter(|o| o.object_type == "vehicle")
        .filter(|o| o.scenename.as_deref().map(scene_layouts::normalize) == Some(TRUCK_SCENE))
        .filter(|o| {
            let s = VehicleState::read(o);
            s.pilot_uuid.is_empty() || s.pilot_uuid == ctx.world.my_uuid
        })
        .map(|o| o.uuid.clone())
        .collect();
    candidates
        .into_iter()
        .filter_map(|u| {
            let p = driving::object_planet_position(&ctx.world, &u, radius)?;
            let d = geo::length(geo::sub(p, spawn_pos));
            (d <= SPAWN_MATCH_M).then_some((u, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(u, _)| u)
}

/// The object of `scene` nearest `from` within `max_m`.
fn nearest_object(ctx: &ActionContext, scene: &str, from: Vec3, radius: f64, max_m: f64) -> Option<String> {
    let ids: Vec<String> = ctx
        .world
        .objects
        .iter()
        .filter(|o| o.scenename.as_deref().map(scene_layouts::normalize) == Some(scene))
        .map(|o| o.uuid.clone())
        .collect();
    ids.into_iter()
        .filter_map(|u| {
            let p = driving::object_planet_position(&ctx.world, &u, radius)?;
            let d = geo::length(geo::sub(p, from));
            (d <= max_m).then_some((u, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(u, _)| u)
}

async fn send(ctx: &ActionContext, data: Value) -> Result<()> {
    ctx.send.send("player", "client_action", data).await
}

async fn face(ctx: &ActionContext, frame: &str, point: Vec3) -> Result<()> {
    send(ctx, json!({"action": "npc_face_position", "uuid": frame, "position": point})).await
}

/// Walk to `target` (local to object `frame`) and wait until there. The
/// arrival fallback measures in the planet frame, rotation included (the
/// garage and the truck are seldom axis-aligned).
async fn walk_to(ctx: &ActionContext, frame: &str, target: Vec3, radius: f64) -> Result<bool> {
    send(ctx, json!({"action": "npc_arrived_ack", "uuid": frame})).await?;
    ctx.world.clear_arrived();
    send(ctx, json!({"action": "npc_go_to_position", "uuid": frame, "position": target})).await?;
    let deadline = Instant::now() + WALK_TIMEOUT;
    while Instant::now() < deadline {
        if ctx.world.npc_arrived() {
            send(ctx, json!({"action": "npc_arrived_ack", "uuid": frame})).await?;
            ctx.world.clear_arrived();
            return Ok(true);
        }
        let close = driving::object_point_to_planet(&ctx.world, frame, target, radius)
            .zip(driving::my_planet_position(&ctx.world, radius))
            .is_some_and(|(t, me)| {
                // Ground distance: drop the component along the local vertical.
                let d = geo::sub(t, me);
                let up = geo::normalized(me);
                let h = geo::sub(d, geo::scale(up, geo::dot(d, up)));
                geo::length(h) <= WALK_ARRIVED_M
            });
        if close {
            return Ok(true);
        }
        time::sleep(Duration::from_millis(200)).await;
    }
    Ok(false)
}

/// Open (`open = true`) or close a door, from `side` ("outdoor" on foot,
/// "indoor" seated), and wait until the replicated state agrees.
async fn set_door(ctx: &ActionContext, truck: &str, door_id: &str, open: bool, side: &str) -> Result<bool> {
    let is = |ctx: &ActionContext| {
        ctx.world.objects.get(truck).is_some_and(|o| driving::door_open(&o, door_id) == open)
    };
    for _ in 0..REQUEST_ATTEMPTS {
        if is(ctx) {
            return Ok(true);
        }
        send(ctx, json!({
            "action": "vehicle_door",
            "target_uuid": truck,
            "door_id": door_id,
            "side": side,
        }))
        .await?;
        let deadline = Instant::now() + CONFIRM_WAIT;
        while Instant::now() < deadline {
            if is(ctx) {
                return Ok(true);
            }
            time::sleep(Duration::from_millis(150)).await;
        }
    }
    Ok(is(ctx))
}

/// Sit in `seat` and wait until the server has put the NPC there.
async fn board(ctx: &ActionContext, truck: &str, seat: &str) -> Result<bool> {
    let seated = |ctx: &ActionContext| {
        ctx.world.my_parent().as_deref() == Some(truck)
            || ctx
                .world
                .objects
                .get(truck)
                .and_then(|o| driving::seat_occupant(&o, seat))
                .is_some_and(|u| u == ctx.world.my_uuid)
    };
    for _ in 0..REQUEST_ATTEMPTS {
        send(ctx, json!({"action": "enter_vehicle", "target_uuid": truck, "seat": seat})).await?;
        let deadline = Instant::now() + CONFIRM_WAIT;
        while Instant::now() < deadline {
            if seated(ctx) {
                return Ok(true);
            }
            time::sleep(Duration::from_millis(150)).await;
        }
    }
    Ok(false)
}

fn truck_state(ctx: &ActionContext, truck: &str) -> Option<VehicleState> {
    ctx.world.objects.get(truck).map(|o| VehicleState::read(&o))
}

/// Toggle a boolean vehicle control (`vehicle_ignition`, `vehicle_handbrake`)
/// until the replicated `read` reports `want`.
async fn set_control(
    ctx: &ActionContext,
    truck: &str,
    action: &str,
    want: bool,
    read: impl Fn(&VehicleState) -> bool,
) -> Result<bool> {
    if truck_state(ctx, truck).is_some_and(|s| read(&s) == want) {
        return Ok(true);
    }
    send(ctx, json!({"action": action, "target_uuid": truck})).await?;
    let deadline = Instant::now() + CONFIRM_WAIT;
    while Instant::now() < deadline {
        if truck_state(ctx, truck).is_some_and(|s| read(&s) == want) {
            return Ok(true);
        }
        time::sleep(Duration::from_millis(150)).await;
    }
    Ok(false)
}

async fn send_input(ctx: &ActionContext, truck: &str, c: &Command) -> Result<()> {
    send(ctx, json!({
        "action": "vehicle_input",
        "target_uuid": truck,
        "throttle": c.throttle,
        "steer": c.steer,
        "brake": c.brake,
    }))
    .await
}

/// The truck as the controller sees it.
fn observe(ctx: &ActionContext, truck: &str, forward_local: Vec3, radius: f64) -> Option<Observation> {
    let pos = driving::object_planet_position(&ctx.world, truck, radius)?;
    let forward = driving::object_dir_to_planet(&ctx.world, truck, forward_local, radius)?;
    let st = truck_state(ctx, truck)?;
    Some(Observation {
        pos,
        forward,
        speed_mps: st.forward_mps,
        steering_rad: st.steering_rad,
        // A truck at rest replicates nothing (only changes are sent), so the
        // age of the last frame says nothing about the link: never stale.
        age_s: 0.0,
    })
}

/// The replicated village (`poi_village`) nearest `pos`, within `max_m`.
fn village_object_near(ctx: &ActionContext, pos: Vec3, radius: f64, max_m: f64) -> Option<String> {
    let ids: Vec<String> = ctx
        .world
        .objects
        .iter()
        .filter(|o| o.object_type == "poi_village")
        .map(|o| o.uuid.clone())
        .collect();
    ids.into_iter()
        .filter_map(|u| {
            let p = driving::object_planet_position(&ctx.world, &u, radius)?;
            let d = geo::arc_m(p, pos, radius);
            (d <= max_m).then_some((u, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(u, _)| u)
}

/// The layout a replicated village was built from (its `spawn_scene`).
fn village_layout<'a>(ctx: &ActionContext, village: &str, layouts: &'a SceneLayouts) -> Option<&'a SceneLayout> {
    let scene = ctx
        .world
        .objects
        .get(village)?
        .property("spawn_scene")
        .and_then(Value::as_str)
        .map(str::to_string)?;
    layouts.get(&scene)
}

/// The village's building footprints, planet frame. The layout's frame is the
/// village object's: every item was spawned at `village pose × layout pose`.
fn village_obstacles(ctx: &ActionContext, village: &str, layout: &SceneLayout, radius: f64) -> Vec<offroad::Obstacle> {
    layout
        .obstacles
        .iter()
        .filter_map(|o| {
            o.corner_points()
                .map(|c| driving::object_point_to_planet(&ctx.world, village, c, radius))
                .collect::<Option<Vec<_>>>()
        })
        .collect()
}

/// Off-road path from `from` to `to` around `obstacles`; a straight line
/// (logged) when the planner finds no way.
fn offroad_path(npc: &str, from: Vec3, to: Vec3, obstacles: &[offroad::Obstacle], what: &str) -> Vec<Vec3> {
    match offroad::plan(from, to, obstacles, OFFROAD_CLEARANCE_M, OFFROAD_CELL_M) {
        Some(p) => p,
        None => {
            warn!(npc = %npc, what, obstacles = obstacles.len(), "no way round the buildings; driving straight");
            vec![from, to]
        }
    }
}

/// From the truck at the garage to the road: straight out of the garage door,
/// then around the village's buildings to the nearest road point (which may be
/// several hundred metres away). The last point is on the road.
fn origin_leg(ctx: &ActionContext, plan: &Plan, truck_pos: Vec3) -> Option<Vec<Vec3>> {
    let npc = &ctx.npc_name;
    let r = plan.radius;
    let spawn = plan.kiosk.spawn.as_ref()?;
    let out_dir = driving::object_dir_to_planet(&ctx.world, &plan.garage, spawn.forward(), r)?;
    let spawn_pos = driving::object_point_to_planet(&ctx.world, &plan.garage, spawn.position(), r)?;
    let outside = geo::add(spawn_pos, geo::scale(out_dir, GARAGE_EXIT_M));
    let entry = plan.wd.road_point_near(outside)?;
    let layouts = SceneLayouts::shared();
    let obstacles = village_object_near(ctx, truck_pos, r, VILLAGE_SEARCH_M)
        .and_then(|v| village_layout(ctx, &v, &layouts).map(|l| village_obstacles(ctx, &v, l, r)))
        .unwrap_or_else(|| {
            warn!(npc = %npc, "home village layout unknown; no buildings to drive around");
            Vec::new()
        });
    let mut leg = vec![truck_pos];
    leg.extend(offroad_path(npc, outside, entry, &obstacles, "garage to road"));
    info!(
        npc = %npc,
        to_road_m = geo::arc_m(outside, entry, r).round() as u64,
        waypoints = leg.len(),
        obstacles = obstacles.len(),
        "way out of the village planned"
    );
    Some(leg)
}

/// From the road to the destination garage, once the destination village is
/// in view (villages replicate up to 10 km away): its garages are known from
/// its layout, so the trip leaves the road at the point nearest the garage —
/// not the village centre — and winds round the buildings to park in front of
/// it. The first point is on the road. `None` while the village is not in view.
fn destination_leg(ctx: &ActionContext, plan: &Plan) -> Option<Vec<Vec3>> {
    let r = plan.radius;
    let layouts = SceneLayouts::shared();
    let centre = plan.wd.village_position(&plan.dest);
    let village = village_object_near(ctx, centre, r, (plan.dest.radius_m * 1.5).max(1_500.0))?;
    let vlayout = village_layout(ctx, &village, &layouts)?;
    let garage_l = layouts.get(GARAGE_SCENE)?;
    let park_local = match garage_l.marker(PARKING_MARKER) {
        Some(m) => m.position(),
        None => {
            let spawn = garage_l.kiosks.iter().find_map(|k| k.spawn.clone())?;
            geo::add(spawn.position(), geo::scale(spawn.forward(), GARAGE_EXIT_M))
        }
    };
    // The garage whose parking is closest to a road.
    let (garage, park, exit) = vlayout
        .garages
        .iter()
        .filter_map(|g| {
            let park = driving::object_point_to_planet(&ctx.world, &village, g.to_village(park_local), r)?;
            let exit = plan.wd.road_point_near(park)?;
            Some((g.name.clone(), park, exit))
        })
        .min_by(|a, b| geo::arc_m(a.1, a.2, r).total_cmp(&geo::arc_m(b.1, b.2, r)))?;
    let obstacles = village_obstacles(ctx, &village, vlayout, r);
    let leg = offroad_path(&ctx.npc_name, exit, park, &obstacles, "road to garage");
    info!(
        npc = %ctx.npc_name,
        %garage,
        from_road_m = geo::arc_m(exit, park, r).round() as u64,
        waypoints = leg.len(),
        obstacles = obstacles.len(),
        "destination village in view; way to its garage planned"
    );
    Some(leg)
}

fn tuning_for(plan: &Plan) -> Tuning {
    Tuning { max_speed_mps: plan.max_speed_kmh / 3.6, ..Tuning::default() }
}

/// The off-road route round a blockage on `side` of it, from `start`, planned
/// on the route the blockage was met on. Village buildings near the truck are
/// kept clear of too.
fn plan_detour(
    ctx: &ActionContext,
    plan: &Plan,
    on: &Controller,
    req: &BypassRequest,
    obstacles: &[Obstacle],
    start: Vec3,
) -> Option<(Controller, f64)> {
    let layouts = SceneLayouts::shared();
    let buildings = village_object_near(ctx, start, plan.radius, VILLAGE_SEARCH_M)
        .and_then(|v| village_layout(ctx, &v, &layouts).map(|l| village_obstacles(ctx, &v, l, plan.radius)))
        .unwrap_or_default();
    let (pts, detour_m) = traffic::detour(on, req, obstacles, &buildings, start, traffic::DETOUR_KMH)?;
    Some((Controller::new(pts, plan.params, traffic::detour_tuning(tuning_for(plan))), detour_m))
}

fn controller_for(plan: &Plan, route: &crate::world_data::road_graph::Route) -> Controller {
    Controller::new(
        route.points.iter().map(|p| RoutePt {
            unit: p.unit,
            s: p.s_m,
            speed_kmh: p.speed_kmh,
            half_width_m: p.half_width_m,
            lanes: p.lanes,
        }),
        plan.params,
        tuning_for(plan),
    )
}

/// Start the truck and drive it to the destination. `Ok(true)` = stopped at
/// the end of the route.
async fn drive(ctx: &mut ActionContext, plan: &Plan, truck: &str, lock: GarageLock) -> Result<bool> {
    let npc = ctx.npc_name.clone();
    // Shut the door before moving (the wind would, noisily), start the engine.
    // The hand brake lets go by itself on the first throttle.
    let _ = set_door(ctx, truck, &plan.door.door_id, false, "indoor").await?;
    if !set_control(ctx, truck, "vehicle_ignition", true, |s| s.engine).await? {
        warn!(npc = %npc, %truck, "engine would not start (no motor or no charge?)");
        return Ok(false);
    }

    let forward_local = plan.truck_layout.vehicle.as_ref().map(|v| v.forward()).unwrap_or(Vec3 {
        x: 0.0,
        y: 0.0,
        z: -1.0,
    });
    let Some(start) = driving::object_planet_position(&ctx.world, truck, plan.radius) else {
        return Ok(false);
    };
    let Some(start_leg) = origin_leg(ctx, plan, start) else {
        warn!(npc = %npc, "cannot plan the way out of the garage");
        return Ok(false);
    };
    // The destination garage is planned for as soon as its village is in view;
    // until then the trip aims at the road point nearest the village centre.
    let mut dest_leg = destination_leg(ctx, plan);
    let end_leg = dest_leg.clone().unwrap_or_else(|| vec![plan.dest_access]);
    let Some(route) = plan.wd.route_via(&start_leg, &end_leg) else {
        warn!(npc = %npc, "no road route from the truck to the destination");
        return Ok(false);
    };
    // Metres of the route still off-road at the start: the destination is not
    // re-planned before the truck is on the road.
    let start_offroad_m = start_leg
        .windows(2)
        .map(|w| geo::arc_m(w[0], w[1], plan.radius))
        .sum::<f64>();
    info!(npc = %npc, length_m = route.length_m as u64, points = route.points.len(), "route planned");
    DriveToFactory::set_phase(ctx, "drive", json!({"length_m": route.length_m.round()}));
    let mut ctrl = controller_for(plan, &route);
    let mut last_dest_try = Instant::now();
    // Traffic: what is near the road, and how to deal with it (ADR 0012).
    let mut traffic = Traffic::default();
    let mut tracker = Tracker::default();
    let clock = Instant::now();
    // While going round a blockage: the route it was met on, the request and
    // what was in the way — to try the other side if this one fails.
    let mut detour: Option<(Controller, BypassRequest, Vec<Obstacle>)> = None;
    let mut horn_release: Option<Instant> = None;

    let tick = drive_tick();
    let mut ticker = time::interval(tick);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut lock = Some(lock);
    let mut resets = 0;
    let mut last_cmd: Option<(Command, Instant)> = None;
    let mut last_tick = Instant::now();
    let mut last_seen = Instant::now();
    let mut last_report = Instant::now();
    let mut traffic_mode = Mode::Cruise;

    loop {
        ticker.tick().await;
        let dt = last_tick.elapsed().as_secs_f64().clamp(0.01, 0.5);
        last_tick = Instant::now();

        let Some(obs) = observe(ctx, truck, forward_local, plan.radius) else {
            if last_seen.elapsed() > TRUCK_LOST {
                warn!(npc = %npc, %truck, "lost sight of the truck; stopping");
                return Ok(false);
            }
            continue;
        };
        last_seen = Instant::now();

        if lock.is_some() && ctrl.progress_m() > RELEASE_GARAGE_AFTER_M {
            lock = None; // spawn box cleared: the kiosk is free again
        }
        if dest_leg.is_none()
            && ctrl.progress_m() > start_offroad_m + 20.0
            && last_dest_try.elapsed() >= DEST_RETRY
        {
            last_dest_try = Instant::now();
            if let Some(leg) = destination_leg(ctx, plan) {
                // On the road now: re-plan from here, along the road to the
                // point nearest the garage, then off-road to it.
                if let Some(r) = plan.wd.route_via(&[obs.pos], &leg) {
                    info!(npc = %npc, remaining_m = r.length_m as u64, "re-planned to the destination garage");
                    ctrl = controller_for(plan, &r);
                    traffic.new_route(None);
                    detour = None;
                }
                dest_leg = Some(leg);
            }
        }

        // ── Traffic: look, decide, tell the controller ──
        let now = clock.elapsed().as_secs_f64();
        let around = obstacles::snapshot(&ctx.world, truck, obs.pos, LOOK_AHEAD_M + 50.0, plan.radius, &mut tracker, now);
        let seen: Vec<Seen> = around.iter().filter_map(|o| traffic::seen(&ctrl, obs.pos, o, LOOK_AHEAD_M)).collect();
        let me = Me { s: ctrl.s_now(), d: ctrl.lateral_m(), v: obs.speed_mps, cruise_mps: ctrl.speed_limit_at(ctrl.s_now()) };
        let decision = traffic.decide(now, me, &seen, &ctrl);
        ctrl.set_manoeuvre(decision.manoeuvre);
        ctrl.set_speed_cap(decision.speed_cap);
        ctrl.set_stop_at(decision.stop_at);
        if decision.mode != traffic_mode {
            info!(npc = %npc, mode = ?decision.mode, lead = ?decision.lead, "traffic");
            traffic_mode = decision.mode;
        }
        let lead = decision.lead.clone();
        let attended = decision.attended;
        if decision.honk {
            info!(npc = %npc, lead = ?lead, "honking at the blockage");
            send(ctx, json!({"action": "vehicle_horn", "target_uuid": truck, "pressed": true})).await?;
            horn_release = Some(Instant::now() + HONK_HOLD);
        }
        if horn_release.is_some_and(|at| Instant::now() >= at) {
            send(ctx, json!({"action": "vehicle_horn", "target_uuid": truck, "pressed": false})).await?;
            horn_release = None;
        }
        if let Some(req) = decision.bypass {
            warn!(npc = %npc, blockers = ?req.blockers, side = req.side, "road blocked for a minute; going round it");
            match plan_detour(ctx, plan, &ctrl, &req, &around, obs.pos) {
                Some((c, detour_m)) => {
                    detour = Some((ctrl.clone(), req, around.clone()));
                    ctrl = c;
                    traffic.new_route(Some(detour_m));
                    last_cmd = None;
                }
                None => {
                    warn!(npc = %npc, "no way round the blockage on that side");
                    traffic.bypass_failed();
                    if traffic.bypass_attempts >= 2 {
                        return give_up_blocked(ctx, truck).await;
                    }
                }
            }
        }

        match ctrl.step(&obs, dt) {
            Step::Arrived => {
                send_input(ctx, truck, &Command::STOP).await?;
                return Ok(true);
            }
            Step::NeedsReset if traffic.mode() == Mode::Bypass => {
                // Could not get round on this side (a cutting's wall, a
                // tunnel…): the other side, from where the truck is now.
                let Some((on, req, around)) = detour.take().filter(|_| traffic.bypass_attempts < 1) else {
                    warn!(npc = %npc, "could not get round the blockage on either side");
                    return give_up_blocked(ctx, truck).await;
                };
                traffic.bypass_failed();
                let other = BypassRequest { side: -req.side, ..req };
                warn!(npc = %npc, side = other.side, "stuck going round; trying the other side");
                match plan_detour(ctx, plan, &on, &other, &around, obs.pos) {
                    Some((c, detour_m)) => {
                        let attempts = traffic.bypass_attempts;
                        detour = Some((on, other, around));
                        ctrl = c;
                        traffic.new_route(Some(detour_m));
                        traffic.bypass_attempts = attempts;
                        last_cmd = None;
                    }
                    None => return give_up_blocked(ctx, truck).await,
                }
            }
            Step::NeedsReset if lead.as_ref().is_some_and(|(_, gap)| *gap < BLOCKED_AHEAD_M) => {
                // Pushing against something it sees: blocked, not stuck. The
                // traffic logic stops it and, after a while, goes round.
                ctrl.reset_stuck();
                last_cmd = None;
            }
            Step::NeedsReset => {
                resets += 1;
                warn!(npc = %npc, %truck, resets, "truck stuck; resetting it");
                if resets > MAX_RESETS {
                    send_input(ctx, truck, &Command::STOP).await?;
                    return Ok(false);
                }
                send(ctx, json!({"action": "reset_vehicle", "target_uuid": truck})).await?;
                time::sleep(Duration::from_secs(2)).await;
                ctrl.reset_stuck();
                last_cmd = None;
            }
            Step::Drive(cmd) => {
                let due = last_cmd
                    .as_ref()
                    .is_none_or(|(c, at)| c.differs(&cmd) || at.elapsed() >= INPUT_REFRESH);
                if due {
                    send_input(ctx, truck, &cmd).await?;
                    last_cmd = Some((cmd, Instant::now()));
                }
            }
        }

        if last_report.elapsed() >= Duration::from_secs(5) {
            last_report = Instant::now();
            let progress = json!({
                "progress_m": ctrl.progress_m().round(),
                "remaining_m": ctrl.remaining_m().round(),
                "speed_kmh": (obs.speed_mps * 3.6).round(),
                "cross_track_m": (ctrl.cross_track_m * 10.0).round() / 10.0,
                "lateral_m": (ctrl.lateral_m() * 10.0).round() / 10.0,
                "traffic": {
                    "mode": traffic.mode(),
                    "lead": lead.as_ref().map(|(id, _)| id),
                    "lead_gap_m": lead.as_ref().map(|(_, g)| (g * 10.0).round() / 10.0),
                    "blocked_s": traffic.blocked_s(now).round(),
                    "attended": attended,
                    "bypass_attempts": traffic.bypass_attempts,
                    "seen": seen.len(),
                },
            });
            debug!(npc = %npc, %progress, "driving");
            DriveToFactory::set_phase(ctx, "drive", progress);
        }
    }
}

/// The road is blocked and there is no way round: stop, record it, give the
/// trip up (`trip` then parks and gets out).
async fn give_up_blocked(ctx: &ActionContext, truck: &str) -> Result<bool> {
    send_input(ctx, truck, &Command::STOP).await?;
    DriveToFactory::set_phase(ctx, "drive", json!({"error": "road blocked"}));
    Ok(false)
}

/// Stop, hand brake, engine off, open the door, get out. `Ok(true)` once the
/// NPC is out.
async fn park_and_exit(ctx: &ActionContext, plan: &Plan, truck: &str) -> Result<bool> {
    DriveToFactory::set_phase(ctx, "exit", json!({}));
    send_input(ctx, truck, &Command::STOP).await?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline
        && truck_state(ctx, truck).is_some_and(|s| s.forward_mps.abs() > 0.3)
    {
        time::sleep(Duration::from_millis(200)).await;
    }
    let _ = set_control(ctx, truck, "vehicle_handbrake", true, |s| s.handbrake).await?;
    let _ = set_control(ctx, truck, "vehicle_ignition", false, |s| s.engine).await?;
    // The server refuses to let a seated player out through a shut door.
    let _ = set_door(ctx, truck, &plan.door.door_id, true, "indoor").await?;
    let out = |ctx: &ActionContext| ctx.world.my_parent().as_deref() != Some(truck);
    for _ in 0..REQUEST_ATTEMPTS {
        send(ctx, json!({"action": "exit_vehicle", "target_uuid": truck})).await?;
        let deadline = Instant::now() + CONFIRM_WAIT;
        while Instant::now() < deadline {
            if out(ctx) {
                return Ok(true);
            }
            time::sleep(Duration::from_millis(150)).await;
        }
    }
    Ok(out(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garage_lock_is_exclusive_and_released_on_drop() {
        let a = GarageLock::try_take("g-test", "alice").expect("free");
        assert!(GarageLock::try_take("g-test", "bob").is_none(), "taken by alice");
        drop(a);
        let b = GarageLock::try_take("g-test", "bob").expect("released");
        drop(b);
    }
}
