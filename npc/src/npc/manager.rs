use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, oneshot};
use tracing::{info, warn};

use crate::{
    config::Config,
    error::{NpcError, Result},
    npc::{
        names::NamePool,
        runtime::{self, NpcHandle},
        spec::{NpcSpec, RosterFile},
    },
};

/// Owns every live NPC task and the name pool. Cloned via `Arc` everywhere.
pub struct NpcManager {
    config: Arc<Config>,
    names: Arc<NamePool>,
    npcs: Mutex<HashMap<String, NpcHandle>>,
}

impl NpcManager {
    pub fn new(config: Arc<Config>) -> Self {
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

        let (stop_tx, stop_rx) = oneshot::channel();
        let cfg = Arc::clone(&self.config);
        let name_for_task = name.clone();
        let spec_for_task = spec.clone();
        let join = tokio::spawn(async move {
            runtime::run(cfg, name_for_task, spec_for_task, stop_rx).await;
        });

        npcs.insert(
            name.clone(),
            NpcHandle {
                name: name.clone(),
                spec,
                stop_tx,
                join,
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
}
