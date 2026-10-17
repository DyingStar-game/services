use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use futures_util::future::join_all;
use tokio::sync::{Mutex, Notify, oneshot};
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

/// Roster counts for the admin API.
#[derive(Debug, Serialize)]
pub struct RosterStatus {
    /// Entries in the roster file.
    pub total: usize,
    /// How many of the first roster entries should be live.
    pub target: usize,
    /// Roster NPCs live right now. Trails `target` while a staggered ramp runs.
    pub running: usize,
}

/// The YAML roster, in file order, and how many of its first entries should be
/// live. Ad-hoc NPCs (`POST /npcs`) are not part of it.
struct Roster {
    specs: Vec<NpcSpec>,
    /// Desired live count. `None` = every entry, including ones a reload adds.
    target: Option<usize>,
    /// Bumped on every change, so a reconcile pass notices it went stale.
    revision: u64,
}

impl Roster {
    fn target(&self) -> usize {
        self.target.map_or(self.specs.len(), |n| n.min(self.specs.len()))
    }

    /// Target after adding `delta`, clamped to `[0, specs.len()]`.
    fn scaled(&self, delta: i64) -> usize {
        (self.target() as i64)
            .saturating_add(delta)
            .clamp(0, self.specs.len() as i64) as usize
    }
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
    roster: Mutex<Roster>,
    /// Wakes the reconciler (see `spawn_reconciler`) after a roster change.
    reconcile: Notify,
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
            names,
            clock_params,
            planets,
            schedule,
            workforce: Arc::new(DepotWorkforce::new()),
            npcs: Mutex::new(HashMap::new()),
            roster: Mutex::new(Roster {
                specs: Vec::new(),
                target: config.npc_active_count,
                revision: 0,
            }),
            reconcile: Notify::new(),
            config,
        }
    }

    /// (Re)load the roster file. Live roster NPCs whose UUID left the file are
    /// despawned; the rest is up to the reconciler. NPCs whose entry changed
    /// keep running on their old spec until they are despawned and respawned.
    pub async fn reload_roster(&self) -> Result<RosterStatus> {
        let path = &self.config.roster_path;
        let specs = match std::fs::read_to_string(path) {
            Ok(raw) => serde_yaml::from_str::<RosterFile>(&raw)?.npcs,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!(%path, "no roster file; roster is empty");
                Vec::new()
            }
            Err(e) => return Err(NpcError::Io(e)),
        };
        info!(count = specs.len(), %path, "loaded roster");

        let new_uuids: HashSet<&str> = specs.iter().map(|s| s.uuid.as_str()).collect();
        let dropped: HashSet<String> = {
            let mut roster = self.roster.lock().await;
            let dropped = roster
                .specs
                .iter()
                .filter(|s| !new_uuids.contains(s.uuid.as_str()))
                .map(|s| s.uuid.clone())
                .collect();
            roster.specs = specs;
            roster.revision += 1;
            dropped
        };
        if !dropped.is_empty() {
            let running = self.running_by_uuid().await;
            let names = dropped.iter().filter_map(|u| running.get(u));
            self.despawn_all(names).await;
        }
        self.reconcile.notify_one();
        Ok(self.roster_status().await)
    }

    /// Run the first `count` roster entries (`None` = all of them).
    pub async fn set_roster_target(&self, count: Option<usize>) -> RosterStatus {
        {
            let mut roster = self.roster.lock().await;
            roster.target = count;
            roster.revision += 1;
        }
        self.reconcile.notify_one();
        self.roster_status().await
    }

    /// Run `delta` more (or, negative, fewer) roster entries than the target.
    pub async fn scale_roster(&self, delta: i64) -> RosterStatus {
        {
            let mut roster = self.roster.lock().await;
            roster.target = Some(roster.scaled(delta));
            roster.revision += 1;
        }
        self.reconcile.notify_one();
        self.roster_status().await
    }

    pub async fn roster_status(&self) -> RosterStatus {
        let (total, target, uuids) = {
            let roster = self.roster.lock().await;
            let uuids: HashSet<String> = roster.specs.iter().map(|s| s.uuid.clone()).collect();
            (roster.specs.len(), roster.target(), uuids)
        };
        let running = self
            .npcs
            .lock()
            .await
            .values()
            .filter(|h| uuids.contains(&h.spec.uuid))
            .count();
        RosterStatus { total, target, running }
    }

    /// Start the background task that keeps the live roster NPCs in line with
    /// the target. Changes made before it starts are picked up on its first run.
    pub fn spawn_reconciler(self: &Arc<Self>) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                mgr.reconcile.notified().await;
                mgr.reconcile_roster().await;
            }
        });
    }

    /// Despawn every live roster NPC past the target, then spawn the missing
    /// ones in file order, `spawn_stagger_ms` apart. A roster change mid-ramp
    /// restarts the pass, so scaling down doesn't wait for a ramp to finish.
    async fn reconcile_roster(&self) {
        let stagger = Duration::from_millis(self.config.spawn_stagger_ms);
        'pass: loop {
            let (specs, target, revision) = {
                let roster = self.roster.lock().await;
                (roster.specs.clone(), roster.target(), roster.revision)
            };
            let running = self.running_by_uuid().await;

            // Last in, first out: the excess comes off the end of the roster.
            let excess = specs[target..].iter().rev().filter_map(|s| running.get(&s.uuid));
            self.despawn_all(excess).await;

            let mut first = true;
            for spec in &specs[..target] {
                if running.contains_key(&spec.uuid) {
                    continue;
                }
                if !first {
                    tokio::time::sleep(stagger).await;
                }
                first = false;
                if self.roster.lock().await.revision != revision {
                    continue 'pass;
                }
                if let Err(e) = self.spawn(spec.clone()).await {
                    warn!(error = %e, uuid = %spec.uuid, "failed to spawn NPC from roster");
                }
            }
            return;
        }
    }

    /// Live NPCs by UUID → name.
    async fn running_by_uuid(&self) -> HashMap<String, String> {
        self.npcs
            .lock()
            .await
            .values()
            .map(|h| (h.spec.uuid.clone(), h.name.clone()))
            .collect()
    }

    /// Despawn several NPCs concurrently, logging (not failing on) errors.
    async fn despawn_all<'a>(&self, names: impl IntoIterator<Item = &'a String>) {
        let results = join_all(names.into_iter().map(|n| async move { (n, self.despawn(n).await) })).await;
        for (name, res) in results {
            if let Err(e) = res {
                warn!(error = %e, npc = %name, "failed to despawn roster NPC");
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn roster(len: usize, target: Option<usize>) -> Roster {
        let specs = (0..len)
            .map(|i| serde_yaml::from_str(&format!("uuid: \"u{i}\"")).unwrap())
            .collect();
        Roster { specs, target, revision: 0 }
    }

    #[test]
    fn target_is_capped_by_roster_size() {
        assert_eq!(roster(5, None).target(), 5);
        assert_eq!(roster(5, Some(3)).target(), 3);
        assert_eq!(roster(5, Some(50)).target(), 5);
    }

    #[test]
    fn scaled_clamps_to_roster_bounds() {
        let r = roster(10, Some(4));
        assert_eq!(r.scaled(3), 7);
        assert_eq!(r.scaled(-3), 1);
        assert_eq!(r.scaled(50), 10);
        assert_eq!(r.scaled(-50), 0);
        assert_eq!(roster(10, None).scaled(-4), 6);
    }

    fn write_roster(path: &std::path::Path, count: usize) {
        let mut yaml = String::from("npcs:\n");
        for i in 0..count {
            yaml.push_str(&format!("  - name: npc{i}\n    uuid: \"u{i}\"\n"));
        }
        std::fs::write(path, yaml).unwrap();
    }

    async fn live_names(mgr: &NpcManager) -> Vec<String> {
        let mut names: Vec<String> = mgr.npcs.lock().await.keys().cloned().collect();
        names.sort();
        names
    }

    // Paused time: the stagger sleeps resolve instantly. Horizon points at a
    // closed local port, so the runtimes just sit in their reconnect backoff.
    #[tokio::test(start_paused = true)]
    async fn reconcile_follows_target_and_reload() {
        let dir = std::env::temp_dir().join(format!("npc-roster-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("npcs.yaml");
        write_roster(&path, 5);

        let mut config = Config::from_env();
        config.horizon_url = "ws://127.0.0.1:1".into();
        config.roster_path = path.to_string_lossy().into_owned();
        config.npc_active_count = Some(2);
        config.spawn_stagger_ms = 100;
        let mgr = NpcManager::new(
            Arc::new(config),
            Arc::new(ClockParams::new(5, 0, None, 1.0)),
            PlanetCatalogHandle::empty(),
            Arc::new(Schedule::empty()),
        );

        mgr.reload_roster().await.unwrap();
        mgr.reconcile_roster().await;
        assert_eq!(live_names(&mgr).await, ["npc0", "npc1"]);

        mgr.scale_roster(2).await;
        mgr.reconcile_roster().await;
        assert_eq!(live_names(&mgr).await, ["npc0", "npc1", "npc2", "npc3"]);

        mgr.scale_roster(-3).await;
        mgr.reconcile_roster().await;
        assert_eq!(live_names(&mgr).await, ["npc0"]);

        mgr.set_roster_target(None).await;
        mgr.reconcile_roster().await;
        assert_eq!(mgr.roster_status().await.running, 5);

        // Entries dropped from the file are despawned on reload.
        write_roster(&path, 3);
        let status = mgr.reload_roster().await.unwrap();
        assert_eq!((status.total, status.target, status.running), (3, 3, 3));
        assert_eq!(live_names(&mgr).await, ["npc0", "npc1", "npc2"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
