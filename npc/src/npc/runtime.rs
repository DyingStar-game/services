//! NPC runtime: one tokio task per NPC. Connects to Horizon (with reconnect),
//! maintains a [`WorldView`] from inbound GORC frames, and runs an
//! [`ActionRegistry`] scheduler on a fixed tick.

use std::{sync::Arc, time::Duration};

use rand::{SeedableRng, rngs::StdRng};
use tokio::{
    sync::{Mutex, oneshot},
    time,
};
use tracing::{debug, info, warn};

use crate::{
    actions::{ActionContext, ActionRegistry},
    clock::{ClockParams, PlanetClock},
    config::Config,
    depot::{DepotRole, DepotWorkforce},
    horizon::{
        client::{HorizonClient, HorizonReceiver, HorizonSender},
        messages::StandardizedFrame,
    },
    jwt,
    npc::{
        spec::NpcSpec,
        state::{NpcState, NpcStatus},
    },
    resources_dynamic::PlanetCatalogHandle,
    schedule::{Activity, Schedule},
    world::view::WorldView,
};

/// Poll cadence while an NPC waits out an off-shift window before (re)connecting.
/// Planetary time maps 1:1 to real time, so a coarse poll is plenty — an idle,
/// disconnected NPC has nothing to do until its shift starts.
const OFF_SHIFT_POLL: Duration = Duration::from_secs(30);

/// Whether `pattern` puts the NPC off-shift at the current planetary time.
/// NPCs without a schedule pattern are never off-shift (they stay connected).
fn off_shift_now(schedule: &Schedule, pattern: Option<&str>, clock: &PlanetClock) -> bool {
    matches!(
        pattern.and_then(|p| schedule.activity_at(p, clock.now())),
        Some(Activity::Off)
    )
}

/// Sleep until the NPC's shift schedule leaves the off-shift window, or until a
/// stop is requested. Returns `true` once on-shift, `false` if stopped.
async fn wait_until_on_shift(
    name: &str,
    schedule: &Schedule,
    pattern: Option<&str>,
    clock: &PlanetClock,
    stop_rx: &mut oneshot::Receiver<()>,
) -> bool {
    if !off_shift_now(schedule, pattern, clock) {
        return true;
    }
    info!(npc = %name, "off-shift; staying disconnected until shift starts");
    while off_shift_now(schedule, pattern, clock) {
        tokio::select! {
            biased;
            _ = &mut *stop_rx => return false,
            _ = tokio::time::sleep(OFF_SHIFT_POLL) => {}
        }
    }
    info!(npc = %name, "shift started; connecting");
    true
}

/// Shared slot holding an NPC's live [`WorldView`], published by its runtime on
/// every session and read by the admin API. `None` while the NPC is between
/// sessions (off shift, reconnecting).
pub type WorldSlot = Arc<std::sync::RwLock<Option<Arc<WorldView>>>>;

/// Shared slot holding an NPC's live [`NpcState`], published alongside its
/// [`WorldSlot`]. "Which status is it in" is the other half of answering why an
/// NPC stands idle: an NPC stuck in `GoingToWork` looks exactly like one that
/// has nothing to do.
pub type StateSlot = Arc<std::sync::RwLock<Option<Arc<NpcState>>>>;

pub struct NpcHandle {
    pub name: String,
    pub spec: NpcSpec,
    pub stop_tx: oneshot::Sender<()>,
    pub join: tokio::task::JoinHandle<()>,
    /// This NPC's live view of the world, for `GET /npcs/world`.
    pub world: WorldSlot,
    /// This NPC's live status + blackboard, for `GET /npcs/world`.
    pub state: StateSlot,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    config: Arc<Config>,
    clock_params: Arc<ClockParams>,
    planets: PlanetCatalogHandle,
    schedule: Arc<Schedule>,
    workforce: Arc<DepotWorkforce>,
    name: String,
    spec: NpcSpec,
    world_slot: WorldSlot,
    state_slot: StateSlot,
    mut stop_rx: oneshot::Receiver<()>,
) {
    info!(npc = %name, actions = ?spec.action_profile, "NPC runtime started");

    // Resolve this NPC's planet -> rotation period -> planetary clock. The
    // planet key comes from `place_where_work.planet`, falling back to the
    // service-wide default. Done once here (rotation periods change rarely).
    let planet_key = spec
        .place_where_work
        .as_ref()
        .and_then(|p| p.planet.clone())
        .unwrap_or_else(|| config.planet_internal_name.clone());
    let rotation_h = planets
        .rotation_h_or(&planet_key, config.rotation_h_fallback)
        .await;
    let clock = PlanetClock::new(Arc::clone(&clock_params), rotation_h);
    // The schedule pattern that gates this NPC's actions per planetary hour.
    let pattern = match &spec.schedule {
        Some(p) if schedule.has_pattern(p) => Some(p.clone()),
        Some(p) => {
            warn!(npc = %name, schedule = %p, "unknown schedule pattern; no schedule gating");
            None
        }
        None => None,
    };
    info!(npc = %name, planet = %planet_key, rotation_h, schedule = ?pattern, "planetary clock resolved");

    let tick = Duration::from_millis(config.tick_interval_ms);
    let startup_delay = Duration::from_millis(config.action_startup_delay_ms);
    // Depot this NPC works at (for the reception role split); `None` = no
    // workplace, so no depot coordination applies.
    let depot_uuid = spec.place_where_work.as_ref().map(|p| p.uuid.clone());

    loop {
        // If the shift schedule says this NPC is off-shift right now, stay
        // disconnected until its shift starts. This covers both startup (never
        // connect while off) and the return after going home (the session below
        // drops the connection once the NPC is home, then we wait here). NPCs
        // without a schedule are never off-shift and fall straight through.
        if !wait_until_on_shift(&name, &schedule, pattern.as_deref(), &clock, &mut stop_rx).await {
            info!(npc = %name, "stop requested while off-shift; exiting runtime");
            return;
        }

        // Generate a fresh JWT before each connection attempt.
        let display_name = spec.name.as_deref().unwrap_or(&name);
        let token = match jwt::generate(&spec.uuid, display_name, &config.jwt_secret) {
            Ok(t) => t,
            Err(e) => {
                warn!(npc = %name, error = %e, "cannot generate JWT; retrying");
                tokio::time::sleep(Duration::from_millis(config.reconnect_backoff_initial_ms)).await;
                continue;
            }
        };

        // Connect (with backoff) until we have a live session, or stop.
        let session = tokio::select! {
            biased;
            _ = &mut stop_rx => {
                info!(npc = %name, "stop requested before connection");
                return;
            }
            t = HorizonClient::connect_with_retry(&config, &config.horizon_url, &token) => t,
        };
        let (sender, mut receiver, my_uuid) = session;
        let world = Arc::new(WorldView::new(my_uuid.clone()));
        // Publish the fresh view to the admin API. Depot behaviour is driven
        // entirely by properties replicated onto the workplace object, so "what
        // does this NPC actually see on its depot" is the first question to ask
        // when an NPC stands idle — and the only way to answer it from outside.
        *world_slot.write().unwrap() = Some(Arc::clone(&world));
        // Shared per-NPC state (FSM status + blackboard), built fresh per
        // session alongside the WorldView: a reconnect respawns the NPC, so
        // its lifecycle starts clean. An NPC that works spawns in its
        // habitation; `going_to_work` walks it out from there.
        let state = Arc::new(NpcState::new());
        *state_slot.write().unwrap() = Some(Arc::clone(&state));
        if spec.place_where_work.is_some() {
            state.set_status(NpcStatus::AtHome);
        }
        info!(
            npc = %name,
            %my_uuid,
            settle_ms = config.action_startup_delay_ms,
            "Horizon session established; settling before running actions"
        );

        // Actions are rebuilt per session, alongside the WorldView: a fresh
        // connection means a freshly spawned NPC, so per-session action state
        // (e.g. "already went to work") starts clean.
        let registry = Arc::new(ActionRegistry::from_spec(&spec));

        let current = Arc::new(Mutex::new(None::<tokio::task::JoinHandle<()>>));

        let outcome = session_loop(
            &name,
            tick,
            startup_delay,
            Arc::clone(&world),
            Arc::clone(&state),
            sender.clone(),
            &mut receiver,
            Arc::clone(&registry),
            Arc::clone(&current),
            &clock,
            Arc::clone(&schedule),
            pattern.as_deref(),
            &workforce,
            depot_uuid.as_deref(),
            &spec.uuid,
            &mut stop_rx,
        )
        .await;

        if let Some(h) = current.lock().await.take() {
            h.abort();
        }

        match outcome {
            SessionOutcome::Stop => {
                info!(npc = %name, "stop requested; exiting runtime");
                return;
            }
            SessionOutcome::Disconnected => {
                warn!(npc = %name, "disconnected from Horizon; reconnecting");
                continue;
            }
            SessionOutcome::OffShift => {
                info!(npc = %name, "off-shift and home; closing session until next shift");
                continue;
            }
        }
    }
}

enum SessionOutcome {
    Stop,
    Disconnected,
    /// The NPC went off-shift and has settled at home: drop the Horizon session
    /// and let the outer loop wait out the off-shift window before reconnecting.
    OffShift,
}

#[allow(clippy::too_many_arguments)]
async fn session_loop(
    name: &str,
    tick: Duration,
    startup_delay: Duration,
    world: Arc<WorldView>,
    state: Arc<NpcState>,
    sender: HorizonSender,
    receiver: &mut HorizonReceiver,
    registry: Arc<ActionRegistry>,
    current: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    clock: &PlanetClock,
    schedule: Arc<Schedule>,
    pattern: Option<&str>,
    workforce: &DepotWorkforce,
    depot_uuid: Option<&str>,
    my_uuid: &str,
    stop_rx: &mut oneshot::Receiver<()>,
) -> SessionOutcome {
    // Hold off on the first tick so the NPC settles in before acting. Only the
    // scheduler waits — the loop below keeps applying inbound frames, so the
    // world is already populated when the first action runs.
    let mut ticker = time::interval_at(time::Instant::now() + startup_delay, tick);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    // Last depot role logged, so only the changes are reported.
    let mut last_depot_role: Option<DepotRole> = None;

    loop {
        tokio::select! {
            biased;
            _ = &mut *stop_rx => return SessionOutcome::Stop,

            maybe_frame = receiver.recv() => {
                match maybe_frame {
                    Some(StandardizedFrame::Gorc(g)) => {
                        let _ = world.apply(&g);
                    }
                    Some(StandardizedFrame::InitAck { player_id }) => {
                        debug!(npc = %name, %player_id, "stray init_ack");
                    }
                    Some(StandardizedFrame::Error { message }) => {
                        warn!(npc = %name, %message, "horizon error");
                        return SessionOutcome::Disconnected;
                    }
                    Some(StandardizedFrame::Other { name: ev, .. }) => {
                        debug!(npc = %name, event = %ev, "ignoring non-GORC frame");
                    }
                    None => return SessionOutcome::Disconnected,
                }
            }

            _ = ticker.tick() => {
                // Remember the NPC's spawn position as home, once per session,
                // before it first moves: an NPC spawns in its habitation, so its
                // first known position is home (used by `go_home`). Runs even
                // while an action is in flight so it is captured as early as
                // possible.
                if state.get(crate::actions::go_to_place::HOME_POSITION_KEY).is_none() {
                    if let Some(pos) = world.my_position() {
                        if let Ok(v) = serde_json::to_value(pos) {
                            state.set(crate::actions::go_to_place::HOME_POSITION_KEY, v);
                            if let Some(parent) = world.my_parent() {
                                state.set(
                                    crate::actions::go_to_place::HOME_PARENT_KEY,
                                    serde_json::json!(parent),
                                );
                            }
                        }
                    }
                }

                // Off-shift and settled at home: no reason to hold a Horizon
                // session open. `go_home` is schedule-gated to the off-window,
                // so by the time the NPC is `AtHome` its trip home is complete
                // and we never cut it short. Drop the session; the outer loop
                // waits out the off-shift window and reconnects at the next shift.
                if state.status() == NpcStatus::AtHome
                    && off_shift_now(&schedule, pattern, clock)
                {
                    return SessionOutcome::OffShift;
                }

                let mut slot = current.lock().await;
                if let Some(h) = slot.as_ref() {
                    if h.is_finished() {
                        slot.take();
                    }
                }
                if slot.is_some() {
                    continue;
                }

                // Which actions does the shift schedule permit this planetary
                // hour? `None` when the NPC has no schedule (no gating).
                let allowed = pattern.and_then(|p| schedule.allowed_actions_at(p, clock.now()));

                // Cargo-depot role this tick: prefer the authoritative
                // `reception_owner` claim replicated on the depot; fall back to
                // the deterministic election when it is not replicated at all —
                // or when the claim is stale (see `is_off_shift_worker`).
                let depot_role = match depot_uuid {
                    Some(d) => {
                        let owner = world
                            .objects
                            .get(d)
                            .and_then(|o| o.property("reception_owner").cloned());
                        // A counter still owned by a co-worker who is off shift
                        // blocks every magasinier that IS on shift: they all read
                        // themselves as stockers and nobody ever takes the box off
                        // the reception. Ignore such a claim and re-elect.
                        let stale = owner
                            .as_ref()
                            .and_then(|v| v.as_str())
                            .filter(|o| !o.is_empty() && *o != my_uuid)
                            .is_some_and(|o| {
                                workforce.is_off_shift_worker(d, o, &schedule, clock.now())
                            });
                        match crate::depot::role_from_owner(owner.as_ref(), my_uuid)
                            .filter(|_| !stale)
                        {
                            Some(r) => r,
                            None => workforce.role_for(d, my_uuid, &schedule, clock.now()),
                        }
                    }
                    None => DepotRole::Uncoordinated,
                };
                // The role decides whether the NPC may touch the reception at
                // all, so a silent flip is the hardest depot stall to diagnose.
                if last_depot_role != Some(depot_role) {
                    info!(npc = %name, role = ?depot_role, "depot role");
                    last_depot_role = Some(depot_role);
                }

                let probe = ActionContext {
                    npc_name: name.to_string(),
                    world: Arc::clone(&world),
                    state: Arc::clone(&state),
                    send: sender.clone(),
                    rng: StdRng::from_entropy(),
                    allowed_actions: allowed.clone(),
                    depot_role,
                };
                let Some(action) = registry.pick(&probe) else {
                    continue;
                };

                let action_name = action.name();
                debug!(npc = %name, action = %action_name, "starting action");

                // Stamp the start before spawning so the `once`/`min_interval`
                // gates see this run on the very next tick.
                registry.mark_started(action_name);
                let run_timeout = registry.timing_of(action_name).and_then(|t| t.timeout);

                let name_owned = name.to_string();
                let action_name_owned = action_name.to_string();
                let registry_for_task = Arc::clone(&registry);
                let mut ctx = ActionContext {
                    npc_name: name.to_string(),
                    world: Arc::clone(&world),
                    state: Arc::clone(&state),
                    send: sender.clone(),
                    rng: StdRng::from_entropy(),
                    allowed_actions: allowed,
                    depot_role,
                };
                let action_clone = Arc::clone(&action);
                let h = tokio::spawn(async move {
                    let result = match run_timeout {
                        Some(d) => match time::timeout(d, action_clone.run(&mut ctx)).await {
                            Ok(r) => r,
                            Err(_) => {
                                warn!(
                                    npc = %name_owned,
                                    action = %action_name_owned,
                                    timeout_ms = d.as_millis(),
                                    "action timed out; aborting run"
                                );
                                Ok(())
                            }
                        },
                        None => action_clone.run(&mut ctx).await,
                    };
                    if let Err(e) = result {
                        warn!(npc = %name_owned, action = %action_name_owned, error = %e, "action failed");
                    }
                    // Stamp completion so the cooldown gate measures from here.
                    registry_for_task.mark_finished(&action_name_owned);
                });
                *slot = Some(h);
            }
        }
    }
}
