//! NPC runtime: one tokio task per NPC. Logs in to Keycloak, connects to
//! Horizon (with reconnect), maintains a [`WorldView`] from inbound GORC frames,
//! and runs an [`ActionRegistry`] scheduler on a fixed tick. Manual tasks from
//! the admin panel pre-empt the schedule. The spec is hot-swappable: a change
//! pushed by the manager rebuilds the actions without reconnecting.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use rand::{SeedableRng, rngs::StdRng};
use serde_json::json;
use ::time::OffsetDateTime;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time,
};
use tracing::{Instrument, debug, info, warn};
use uuid::Uuid;

use crate::{
    actions::{ActionContext, ActionRegistry, driving, manual},
    clock::PlanetClock,
    db,
    depot::DepotRole,
    horizon::{
        client::{HorizonClient, HorizonReceiver, HorizonSender},
        messages::StandardizedFrame,
    },
    npc::{
        manager::{
            CurrentAction, CurrentSlot, DepotCandidate, NpcConfig, RuntimeEvent, Services, StateSlot,
            TaskSignal, WorldSlot,
        },
        state::{NpcState, NpcStatus},
    },
    schedule::{Activity, Schedule},
    world::view::WorldView,
    world_data::WorldData,
};

/// Poll cadence while an NPC waits out an off-shift window before (re)connecting.
/// Planetary time maps 1:1 to real time, so a coarse poll is plenty — an idle,
/// disconnected NPC has nothing to do until its shift starts.
const OFF_SHIFT_POLL: Duration = Duration::from_secs(30);

/// Everything one runtime needs, handed over by the manager at spawn.
pub struct RuntimeCtx {
    pub svc: Arc<Services>,
    pub id: Uuid,
    pub login: String,
    pub cfg_rx: watch::Receiver<NpcConfig>,
    pub world_slot: WorldSlot,
    pub state_slot: StateSlot,
    pub current: CurrentSlot,
    pub tasks: Arc<TaskSignal>,
    pub connected: Arc<std::sync::atomic::AtomicBool>,
    pub runtime_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

/// Whether `pattern` puts the NPC off-shift at the current planetary time.
/// NPCs without a schedule pattern are never off-shift (they stay connected).
fn off_shift_now(schedule: &Schedule, pattern: Option<&str>, clock: &PlanetClock) -> bool {
    matches!(
        pattern.and_then(|p| schedule.activity_at(p, clock.now())),
        Some(Activity::Off)
    )
}

/// What the current spec resolves to: the planetary clock and the schedule
/// pattern that gates the actions.
struct Plan {
    cfg: NpcConfig,
    clock: PlanetClock,
    pattern: Option<String>,
}

impl Plan {
    async fn resolve(ctx: &RuntimeCtx, cfg: NpcConfig) -> Self {
        let (planet, rotation_h, clock) = ctx.svc.clock_for(&cfg.spec).await;
        let pattern = match &cfg.spec.schedule {
            Some(p) if ctx.svc.schedule.load().has_pattern(p) => Some(p.clone()),
            Some(p) => {
                warn!(npc = %ctx.login, schedule = %p, "unknown schedule pattern; no schedule gating");
                None
            }
            None => None,
        };
        info!(npc = %ctx.login, %planet, rotation_h, schedule = ?pattern, "planetary clock resolved");
        Self { cfg, clock, pattern }
    }

    fn depot_uuid(&self) -> Option<String> {
        self.cfg.spec.place_where_work.as_ref().map(|p| p.uuid.clone())
    }
}

enum Wait {
    OnShift,
    Stop,
    /// The spec or the schedule changed: resolve the plan again.
    Changed,
}

/// Sleep until the NPC's shift schedule leaves the off-shift window (or a
/// manual task asks to connect it anyway), the spec or schedule changes, or a
/// stop is requested.
async fn wait_until_on_shift(
    ctx: &mut RuntimeCtx,
    plan: &Plan,
    stop_rx: &mut oneshot::Receiver<()>,
) -> Wait {
    let mut sched_rx = ctx.svc.schedule.subscribe();
    let mut announced = false;
    loop {
        let off = off_shift_now(&ctx.svc.schedule.load(), plan.pattern.as_deref(), &plan.clock);
        if !off {
            if announced {
                info!(npc = %ctx.login, "shift started; connecting");
            }
            return Wait::OnShift;
        }
        if db::tasks::has_open_force_online(&ctx.svc.pool, ctx.id).await.unwrap_or(false) {
            info!(npc = %ctx.login, "manual task while off-shift; connecting");
            return Wait::OnShift;
        }
        if !announced {
            info!(npc = %ctx.login, "off-shift; staying disconnected until shift starts");
            ctx.svc.events.info(ctx.id, "off_shift", "off-shift; disconnected until the next shift", None);
            announced = true;
        }
        tokio::select! {
            biased;
            _ = &mut *stop_rx => return Wait::Stop,
            r = ctx.cfg_rx.changed() => {
                if r.is_err() { return Wait::Stop; }
                return Wait::Changed;
            }
            _ = sched_rx.changed() => return Wait::Changed,
            _ = ctx.tasks.notify.notified() => {}
            _ = tokio::time::sleep(OFF_SHIFT_POLL) => {}
        }
    }
}

/// Sleep `d`, or return `false` early when a stop is requested.
async fn sleep_or_stop(d: Duration, stop_rx: &mut oneshot::Receiver<()>) -> bool {
    tokio::select! {
        biased;
        _ = &mut *stop_rx => false,
        _ = tokio::time::sleep(d) => true,
    }
}

pub async fn run(mut ctx: RuntimeCtx, mut stop_rx: oneshot::Receiver<()>) {
    let cfg = ctx.cfg_rx.borrow_and_update().clone();
    info!(npc = %ctx.login, actions = ?cfg.spec.action_profile, "NPC runtime started");
    let mut plan = Plan::resolve(&ctx, cfg).await;
    let mut backoff = ctx.svc.config.reconnect_backoff_initial_ms;
    let backoff_max = ctx.svc.config.reconnect_backoff_max_ms;

    loop {
        if ctx.cfg_rx.has_changed().unwrap_or(false) {
            let cfg = ctx.cfg_rx.borrow_and_update().clone();
            plan = Plan::resolve(&ctx, cfg).await;
        }

        // If the shift schedule says this NPC is off-shift right now, stay
        // disconnected until its shift starts. This covers both startup (never
        // connect while off) and the return after going home (the session below
        // drops the connection once the NPC is home, then we wait here). NPCs
        // without a schedule are never off-shift and fall straight through.
        match wait_until_on_shift(&mut ctx, &plan, &mut stop_rx).await {
            Wait::OnShift => {}
            Wait::Stop => {
                info!(npc = %ctx.login, "stop requested while off-shift; exiting runtime");
                return;
            }
            Wait::Changed => {
                let cfg = ctx.cfg_rx.borrow_and_update().clone();
                plan = Plan::resolve(&ctx, cfg).await;
                continue;
            }
        }

        // Keycloak access token (cached; logs in again only when needed).
        let token = tokio::select! {
            biased;
            _ = &mut stop_rx => return,
            t = ctx.svc.tokens.token_for(ctx.id, &ctx.login) => t,
        };
        let token = match token {
            Ok(t) => t,
            Err(e) => {
                warn!(npc = %ctx.login, error = %e, backoff_ms = backoff, "no Keycloak token; retrying");
                if !sleep_or_stop(Duration::from_millis(backoff), &mut stop_rx).await {
                    return;
                }
                backoff = backoff.saturating_mul(2).min(backoff_max);
                continue;
            }
        };

        let session = tokio::select! {
            biased;
            _ = &mut stop_rx => {
                info!(npc = %ctx.login, "stop requested before connection");
                return;
            }
            s = HorizonClient::connect(&ctx.svc.config.horizon_url, &token) => s,
        };
        let (sender, mut receiver, my_uuid) = match session {
            Ok(s) => s,
            Err(e) => {
                warn!(npc = %ctx.login, error = %e, backoff_ms = backoff, "Horizon connect failed; retrying");
                if !sleep_or_stop(Duration::from_millis(backoff), &mut stop_rx).await {
                    return;
                }
                backoff = backoff.saturating_mul(2).min(backoff_max);
                continue;
            }
        };
        backoff = ctx.svc.config.reconnect_backoff_initial_ms;

        let world = Arc::new(WorldView::new(my_uuid.clone()));
        // Publish the fresh view to the admin API. Depot behaviour is driven
        // entirely by properties replicated onto the workplace object, so "what
        // does this NPC actually see on its depot" is the first question to ask
        // when an NPC stands idle — and the only way to answer it from outside.
        *ctx.world_slot.write().unwrap() = Some(Arc::clone(&world));
        // Shared per-NPC state (FSM status + blackboard), built fresh per
        // session alongside the WorldView: a reconnect respawns the NPC, so
        // its lifecycle starts clean. An NPC that works spawns in its
        // habitation; `going_to_work` walks it out from there.
        let state = Arc::new(NpcState::new());
        *ctx.state_slot.write().unwrap() = Some(Arc::clone(&state));
        if plan.cfg.spec.place_where_work.is_some() {
            state.set_status(NpcStatus::AtHome);
        }
        ctx.connected.store(true, Ordering::Relaxed);
        ctx.svc.events.info(ctx.id, "connected", "Horizon session established", Some(json!({ "player_id": my_uuid })));
        {
            let pool = ctx.svc.pool.clone();
            let id = ctx.id;
            tokio::spawn(async move {
                let _ = db::npcs::touch_seen(&pool, id).await;
            });
        }
        info!(
            npc = %ctx.login,
            %my_uuid,
            settle_ms = ctx.svc.config.action_startup_delay_ms,
            "Horizon session established; settling before running actions"
        );

        let outcome = session_loop(
            &mut ctx,
            &mut plan,
            Arc::clone(&world),
            Arc::clone(&state),
            sender,
            &mut receiver,
            &mut stop_rx,
        )
        .await;

        ctx.connected.store(false, Ordering::Relaxed);
        *ctx.world_slot.write().unwrap() = None;
        *ctx.current.write().unwrap() = None;

        match outcome {
            SessionOutcome::Stop => {
                info!(npc = %ctx.login, "stop requested; exiting runtime");
                return;
            }
            SessionOutcome::Disconnected => {
                warn!(npc = %ctx.login, "disconnected from Horizon; reconnecting");
                ctx.svc.events.warn(ctx.id, "disconnected", "Horizon session lost; reconnecting", None);
                if !sleep_or_stop(Duration::from_millis(backoff), &mut stop_rx).await {
                    return;
                }
                continue;
            }
            SessionOutcome::OffShift => {
                info!(npc = %ctx.login, "off-shift and home; closing session until next shift");
                ctx.svc.events.info(ctx.id, "disconnected", "off-shift and home; session closed", None);
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

/// The action in flight, if any.
struct Running {
    handle: JoinHandle<()>,
    /// Set when it is a manual task (its id), so it can be cancelled and so
    /// the off-shift cut-off waits for it.
    task_id: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
async fn session_loop(
    ctx: &mut RuntimeCtx,
    plan: &mut Plan,
    world: Arc<WorldView>,
    state: Arc<NpcState>,
    sender: HorizonSender,
    receiver: &mut HorizonReceiver,
    stop_rx: &mut oneshot::Receiver<()>,
) -> SessionOutcome {
    let startup_delay = Duration::from_millis(ctx.svc.config.action_startup_delay_ms);
    let tick = Duration::from_millis(ctx.svc.config.tick_interval_ms);
    // Hold off on the first tick so the NPC settles in before acting. Only the
    // scheduler waits — the loop below keeps applying inbound frames, so the
    // world is already populated when the first action runs.
    let mut ticker = time::interval_at(time::Instant::now() + startup_delay, tick);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    // Actions are rebuilt per session, alongside the WorldView: a fresh
    // connection means a freshly spawned NPC, so per-session action state
    // (e.g. "already went to work") starts clean.
    let mut registry = Arc::new(ActionRegistry::from_spec(&plan.cfg.spec));
    let mut depot_uuid = plan.depot_uuid();
    let my_uuid = plan.cfg.spec.uuid.clone();
    let mut running: Option<Running> = None;
    // Last depot role logged, so only the changes are reported.
    let mut last_depot_role: Option<DepotRole> = None;
    // Last scheduled action started, so only changes become events.
    let mut last_action: Option<&'static str> = None;
    // The assignment is asked for once per session; the manager re-pushes the
    // spec when it is done (or leaves it pending to retry next session).
    let mut assign_requested = false;
    // Edited or imported schedules: the pattern may have appeared or vanished.
    let mut sched_rx = ctx.svc.schedule.subscribe();
    sched_rx.borrow_and_update();

    let outcome = loop {
        tokio::select! {
            biased;
            _ = &mut *stop_rx => break SessionOutcome::Stop,

            maybe_frame = receiver.recv() => {
                match maybe_frame {
                    Some(StandardizedFrame::Gorc(g)) => {
                        let _ = world.apply(&g);
                    }
                    Some(StandardizedFrame::InitAck { player_id }) => {
                        debug!(npc = %ctx.login, %player_id, "stray init_ack");
                    }
                    Some(StandardizedFrame::Error { message }) => {
                        warn!(npc = %ctx.login, %message, "horizon error");
                        break SessionOutcome::Disconnected;
                    }
                    Some(StandardizedFrame::Other { name: ev, .. }) => {
                        debug!(npc = %ctx.login, event = %ev, "ignoring non-GORC frame");
                    }
                    None => break SessionOutcome::Disconnected,
                }
            }

            _ = ticker.tick() => {
                // A spec pushed by the manager (admin edit, assignment): rebuild
                // the actions in place. A running action finishes on the old one.
                if ctx.cfg_rx.has_changed().unwrap_or(false) {
                    let cfg = ctx.cfg_rx.borrow_and_update().clone();
                    *plan = Plan::resolve(ctx, cfg).await;
                    registry = Arc::new(ActionRegistry::from_spec(&plan.cfg.spec));
                    depot_uuid = plan.depot_uuid();
                    info!(npc = %ctx.login, actions = ?plan.cfg.spec.action_profile, "spec reloaded");
                    ctx.svc.events.info(ctx.id, "spec_reloaded", "configuration updated", None);
                } else if sched_rx.has_changed().unwrap_or(false) {
                    sched_rx.borrow_and_update();
                    let cfg = plan.cfg.clone();
                    *plan = Plan::resolve(ctx, cfg).await;
                }

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

                // First spawn of an unassigned NPC: report where it stands so the
                // manager makes it a storekeeper of its village (or a villager).
                if plan.cfg.assign_pending && !assign_requested {
                    if let Some(ev) = first_position_event(ctx.id, &world) {
                        assign_requested = true;
                        info!(npc = %ctx.login, "position known; asking for a role");
                        let _ = ctx.runtime_tx.send(ev);
                    }
                }

                // An admin cancelled the manual task in flight.
                let cancel = ctx.tasks.cancel.lock().unwrap().take();
                if let (Some(c), Some(r)) = (cancel, running.as_ref()) {
                    if r.task_id == Some(c) {
                        r.handle.abort();
                        running = None;
                        *ctx.current.write().unwrap() = None;
                        info!(npc = %ctx.login, task = c, "manual task cancelled");
                        ctx.svc.events.info(ctx.id, "task_cancelled", format!("task {c} cancelled"), Some(json!({ "task_id": c })));
                    }
                }

                if let Some(r) = running.as_ref() {
                    if r.handle.is_finished() {
                        running = None;
                        *ctx.current.write().unwrap() = None;
                    }
                }

                // Off-shift and settled at home: no reason to hold a Horizon
                // session open. `go_home` is schedule-gated to the off-window,
                // so by the time the NPC is `AtHome` its trip home is complete
                // and we never cut it short. Drop the session; the outer loop
                // waits out the off-shift window and reconnects at the next
                // shift. A manual task keeps the session up until it ends.
                let schedule = ctx.svc.schedule.load();
                let manual_running = running.as_ref().is_some_and(|r| r.task_id.is_some());
                if state.status() == NpcStatus::AtHome
                    && !manual_running
                    && !ctx.tasks.pending.load(Ordering::SeqCst)
                    && off_shift_now(&schedule, plan.pattern.as_deref(), &plan.clock)
                {
                    break SessionOutcome::OffShift;
                }

                if running.is_some() {
                    continue;
                }

                // Which actions does the shift schedule permit this planetary
                // hour? `None` when the NPC has no schedule (no gating).
                let allowed = plan
                    .pattern
                    .as_deref()
                    .and_then(|p| schedule.allowed_actions_at(p, plan.clock.now()));

                // Cargo-depot role this tick: prefer the authoritative
                // `reception_owner` claim replicated on the depot; fall back to
                // the deterministic election when it is not replicated at all —
                // or when the claim is stale (see `is_off_shift_worker`).
                let depot_role = match depot_uuid.as_deref() {
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
                                ctx.svc.workforce.is_off_shift_worker(d, o, &schedule, plan.clock.now())
                            });
                        match crate::depot::role_from_owner(owner.as_ref(), &my_uuid)
                            .filter(|_| !stale)
                        {
                            Some(r) => r,
                            None => ctx.svc.workforce.role_for(d, &my_uuid, &schedule, plan.clock.now()),
                        }
                    }
                    None => DepotRole::Uncoordinated,
                };
                // The role decides whether the NPC may touch the reception at
                // all, so a silent flip is the hardest depot stall to diagnose.
                if last_depot_role != Some(depot_role) {
                    info!(npc = %ctx.login, role = ?depot_role, "depot role");
                    last_depot_role = Some(depot_role);
                }

                let make_ctx = |allowed| ActionContext {
                    npc_name: ctx.login.clone(),
                    world: Arc::clone(&world),
                    state: Arc::clone(&state),
                    send: sender.clone(),
                    rng: StdRng::from_entropy(),
                    allowed_actions: allowed,
                    depot_role,
                };

                // Manual tasks first: they pre-empt the schedule.
                if ctx.tasks.pending.load(Ordering::SeqCst) {
                    match db::tasks::claim_next(&ctx.svc.pool, ctx.id).await {
                        Ok(Some(task)) => {
                            info!(npc = %ctx.login, task = task.id, kind = %task.kind, "starting manual task");
                            ctx.svc.events.info(
                                ctx.id,
                                "task_started",
                                format!("task {} ({}) started", task.id, task.kind),
                                Some(json!({ "task_id": task.id, "kind": task.kind, "params": task.params.0 })),
                            );
                            *ctx.current.write().unwrap() = Some(CurrentAction {
                                name: manual::label(&task),
                                source: "manual",
                                task_id: Some(task.id),
                                started_at: OffsetDateTime::now_utc(),
                            });
                            let task_id = task.id;
                            let action_ctx = make_ctx(None);
                            let spec = Arc::clone(&plan.cfg.spec);
                            let pool = ctx.svc.pool.clone();
                            let events = ctx.svc.events.clone();
                            let npc_id = ctx.id;
                            let h = tokio::spawn(
                                async move {
                                    manual::execute(task, action_ctx, spec, pool, events, npc_id).await;
                                }
                                .in_current_span(),
                            );
                            running = Some(Running { handle: h, task_id: Some(task_id) });
                            continue;
                        }
                        Ok(None) => ctx.tasks.pending.store(false, Ordering::SeqCst),
                        Err(e) => warn!(npc = %ctx.login, error = %e, "cannot read manual tasks"),
                    }
                }

                let probe = make_ctx(allowed.clone());
                let Some(action) = registry.pick(&probe) else {
                    continue;
                };

                let action_name = action.name();
                debug!(npc = %ctx.login, action = %action_name, "starting action");
                if last_action != Some(action_name) {
                    last_action = Some(action_name);
                    ctx.svc.events.emit(Some(ctx.id), "action_started", "debug", action_name, None);
                }
                *ctx.current.write().unwrap() = Some(CurrentAction {
                    name: action_name.to_string(),
                    source: "schedule",
                    task_id: None,
                    started_at: OffsetDateTime::now_utc(),
                });

                // Stamp the start before spawning so the `once`/`min_interval`
                // gates see this run on the very next tick.
                registry.mark_started(action_name);
                let run_timeout = registry.timing_of(action_name).and_then(|t| t.timeout);

                let login = ctx.login.clone();
                let registry_for_task = Arc::clone(&registry);
                let mut action_ctx = make_ctx(allowed);
                let events = ctx.svc.events.clone();
                let npc_id = ctx.id;
                let h = tokio::spawn(
                    async move {
                        let result = match run_timeout {
                            Some(d) => match time::timeout(d, action.run(&mut action_ctx)).await {
                                Ok(r) => r,
                                Err(_) => {
                                    warn!(
                                        npc = %login,
                                        action = %action_name,
                                        timeout_ms = d.as_millis(),
                                        "action timed out; aborting run"
                                    );
                                    events.warn(npc_id, "action_timeout", action_name, Some(json!({ "timeout_ms": d.as_millis() as u64 })));
                                    Ok(())
                                }
                            },
                            None => action.run(&mut action_ctx).await,
                        };
                        if let Err(e) = result {
                            warn!(npc = %login, action = %action_name, error = %e, "action failed");
                            events.warn(npc_id, "action_failed", format!("{action_name}: {e}"), None);
                        }
                        // Stamp completion so the cooldown gate measures from here.
                        registry_for_task.mark_finished(action_name);
                    }
                    .in_current_span(),
                );
                running = Some(Running { handle: h, task_id: None });
            }
        }
    };

    if let Some(r) = running.take() {
        r.handle.abort();
        if let Some(task_id) = r.task_id {
            let reason = match outcome {
                SessionOutcome::Stop => "NPC stopped",
                _ => "Horizon session lost",
            };
            let _ = db::tasks::finish(&ctx.svc.pool, task_id, "failed", Some(reason)).await;
            ctx.svc.events.warn(ctx.id, "task_failed", format!("task {task_id}: {reason}"), Some(json!({ "task_id": task_id })));
        }
    }
    outcome
}

/// The assignment request for an NPC whose planet-frame position resolves,
/// with the cargo_depots it sees as fallback workplaces.
fn first_position_event(id: Uuid, world: &WorldView) -> Option<RuntimeEvent> {
    let wd = WorldData::shared()?;
    let radius = wd.radius_m();
    let planet_pos = driving::my_planet_position(world, radius)?;
    let depots = world
        .objects
        .iter()
        .filter(|o| o.object_type == "cargo_depot")
        .filter_map(|o| {
            let uuid = Uuid::parse_str(&o.uuid).ok()?;
            let pos = match &o.parent_id {
                Some(parent) if !parent.is_empty() => {
                    driving::object_point_to_planet(world, parent, o.position, radius)?
                }
                _ => o.position,
            };
            Some(DepotCandidate { uuid, scenename: o.scenename.clone(), planet_pos: pos })
        })
        .collect();
    Some(RuntimeEvent::FirstPosition { id, planet_pos, depots })
}
