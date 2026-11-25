//! What a long-running service needs around the model: one body's model kept in memory and
//! advanced one chunk at a time under the roll-back guard, the latest tables rendered in memory,
//! and two rolling checkpoints written in turn into a `StateStore` (files here; PostgreSQL in the
//! weather service).
//!
//! The pacing (how fast model time follows the wall clock) is the caller's business: this module
//! never looks at the clock, so it stays testable.

use crate::io::{EncodedTable, Sampler};
use crate::outputs;
use crate::pe::{ChunkOutcome, Guard, PeDiag, PeModel, PeState, PeWork};
use crate::setup::{Init, Scenario};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One body's model, its state and everything needed to advance it.
pub struct LiveModel {
    pub sc: Scenario,
    pub model: PeModel,
    pub st: PeState,
    pub work: PeWork,
    pub guard: Guard,
    /// Model relief per cell (m), for the sea-level pressure of the tables.
    pub relief: Vec<f64>,
    smp: Sampler,
    /// False until the first chunk since the start or the last resume: the physics has not yet
    /// left its diagnostics (the "ingredients") in the work buffers.
    stepped: bool,
}

impl LiveModel {
    /// Builds the model of a scenario and its initial state (equilibrium of the relaxation, at
    /// rest, plus the small noise of the settings): a cold start.
    pub fn cold_start(sc: Scenario) -> Result<LiveModel, String> {
        let setup = sc.build(Init::Equilibrium)?;
        let smp = Sampler::new(&setup.model.grid, sc.s.run.img_w, sc.s.run.img_h);
        let work = setup.model.new_work();
        let guard = Guard::new(&setup.state);
        Ok(LiveModel { sc, model: setup.model, st: setup.state, work, guard, relief: setup.relief, smp, stepped: false })
    }

    /// Replaces the state (from a checkpoint); the guard's last good state becomes this one.
    pub fn resume_from(&mut self, st: PeState) -> Result<(), String> {
        self.model.check(&st)?;
        self.guard = Guard::new(&st);
        self.st = st;
        self.stepped = false;
        Ok(())
    }

    /// Length of a chunk (s of model time).
    pub fn chunk_s(&self) -> f64 {
        self.sc.chunk_s()
    }

    /// Model day (s): the body's solar day (NOTES.md of the prototype, section 39).
    pub fn day_s(&self) -> f64 {
        self.sc.body.day_s
    }

    /// Model time in model days.
    pub fn model_day(&self) -> f64 {
        self.st.time / self.day_s()
    }

    /// Advances one chunk under the guard. `Ok(Done)`: the new state passed the checks (finite
    /// values, pressure and wind bounds). `Ok(RolledBack)`: the state went back to the last model-day
    /// boundary with a smaller time step. `Err`: the third failure in a row, the model stops.
    pub fn step_chunk(&mut self) -> Result<ChunkOutcome, String> {
        let s = &self.sc.s;
        self.stepped = true;
        self.model.advance_guarded(&mut self.st, &mut self.guard, self.sc.chunk_s(), s.numerics.courant, s.numerics.chunks_per_day, &mut self.work)
    }

    pub fn diag(&self) -> PeDiag {
        self.model.diag(&self.st)
    }

    /// Every table of the current instant, encoded (16-bit PNG + JSON sidecar). The physics
    /// "ingredients" come from the last step's work buffers: before the first chunk (after a start
    /// or a resume) only the tables of the state itself are given (pressures, winds, temperatures).
    pub fn tables(&self) -> std::io::Result<Vec<(&'static str, EncodedTable)>> {
        let work = if self.stepped { Some(&self.work) } else { None };
        outputs::render_tables(&self.model, &self.st, &self.relief, &self.smp, work, self.day_s())
    }
}

/// What a checkpoint holds, besides the model state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckpointMeta {
    pub body: String,
    /// Number of completed chunks and model time (s) of the saved state.
    pub chunk: u64,
    pub model_time_s: f64,
    /// Wall-clock time (Unix seconds) when it was saved.
    pub written_unix: f64,
    /// True when the guard accepted the state (only healthy states are loaded back).
    pub healthy: bool,
    /// True once the spin-up is over (the model then follows the wall clock).
    pub spun_up: bool,
    /// Pacing anchor: the model time that corresponds to the wall-clock time `anchor_unix`.
    pub anchor_model_s: f64,
    pub anchor_unix: f64,
}

/// A saved checkpoint: the model state as bytes (`PeModel::restart_bytes`) and what it holds.
pub struct StoredState {
    pub bytes: Vec<u8>,
    pub meta: CheckpointMeta,
    /// The slot it was saved in.
    pub slot: usize,
    /// Where it lives, for the logs (a file path, a table row...).
    pub location: String,
}

/// Where checkpoints are kept. Each body has a few slots (two rolling ones are used) so that a
/// failure while saving one never loses the other.
///
/// Implementations: `FileStateStore` here (one folder per body), and the PostgreSQL store of the
/// weather service (`weather-service/src/pg_store.rs`, one row per body and slot), kept out of this
/// crate so that the model needs no database driver. The service only talks to this trait.
pub trait StateStore: Send {
    /// Saves a checkpoint of `body` in `slot` (replacing what the slot held), atomically: either
    /// the old or the new checkpoint is readable afterwards, never a mix. Returns its location.
    fn save(&mut self, body: &str, slot: usize, bytes: &[u8], meta: &CheckpointMeta) -> std::io::Result<String>;

    /// The healthy checkpoints of `body`, newest first (by chunk). The caller takes the first
    /// one its model accepts, so a damaged newest checkpoint falls back to the older one.
    fn load_latest_healthy(&self, body: &str) -> std::io::Result<Vec<StoredState>>;
}

/// Checkpoints as files: `<dir>/<body in lower case>/checkpoint_<a|b>.bin` (the state) and
/// `checkpoint_<a|b>.toml` (its `CheckpointMeta`). Each file is written to a temporary name and
/// renamed; the state first, then its sidecar. A sidecar whose chunk does not match its state is
/// skipped by `RollingCheckpoints`.
pub struct FileStateStore {
    pub dir: PathBuf,
}

const SLOTS: [&str; 2] = ["a", "b"];

impl FileStateStore {
    pub fn new(dir: &Path) -> FileStateStore {
        FileStateStore { dir: dir.to_path_buf() }
    }

    fn body_dir(&self, body: &str) -> PathBuf {
        self.dir.join(body.to_lowercase().replace(' ', "_"))
    }

    fn paths(&self, body: &str, slot: usize) -> (PathBuf, PathBuf) {
        let d = self.body_dir(body);
        let s = SLOTS[slot % SLOTS.len()];
        (d.join(format!("checkpoint_{s}.bin")), d.join(format!("checkpoint_{s}.toml")))
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

impl StateStore for FileStateStore {
    fn save(&mut self, body: &str, slot: usize, bytes: &[u8], meta: &CheckpointMeta) -> std::io::Result<String> {
        std::fs::create_dir_all(self.body_dir(body))?;
        let (bin, side) = self.paths(body, slot);
        write_atomic(&bin, bytes)?;
        write_atomic(&side, toml::to_string(meta).map_err(std::io::Error::other)?.as_bytes())?;
        Ok(bin.display().to_string())
    }

    fn load_latest_healthy(&self, body: &str) -> std::io::Result<Vec<StoredState>> {
        let mut out = Vec::new();
        for slot in 0..SLOTS.len() {
            let (bin, side) = self.paths(body, slot);
            let Ok(txt) = std::fs::read_to_string(&side) else { continue };
            let Ok(meta) = toml::from_str::<CheckpointMeta>(&txt) else { continue };
            if !meta.healthy {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&bin) {
                out.push(StoredState { bytes, meta, slot, location: bin.display().to_string() });
            }
        }
        out.sort_by(|a, b| b.meta.chunk.cmp(&a.meta.chunk));
        Ok(out)
    }
}

/// Two rolling checkpoints of one body in a `StateStore`, written in turn (MITgcm-style "pickup"
/// files): the next save goes to the slot that does NOT hold the newest checkpoint.
pub struct RollingCheckpoints {
    pub store: Box<dyn StateStore>,
    pub body: String,
    next: usize,
}

impl RollingCheckpoints {
    pub fn new(store: Box<dyn StateStore>, body: &str) -> RollingCheckpoints {
        RollingCheckpoints { store, body: body.to_string(), next: 0 }
    }

    /// The newest checkpoint that reads back for this model (same grid and levels), matches its
    /// metadata and passes the guard's checks. Returns the state, its metadata and the reasons the
    /// other checkpoints were skipped.
    pub fn latest_healthy(&mut self, model: &PeModel) -> (Option<(PeState, CheckpointMeta)>, Vec<String>) {
        let mut skipped = Vec::new();
        let found = match self.store.load_latest_healthy(&self.body) {
            Ok(f) => f,
            Err(e) => return (None, vec![format!("state store: {e}")]),
        };
        let mut result = None;
        for s in found {
            match model.state_from_bytes(&s.bytes) {
                Ok(st) if st.chunk != s.meta.chunk => skipped.push(format!("{}: holds chunk {}, its metadata says {}", s.location, st.chunk, s.meta.chunk)),
                Ok(st) => match model.check(&st) {
                    Ok(()) => {
                        result = Some((st, s.meta, s.slot % 2));
                        break;
                    }
                    Err(e) => skipped.push(format!("{}: refused by the guard: {e}", s.location)),
                },
                Err(e) => skipped.push(format!("{}: {e}", s.location)),
            }
        }
        // Never overwrite the checkpoint we resume from: write next into the other slot.
        if let Some((_, _, slot)) = &result {
            self.next = 1 - *slot;
        }
        (result.map(|(st, meta, _)| (st, meta)), skipped)
    }

    /// Saves a state the guard has just accepted into the next slot, then alternates. Returns
    /// the checkpoint's location.
    pub fn save(&mut self, model: &PeModel, st: &PeState, meta: &CheckpointMeta) -> std::io::Result<String> {
        let loc = self.store.save(&self.body, self.next, &model.restart_bytes(st), meta)?;
        self.next = 1 - self.next;
        Ok(loc)
    }
}

