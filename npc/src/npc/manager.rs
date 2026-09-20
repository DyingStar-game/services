use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, oneshot};
use tracing::{info, warn};

use serde::Serialize;

use crate::{
    clock::{ClockParams, PlanetClock},
    config::Config,
    depot::DepotWorkforce,
    error::{NpcError, Result},
    npc::{
        names::NamePool,
        runtime::{self, NpcHandle},
        spec::{NpcSpec, RosterFile},
    },
    resources_dynamic::PlanetCatalogHandle,
    schedule::Schedule,
};

/// A snapshot of one NPC's planetary time and the activity its schedule assigns
/// right now — for the admin API, to eyeball the rota as the day drifts.
#[derive(Debug, Serialize)]
pub struct NpcScheduleReport {
    pub name: String,
    pub planet: String,
    pub rotation_h: f64,
    pub schedule: Option<String>,
    pub day: u32,
    pub hour: u32,
    /// Scheduled activity (`work`/`transmission`/`meal`/`mandatory`/`off`), or
    /// `null` when the NPC has no schedule.
    pub activity: Option<String>,
}

/// Owns every live NPC task and the name pool. Cloned via `Arc` everywhere.
pub struct NpcManager {
    config: Arc<Config>,
    names: Arc<NamePool>,
    clock_params: Arc<ClockParams>,
    planets: PlanetCatalogHandle,
    schedule: Arc<Schedule>,
    /// Shared roster index for cargo-depot coordination (deterministic fallback
    /// election). Maintained on spawn/despawn, read-only from each runtime.
    workforce: Arc<DepotWorkforce>,
    npcs: Mutex<HashMap<String, NpcHandle>>,
}

impl NpcManager {
    pub fn new(
        config: Arc<Config>,
        clock_params: Arc<ClockParams>,
        planets: PlanetCatalogHandle,
        schedule: Arc<Schedule>,
    ) -> Self {
        let names = match NamePool::from_file(&config.names_path) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                warn!(error = %e, path = %config.names_path, "names file unreadable; using empty pool");
                Arc::new(NamePool::new(Vec::new()))
            }
        };
        Self {
            config,
            names,
            clock_params,
            planets,
            schedule,
            workforce: Arc::new(DepotWorkforce::new()),
            npcs: Mutex::new(HashMap::new()),
        }
    }

    /// Load `config/npcs.yaml` and spawn every NPC it lists.
    pub async fn load_roster_file(&self, path: &str) -> Result<()> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!(%path, "no roster file; starting empty");
                return Ok(());
            }
            Err(e) => return Err(NpcError::Io(e)),
        };
        let roster: RosterFile = serde_yaml::from_str(&raw)?;
        info!(count = roster.npcs.len(), %path, "loaded roster");
        for spec in roster.npcs {
            if let Err(e) = self.spawn(spec).await {
                warn!(error = %e, "failed to spawn NPC from roster");
            }
        }
        Ok(())
    }

    /// Spawn a new NPC. Returns the assigned name.
    pub async fn spawn(&self, mut spec: NpcSpec) -> Result<String> {
        let name = match spec.name.clone() {
            Some(n) => n,
            None => {
                let n = self.names.take();
                spec.name = Some(n.clone());
                n
            }
        };

        let mut npcs = self.npcs.lock().await;
        if npcs.contains_key(&name) {
            return Err(NpcError::AlreadyExists(name));
        }

        // Register in the depot workforce before spawning so the fallback
        // election sees this NPC as soon as its runtime starts ticking.
        self.workforce.insert_spec(&spec);

        let (stop_tx, stop_rx) = oneshot::channel();
        let cfg = Arc::clone(&self.config);
        let clock_params = Arc::clone(&self.clock_params);
        let planets = self.planets.clone();
        let schedule = Arc::clone(&self.schedule);
        let workforce = Arc::clone(&self.workforce);
        let name_for_task = name.clone();
        let spec_for_task = spec.clone();
        // Slot the runtime publishes its live WorldView into, so the admin API
        // can report what this NPC actually sees (see `world_status`).
        let world: runtime::WorldSlot = Arc::new(std::sync::RwLock::new(None));
        let world_for_task = Arc::clone(&world);
        let state: runtime::StateSlot = Arc::new(std::sync::RwLock::new(None));
        let state_for_task = Arc::clone(&state);
        let join = tokio::spawn(async move {
            runtime::run(
                cfg,
                clock_params,
                planets,
                schedule,
                workforce,
                name_for_task,
                spec_for_task,
                world_for_task,
                state_for_task,
                stop_rx,
            )
            .await;
        });

        npcs.insert(
            name.clone(),
            NpcHandle {
                name: name.clone(),
                spec,
                stop_tx,
                join,
                world,
                state,
            },
        );
        info!(npc = %name, "spawned NPC");
        Ok(name)
    }

    /// Stop and remove an NPC by name.
    pub async fn despawn(&self, name: &str) -> Result<()> {
        let handle = {
            let mut npcs = self.npcs.lock().await;
            npcs.remove(name)
                .ok_or_else(|| NpcError::NotFound(name.to_string()))?
        };
        let _ = handle.stop_tx.send(());
        let _ = handle.join.await;
        self.workforce.remove_uuid(&handle.spec.uuid);
        self.names.release(handle.name);
        info!(npc = %name, "despawned NPC");
        Ok(())
    }

    /// List every live NPC's spec.
    pub async fn list(&self) -> Vec<NpcSpec> {
        self.npcs
            .lock()
            .await
            .values()
            .map(|h| h.spec.clone())
            .collect()
    }

    /// Current planetary time and scheduled activity for every live NPC.
    /// What each NPC currently sees on its workplace object: the properties the
    /// depot actions gate on, plus whether the object is in view at all.
    ///
    /// Every depot behaviour is driven by properties replicated onto the depot
    /// (`pnj_zones`, `pnj_reception_deposited_item`, `pnj_box_temp_zones`,
    /// `reception_owner`). When an NPC stands idle, the question is always which
    /// of those is missing — and nothing else exposes it, because the WorldView
    /// lives inside the runtime task.
    pub async fn world_status(&self) -> Vec<serde_json::Value> {
        let npcs = self.npcs.lock().await;
        let mut out = Vec::with_capacity(npcs.len());
        for handle in npcs.values() {
            let depot = handle.spec.place_where_work.as_ref().map(|p| p.uuid.clone());
            // The FSM status answers the other half of "why is this NPC idle":
            // one stuck mid-trip (`GoingToWork`, waiting on an `npc_arrived`
            // that never came) is indistinguishable from one with nothing to do.
            let status = handle
                .state
                .read()
                .unwrap()
                .as_ref()
                .map(|s| format!("{:?}", s.status()));
            let world = handle.world.read().unwrap().clone();
            let Some(world) = world else {
                // Between sessions: off shift, or reconnecting.
                out.push(serde_json::json!({
                    "name": handle.name,
                    "connected": false,
                    "status": status,
                    "work_uuid": depot,
                }));
                continue;
            };
            let depot_view = depot.as_ref().and_then(|d| world.objects.get(d)).map(|o| {
                serde_json::json!({
                    "object_type": o.object_type,
                    "in_view": true,
                    // The full property set, so a missing key is as visible as a
                    // wrong one. Depots carry a handful of keys; this stays small.
                    "props": o.props,
                })
            });
            out.push(serde_json::json!({
                "name": handle.name,
                "connected": true,
                "my_uuid": world.my_uuid,
                "status": status,
                "work_uuid": depot,
                // Where the NPC actually stands, and in whose frame. An NPC that
                // never left its habitation reports the habitation as parent.
                "my_parent": world.my_parent(),
                "my_position": world.my_position(),
                // The server-side flag the travel actions wait on.
                "npc_arrived": world.npc_arrived(),
                "objects_in_view": world.objects.len(),
                "players_in_view": world.players.len(),
                "depot": depot_view.unwrap_or(serde_json::json!({"in_view": false})),
            }));
        }
        out
    }

    pub async fn schedule_status(&self) -> Vec<NpcScheduleReport> {
        let specs: Vec<NpcSpec> = self.list().await;
        let mut out = Vec::with_capacity(specs.len());
        for spec in specs {
            let planet_key = spec
                .place_where_work
                .as_ref()
                .and_then(|p| p.planet.clone())
                .unwrap_or_else(|| self.config.planet_internal_name.clone());
            let rotation_h = self
                .planets
                .rotation_h_or(&planet_key, self.config.rotation_h_fallback)
                .await;
            let t = PlanetClock::new(Arc::clone(&self.clock_params), rotation_h).now();
            let activity = spec
                .schedule
                .as_ref()
                .and_then(|p| self.schedule.activity_at(p, t))
                .map(|a| format!("{a:?}").to_lowercase());
            out.push(NpcScheduleReport {
                name: spec.name.unwrap_or_default(),
                planet: planet_key,
                rotation_h,
                schedule: spec.schedule,
                day: t.day,
                hour: t.hour,
                activity,
            });
        }
        out
    }
}
