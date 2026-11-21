//! Owns every live NPC runtime and keeps them in line with the database: an NPC
//! runs when the global switch is on and its own `enabled` is set. Admin writes
//! go to the database and then [`NpcManager::poke`] the reconciler, which
//! spawns, despawns and hot-pushes changed specs to running NPCs.

use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::future::join_all;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::{Mutex, Notify, mpsc, oneshot, watch};
use tracing::{Instrument, info, info_span, warn};
use uuid::Uuid;

use crate::{
    clock::{ClockParams, PlanetClock},
    config::Config,
    crypto::PasswordBox,
    db::{self, npcs::NpcRow},
    depot::DepotWorkforce,
    error::{NpcError, Result},
    events::EventSink,
    horizon::messages::Vec3,
    keycloak::{KcAdmin, TokenManager},
    logs::{LogHub, NPC_SPAN},
    npc::{
        assign,
        names::NameGenerator,
        runtime::{self, RuntimeCtx},
        spec::NpcSpec,
    },
    resources_dynamic::PlanetCatalogHandle,
    roles::RoleTemplates,
    schedule::ScheduleStore,
    usim::Usim,
    world::view::WorldView,
};

/// Safety net: re-read the database this often even without a poke.
const RECONCILE_EVERY: Duration = Duration::from_secs(60);
/// How often the warehouses of the Mining villages are re-read from
/// universesimulator (and how soon after a failed read).
const WORKPLACES_EVERY: Duration = Duration::from_secs(600);
const WORKPLACES_RETRY: Duration = Duration::from_secs(60);

/// What every runtime shares.
pub struct Services {
    pub config: Arc<Config>,
    pub pool: PgPool,
    pub clock_params: Arc<ClockParams>,
    pub planets: PlanetCatalogHandle,
    pub schedule: Arc<ScheduleStore>,
    pub workforce: Arc<DepotWorkforce>,
    pub tokens: Arc<TokenManager>,
    pub events: EventSink,
}

impl Services {
    /// The planet key of a spec (its workplace's, else the service default).
    pub fn planet_of(&self, spec: &NpcSpec) -> String {
        spec.place_where_work
            .as_ref()
            .and_then(|p| p.planet.clone())
            .unwrap_or_else(|| self.config.planet_internal_name.clone())
    }

    pub async fn clock_for(&self, spec: &NpcSpec) -> (String, f64, PlanetClock) {
        let planet = self.planet_of(spec);
        let rotation_h = self.planets.rotation_h_or(&planet, self.config.rotation_h_fallback).await;
        (planet, rotation_h, PlanetClock::new(Arc::clone(&self.clock_params), rotation_h))
    }
}

/// What a runtime runs on, pushed again whenever the row changes.
#[derive(Clone)]
pub struct NpcConfig {
    pub spec: Arc<NpcSpec>,
    /// The automatic storekeeper/villager assignment is still owed.
    pub assign_pending: bool,
    /// `updated_at` of the row this was built from.
    pub version: OffsetDateTime,
}

impl NpcConfig {
    fn from_row(row: &NpcRow) -> Result<Self> {
        Ok(Self {
            spec: Arc::new(row.to_spec()?),
            assign_pending: row.assignment_pending(),
            version: row.updated_at,
        })
    }
}

/// Manual tasks waiting for a runtime: a flag (so the runtime only queries the
/// database when there is something to claim) and a cancel request.
#[derive(Default)]
pub struct TaskSignal {
    pub pending: AtomicBool,
    pub cancel: std::sync::Mutex<Option<i64>>,
    pub notify: Notify,
}

impl TaskSignal {
    pub fn kick(&self) {
        self.pending.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
}

/// The action an NPC is running right now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentAction {
    pub name: String,
    /// `schedule` or `manual`.
    pub source: &'static str,
    pub task_id: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

pub type CurrentSlot = Arc<RwLock<Option<CurrentAction>>>;

/// Shared slot holding an NPC's live [`WorldView`], published by its runtime on
/// every session and read by the admin API. `None` between sessions.
pub type WorldSlot = Arc<RwLock<Option<Arc<WorldView>>>>;

/// Shared slot holding an NPC's live [`NpcState`](crate::npc::state::NpcState).
pub type StateSlot = Arc<RwLock<Option<Arc<crate::npc::state::NpcState>>>>;

pub struct NpcHandle {
    pub id: Uuid,
    pub login: String,
    cfg_tx: watch::Sender<NpcConfig>,
    stop_tx: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
    pub world: WorldSlot,
    pub state: StateSlot,
    pub current: CurrentSlot,
    pub tasks: Arc<TaskSignal>,
    pub connected: Arc<AtomicBool>,
    pub started_at: OffsetDateTime,
}

/// Sent by runtimes to the manager.
pub enum RuntimeEvent {
    /// The NPC's position is known for the first time this session and its
    /// assignment is owed.
    FirstPosition { id: Uuid, planet_pos: Vec3, depots: Vec<DepotCandidate> },
}

/// A cargo_depot the NPC sees, as a fallback workplace.
#[derive(Debug, Clone)]
pub struct DepotCandidate {
    pub uuid: Uuid,
    pub scenename: Option<String>,
    pub planet_pos: Vec3,
}

/// Live status of one NPC for the admin API.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    pub running: bool,
    pub connected: bool,
    /// FSM status (`at_work`, `going_home`…) while connected.
    pub status: Option<String>,
    pub current_action: Option<CurrentAction>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub running_since: Option<OffsetDateTime>,
}

impl LiveStatus {
    fn stopped() -> Self {
        Self { running: false, connected: false, status: None, current_action: None, running_since: None }
    }
}

/// Planetary time and routine of one NPC right now.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub pattern: Option<String>,
    /// The pattern is set but unknown (deleted, never imported).
    pub pattern_missing: bool,
    pub planet: String,
    pub rotation_h: f64,
    pub day: u32,
    pub hour: u32,
    pub activity: Option<String>,
    /// Actions the schedule allows now; `null` = no gating.
    pub allowed_actions: Option<Vec<String>>,
    /// The pattern grid, one line per rota-day.
    pub days: Option<Vec<String>>,
}

/// Counts for the admin panel header.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: usize,
    pub enabled: usize,
    pub running: usize,
    pub connected: usize,
}

pub struct NpcManager {
    pub svc: Arc<Services>,
    pub kc: Arc<KcAdmin>,
    pub usim: Option<Arc<Usim>>,
    pub roles: Arc<RoleTemplates>,
    pub names: NameGenerator,
    pub passwords: PasswordBox,
    pub logs: Arc<LogHub>,
    npcs: Mutex<HashMap<Uuid, NpcHandle>>,
    reconcile: Notify,
    revision: AtomicU64,
    runtime_tx: mpsc::UnboundedSender<RuntimeEvent>,
    runtime_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<RuntimeEvent>>>,
}

impl NpcManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        svc: Arc<Services>,
        kc: Arc<KcAdmin>,
        usim: Option<Arc<Usim>>,
        roles: Arc<RoleTemplates>,
        names: NameGenerator,
        passwords: PasswordBox,
        logs: Arc<LogHub>,
    ) -> Arc<Self> {
        let (runtime_tx, runtime_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            svc,
            kc,
            usim,
            roles,
            names,
            passwords,
            logs,
            npcs: Mutex::new(HashMap::new()),
            reconcile: Notify::new(),
            revision: AtomicU64::new(0),
            runtime_tx,
            runtime_rx: std::sync::Mutex::new(Some(runtime_rx)),
        })
    }

    pub fn pool(&self) -> &PgPool {
        &self.svc.pool
    }

    pub fn events(&self) -> &EventSink {
        &self.svc.events
    }

    /// Start the reconciler and the runtime-event handler.
    pub fn start(self: &Arc<Self>) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                if let Err(e) = mgr.reconcile_once().await {
                    warn!(error = %e, "reconcile pass failed");
                }
                tokio::select! {
                    _ = mgr.reconcile.notified() => {}
                    _ = tokio::time::sleep(RECONCILE_EVERY) => {}
                }
            }
        });

        if self.usim.is_some() {
            let mgr = Arc::clone(self);
            tokio::spawn(async move {
                let mut first = true;
                loop {
                    let wait = match assign::resolve_all_workplaces(&mgr).await {
                        Ok(sweep) => {
                            if first || sweep.written > 0 {
                                info!(
                                    written = sweep.written,
                                    without_warehouse = sweep.missing,
                                    "village workplaces read from universesimulator"
                                );
                            }
                            first = false;
                            WORKPLACES_EVERY
                        }
                        Err(e) => {
                            warn!(error = %e, "cannot read village workplaces from universesimulator");
                            WORKPLACES_RETRY
                        }
                    };
                    tokio::time::sleep(wait).await;
                }
            });
        }

        let mut rx = self.runtime_rx.lock().unwrap().take().expect("manager started twice");
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let mgr = Arc::clone(&mgr);
                tokio::spawn(async move {
                    match ev {
                        RuntimeEvent::FirstPosition { id, planet_pos, depots } => {
                            if let Err(e) = assign::assign(&mgr, id, planet_pos, &depots).await {
                                warn!(npc_id = %id, error = %e, "assignment failed");
                                mgr.events().warn(id, "assign_failed", e.to_string(), None);
                            }
                        }
                    }
                });
            }
        });
    }

    /// Ask the reconciler for a pass (after any write).
    pub fn poke(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.reconcile.notify_one();
    }

    /// Bring the live NPCs in line with the database. A poke during the
    /// staggered spawn ramp restarts the pass, so turning things off never
    /// waits for a ramp to finish.
    async fn reconcile_once(&self) -> Result<()> {
        let stagger = Duration::from_millis(self.svc.config.spawn_stagger_ms);
        'pass: loop {
            let revision = self.revision.load(Ordering::SeqCst);
            let settings = db::settings::get(self.pool()).await?;
            let rows = db::npcs::all(self.pool()).await?;
            let desired: Vec<&NpcRow> = if settings.npcs_enabled {
                rows.iter().filter(|r| r.enabled).collect()
            } else {
                Vec::new()
            };
            let wanted: HashMap<Uuid, &NpcRow> = desired.iter().map(|r| (r.id, *r)).collect();

            // Despawn what is no longer wanted.
            let extra: Vec<Uuid> = {
                let npcs = self.npcs.lock().await;
                npcs.keys().filter(|id| !wanted.contains_key(id)).copied().collect()
            };
            join_all(extra.iter().map(|id| self.despawn(*id, "disabled"))).await;

            // Hot-push changed rows to the running ones.
            {
                let npcs = self.npcs.lock().await;
                for (id, handle) in npcs.iter() {
                    let Some(row) = wanted.get(id) else { continue };
                    if handle.cfg_tx.borrow().version == row.updated_at {
                        continue;
                    }
                    match NpcConfig::from_row(row) {
                        Ok(cfg) => {
                            self.svc.workforce.insert_spec(&cfg.spec);
                            let _ = handle.cfg_tx.send(cfg);
                        }
                        Err(e) => warn!(npc_id = %id, error = %e, "invalid NPC configuration; keeping the old one"),
                    }
                }
            }

            // Spawn the missing ones, oldest first.
            let running: Vec<Uuid> = self.npcs.lock().await.keys().copied().collect();
            let mut first = true;
            for row in desired.iter().filter(|r| !running.contains(&r.id)) {
                if !first {
                    tokio::time::sleep(stagger).await;
                }
                first = false;
                if self.revision.load(Ordering::SeqCst) != revision {
                    continue 'pass;
                }
                if let Err(e) = self.spawn(row).await {
                    warn!(npc_id = %row.id, error = %e, "failed to spawn NPC");
                    self.events().warn(row.id, "spawn_failed", e.to_string(), None);
                }
            }
            return Ok(());
        }
    }

    async fn spawn(&self, row: &NpcRow) -> Result<()> {
        let cfg = NpcConfig::from_row(row)?;
        let mut npcs = self.npcs.lock().await;
        if npcs.contains_key(&row.id) {
            return Err(NpcError::AlreadyExists(row.login.clone()));
        }
        // Register in the depot workforce before spawning so the fallback
        // election sees this NPC as soon as its runtime starts ticking.
        self.svc.workforce.insert_spec(&cfg.spec);

        let (cfg_tx, cfg_rx) = watch::channel(cfg);
        let (stop_tx, stop_rx) = oneshot::channel();
        let handle_world: WorldSlot = Arc::new(RwLock::new(None));
        let handle_state: StateSlot = Arc::new(RwLock::new(None));
        let current: CurrentSlot = Arc::new(RwLock::new(None));
        let tasks = Arc::new(TaskSignal::default());
        // Tasks queued while the NPC was not running.
        tasks.pending.store(true, Ordering::SeqCst);
        let connected = Arc::new(AtomicBool::new(false));

        let ctx = RuntimeCtx {
            svc: Arc::clone(&self.svc),
            id: row.id,
            login: row.login.clone(),
            cfg_rx,
            world_slot: Arc::clone(&handle_world),
            state_slot: Arc::clone(&handle_state),
            current: Arc::clone(&current),
            tasks: Arc::clone(&tasks),
            connected: Arc::clone(&connected),
            runtime_tx: self.runtime_tx.clone(),
        };
        // Every log line of this NPC (and of the actions it spawns) is filed
        // in its ring buffer through this span (see `logs`).
        let span = info_span!(NPC_SPAN, id = %row.id, name = %row.display_name);
        let join = tokio::spawn(runtime::run(ctx, stop_rx).instrument(span));

        npcs.insert(
            row.id,
            NpcHandle {
                id: row.id,
                login: row.login.clone(),
                cfg_tx,
                stop_tx,
                join,
                world: handle_world,
                state: handle_state,
                current,
                tasks,
                connected,
                started_at: OffsetDateTime::now_utc(),
            },
        );
        info!(npc = %row.display_name, login = %row.login, "spawned NPC");
        self.events().info(row.id, "spawned", "runtime started", None);
        Ok(())
    }

    /// Stop an NPC's runtime (closing its Horizon session). No-op when it is
    /// not running.
    async fn despawn(&self, id: Uuid, reason: &str) {
        let Some(handle) = self.npcs.lock().await.remove(&id) else { return };
        let _ = handle.stop_tx.send(());
        let _ = handle.join.await;
        self.svc.workforce.remove_uuid(&id.to_string());
        self.svc.tokens.forget(id).await;
        info!(login = %handle.login, reason, "despawned NPC");
        self.events().info(id, "despawned", format!("runtime stopped ({reason})"), None);
    }

    /// Delete an NPC for good: runtime, Keycloak user, row, logs.
    pub async fn delete_npc(&self, id: Uuid) -> Result<()> {
        let row = db::npcs::require(self.pool(), id).await?;
        self.despawn(id, "deleted").await;
        self.kc.delete_user(id).await?;
        db::npcs::delete(self.pool(), id).await?;
        self.logs.remove(&id.to_string());
        info!(login = %row.login, "deleted NPC");
        self.events().global("npc_deleted", format!("{} ({}) deleted", row.display_name, row.login), Some(json!({ "id": id })));
        Ok(())
    }

    pub async fn live(&self, id: Uuid) -> LiveStatus {
        let npcs = self.npcs.lock().await;
        npcs.get(&id).map(live_of).unwrap_or_else(LiveStatus::stopped)
    }

    pub async fn live_all(&self) -> HashMap<Uuid, LiveStatus> {
        self.npcs.lock().await.iter().map(|(id, h)| (*id, live_of(h))).collect()
    }

    pub async fn counts(&self) -> Result<Counts> {
        let (total, enabled): (i64, i64) =
            sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE enabled) FROM npcs")
                .fetch_one(self.pool())
                .await?;
        let npcs = self.npcs.lock().await;
        Ok(Counts {
            total: total as usize,
            enabled: enabled as usize,
            running: npcs.len(),
            connected: npcs.values().filter(|h| h.connected.load(Ordering::Relaxed)).count(),
        })
    }

    /// Planetary time and routine of `row` right now.
    pub async fn routine(&self, row: &NpcRow) -> Result<Routine> {
        let spec = row.to_spec()?;
        let (planet, rotation_h, clock) = self.svc.clock_for(&spec).await;
        let t = clock.now();
        let schedule = self.svc.schedule.load();
        let pattern = spec.schedule.clone();
        let known = pattern.as_deref().is_some_and(|p| schedule.has_pattern(p));
        let days = match pattern.as_deref() {
            Some(p) => db::patterns::get(self.pool(), p).await?.map(|p| p.days),
            None => None,
        };
        let allowed = pattern.as_deref().and_then(|p| schedule.allowed_actions_at(p, t)).map(|s| {
            let mut v: Vec<String> = s.into_iter().collect();
            v.sort();
            v
        });
        Ok(Routine {
            pattern_missing: pattern.is_some() && !known,
            activity: pattern
                .as_deref()
                .and_then(|p| schedule.activity_at(p, t))
                .map(|a| a.key().to_string()),
            pattern,
            planet,
            rotation_h,
            day: t.day,
            hour: t.hour,
            allowed_actions: allowed,
            days,
        })
    }

    /// Wake an NPC's runtime about a new task (no-op when not running: the
    /// task is picked up at its next spawn).
    pub async fn kick_tasks(&self, id: Uuid) {
        if let Some(h) = self.npcs.lock().await.get(&id) {
            h.tasks.kick();
        }
    }

    /// Ask a running NPC to abort task `task_id` if it is running it.
    pub async fn cancel_task(&self, id: Uuid, task_id: i64) {
        if let Some(h) = self.npcs.lock().await.get(&id) {
            *h.tasks.cancel.lock().unwrap() = Some(task_id);
            h.tasks.notify.notify_one();
        }
    }

    /// What each running NPC sees on its workplace object (debug view): the
    /// properties the depot actions gate on, its position and FSM status.
    pub async fn world_status(&self) -> Vec<Value> {
        let npcs = self.npcs.lock().await;
        let mut out = Vec::with_capacity(npcs.len());
        for h in npcs.values() {
            let spec = Arc::clone(&h.cfg_tx.borrow().spec);
            let depot = spec.place_where_work.as_ref().map(|p| p.uuid.clone());
            let status = h.state.read().unwrap().as_ref().map(|s| format!("{:?}", s.status()));
            let trip = h.state.read().unwrap().as_ref().and_then(|s| s.get("drive_to_factory"));
            let world = h.world.read().unwrap().clone();
            let Some(world) = world else {
                out.push(json!({
                    "id": h.id, "login": h.login, "name": spec.name, "connected": false,
                    "status": status, "work_uuid": depot, "drive_to_factory": trip,
                }));
                continue;
            };
            let depot_view = depot.as_ref().and_then(|d| world.objects.get(d)).map(|o| {
                json!({ "object_type": o.object_type, "in_view": true, "props": o.props })
            });
            out.push(json!({
                "id": h.id,
                "login": h.login,
                "name": spec.name,
                "connected": true,
                "my_uuid": world.my_uuid,
                "status": status,
                "work_uuid": depot,
                "my_parent": world.my_parent(),
                "my_position": world.my_position(),
                "npc_arrived": world.npc_arrived(),
                "objects_in_view": world.objects.len(),
                "players_in_view": world.players.len(),
                "depot": depot_view.unwrap_or(json!({"in_view": false})),
                "drive_to_factory": trip,
            }));
        }
        out
    }

    /// The live FSM state (status + blackboard) of a running NPC.
    pub async fn state_snapshot(&self, id: Uuid) -> Option<Value> {
        let npcs = self.npcs.lock().await;
        let h = npcs.get(&id)?;
        let state = h.state.read().unwrap().clone()?;
        let world = h.world.read().unwrap().clone();
        Some(json!({
            "state": state.snapshot(),
            "position": world.as_ref().and_then(|w| w.my_position()),
            "parent": world.as_ref().and_then(|w| w.my_parent()),
        }))
    }

    /// Stop every runtime (shutdown).
    pub async fn shutdown(&self) {
        let ids: Vec<Uuid> = self.npcs.lock().await.keys().copied().collect();
        join_all(ids.iter().map(|id| self.despawn(*id, "shutdown"))).await;
    }
}

fn live_of(h: &NpcHandle) -> LiveStatus {
    LiveStatus {
        running: true,
        connected: h.connected.load(Ordering::Relaxed),
        status: h
            .state
            .read()
            .unwrap()
            .as_ref()
            .and_then(|s| serde_json::to_value(s.status()).ok())
            .and_then(|v| v.as_str().map(str::to_string)),
        current_action: h.current.read().unwrap().clone(),
        running_since: Some(h.started_at),
    }
}
