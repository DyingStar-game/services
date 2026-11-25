//! The stateful atmosphere model (crate `weather-atmosphere`), OFF by default.
//!
//! For each body listed in `ATMOSPHERE_BODIES`, a dedicated thread (the model is CPU-bound and
//! never awaits) keeps the body's model in memory:
//!
//! 0. the state store is PostgreSQL when `DATABASE_URL` is set (`pg_store.rs`), else files under
//!    `ATMOSPHERE_STATE_DIR`; with neither, the atmosphere mode does not start (status `failed`);
//! 1. it loads the newest healthy checkpoint from the state store, or makes a cold start and runs
//!    the spin-up as fast as it can (status `warming`);
//! 2. then it follows the wall clock: one second of real time = one second of model time, so one
//!    model day = the body's solar day. After a stop it catches up in accelerated time (status
//!    `catching_up`), at most `ATMOSPHERE_MAX_CATCHUP_DAYS` model days;
//! 3. after every chunk the guard accepted, it renders the tables (16-bit PNG) into memory, and at
//!    most every `ATMOSPHERE_CHECKPOINT_S` seconds it saves a checkpoint into one of two rolling
//!    slots; a roll-back of the guard is logged and counted, a third failure in a row stops the
//!    model (status `failed`) until the next restart, which resumes from the last healthy checkpoint.
//!
//! The synoptic generator, its ds_bridge publication and every other endpoint are untouched: the
//! model runs beside them and is only visible through `/api/v1/{body}/atmosphere` and the metrics.
//!
//! The sky is the real one: model time and Unix time are tied by an offset fixed at the cold start
//! (`unix = model time + offset`, saved in every checkpoint), and the model's declination,
//! sub-stellar longitude and distance to the star come from the body's orbit and spin at that Unix
//! time (`weather_atmosphere::ephemeris`, built on `weather_core::orbit` and `spin` like the rest of
//! the service). A cold start spins up over the period that ends at the real instant; a resume
//! catches up to the real instant, or makes a cold start when the lag is longer than a spin-up.

use crate::config::AtmosphereConfig;
use crate::sim::now_unix;
use crate::state::{Body as ServiceBody, Shared};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use weather_atmosphere::body::{Body, BodyOverrides};
use weather_atmosphere::ephemeris::Ephemeris;
use crate::pg_store::PgStateStore;
use std::path::PathBuf;
use weather_atmosphere::live::{CheckpointMeta, FileStateStore, LiveModel, RollingCheckpoints, StateStore};
use weather_atmosphere::pe::ChunkOutcome;
use weather_atmosphere::settings::Settings;
use weather_atmosphere::setup::Scenario;

/// What the HTTP side can read about one body's run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RunStatus {
    /// `starting`, `warming`, `catching_up`, `running` or `failed`.
    pub status: String,
    pub error: Option<String>,
    /// `cold start` or the checkpoint it resumed from.
    pub started_from: Option<String>,
    pub settings_file: String,
    pub grid_level: u32,
    pub layers: usize,
    pub cells: usize,
    /// Model day (s) = the body's solar day; chunk = the unit the model advances by.
    pub day_s: f64,
    pub chunk_s: f64,
    pub spinup_days: f64,
    pub chunk: u64,
    pub model_time_s: f64,
    pub model_day: f64,
    /// How far behind the wall clock the model is (model seconds; 0 when in pace).
    pub lag_s: f64,
    pub max_wind_m_s: f64,
    pub min_ps_hpa: f64,
    pub max_ps_hpa: f64,
    pub min_t_k: f64,
    pub max_t_k: f64,
    pub rollbacks: u64,
    /// The guard's last events (roll-backs, final stop), newest last.
    pub guard_events: Vec<String>,
    pub last_checkpoint: Option<String>,
    pub last_checkpoint_unix: Option<f64>,
}

/// One rendered table.
pub struct TableEntry {
    pub png: Vec<u8>,
    pub json: String,
    pub min: f64,
    pub max: f64,
}

/// The latest tables of a body, all from the same model instant.
#[derive(Default)]
pub struct TableSet {
    pub model_time_s: f64,
    pub model_day: f64,
    pub rendered_unix: f64,
    pub tables: BTreeMap<String, TableEntry>,
}

/// One body's run of the model.
pub struct AtmosphereRun {
    pub body_index: usize,
    pub name: String,
    pub status: RwLock<RunStatus>,
    pub tables: RwLock<TableSet>,
}

/// The runs to start, from the configuration: bodies that exist, have an air and a weather.
pub fn plan(cfg: &AtmosphereConfig, bodies: &[ServiceBody]) -> HashMap<String, Arc<AtmosphereRun>> {
    let mut out = HashMap::new();
    for wanted in &cfg.bodies {
        let Some(i) = bodies.iter().position(|b| b.inputs.name.eq_ignore_ascii_case(wanted)) else {
            warn!(body = %wanted, "ATMOSPHERE_BODIES: no such body, ignored");
            continue;
        };
        let b = &bodies[i];
        if !b.climate.has_atmosphere || !b.climate.extras.weather_enabled {
            warn!(body = %b.inputs.name, "ATMOSPHERE_BODIES: body without air or with weather disabled, ignored");
            continue;
        }
        let status = RunStatus { status: "starting".into(), ..Default::default() };
        out.insert(b.inputs.name.to_lowercase(), Arc::new(AtmosphereRun { body_index: i, name: b.inputs.name.clone(), status: RwLock::new(status), tables: RwLock::new(TableSet::default()) }));
    }
    out
}

/// Where the checkpoints go.
#[derive(Clone)]
pub enum StoreChoice {
    Postgres(PgStateStore),
    Files(PathBuf),
}

impl StoreChoice {
    fn make(&self) -> Box<dyn StateStore> {
        match self {
            StoreChoice::Postgres(s) => Box::new(s.clone()),
            StoreChoice::Files(d) => Box::new(FileStateStore::new(d)),
        }
    }

    fn describe(&self) -> String {
        match self {
            StoreChoice::Postgres(_) => "PostgreSQL (table atmosphere_checkpoint)".into(),
            StoreChoice::Files(d) => format!("files under {}", d.display()),
        }
    }
}

fn fail_all(state: &Shared, msg: String) {
    error!("atmosphere mode not started: {msg}");
    for run in state.atmosphere.values() {
        set_status(run, |s| {
            s.status = "failed".into();
            s.error = Some(msg.clone());
        });
    }
}

/// Picks the state store (`DATABASE_URL` → PostgreSQL, else `ATMOSPHERE_STATE_DIR` → files,
/// else refuse), then starts one thread per planned body. Runs on the tokio runtime; the rest of
/// the service is already up and does not wait for it.
pub async fn start(state: Shared) {
    let cfg = &state.config.atmosphere;
    let choice = match (&cfg.database_url, &cfg.state_dir) {
        (Some(url), _) => match PgStateStore::connect(&url.0).await {
            Ok(s) => StoreChoice::Postgres(s),
            Err(e) => return fail_all(&state, format!("PostgreSQL (DATABASE_URL) unusable: {e}")),
        },
        (None, Some(dir)) => StoreChoice::Files(dir.clone()),
        (None, None) => {
            return fail_all(&state, "no place for the checkpoints: set DATABASE_URL (PostgreSQL) or ATMOSPHERE_STATE_DIR (files)".into());
        }
    };
    info!(store = %choice.describe(), "atmosphere state store");
    spawn_all(&state, choice);
}

/// Starts one thread per planned body.
fn spawn_all(state: &Shared, choice: StoreChoice) {
    for run in state.atmosphere.values() {
        let (st, r, store) = (state.clone(), run.clone(), choice.make());
        let spawned = std::thread::Builder::new().name(format!("atmosphere-{}", r.name)).spawn(move || {
            if let Err(e) = run_body(&st, &r, store) {
                error!(body = %r.name, error = %e, "atmosphere model stopped");
                if let Ok(mut s) = r.status.write() {
                    s.status = "failed".into();
                    s.error = Some(e);
                }
            }
        });
        if let Err(e) = spawned {
            error!(body = %run.name, error = %e, "cannot start the atmosphere thread");
        }
    }
}

fn run_body(state: &Shared, run: &AtmosphereRun, store: Box<dyn StateStore>) -> Result<(), String> {
    let cfg = &state.config.atmosphere;
    let b = &state.bodies[run.body_index];
    // A moon's seasons and solar day follow its planet's year.
    let planet = &state.bodies[b.star_orbit_of];
    let planet_year = planet.inputs.orbit.solver(planet.inputs.mass_kg).period_s();
    let body = Body::from_derived(b.climate.clone(), b.inputs.kind.clone(), Some(planet_year), &BodyOverrides::default())?;
    let mut sc = Scenario::from_body(body, &cfg.data_dir, &cfg.sets)?;
    if let Some(d) = cfg.spinup_days {
        sc.s.run.spinup_days = d;
    }
    let settings_file = Settings::file_for_body(&cfg.data_dir, &run.name).display().to_string();
    // The real sky: the body's tilt and spin, the orbit around the star (its planet's for a moon),
    // exactly what `state.to_star` and the synoptic generator use.
    let (tilt_rad, rotation_period_s) = (b.inputs.tilt_deg.to_radians(), b.inputs.rotation_period_s);
    let star_orbit = planet.inputs.orbit.solver(planet.inputs.mass_kg);
    let ephemeris = move |unix_offset_s: f64| Arc::new(Ephemeris { unix_offset_s, tilt_rad, rotation_period_s, star_orbit: star_orbit.clone() });
    info!(body = %run.name, settings = %settings_file, threads = cfg.threads, "atmosphere model: building");
    weather_atmosphere::with_threads(cfg.threads, || drive(state, run, sc, &ephemeris, settings_file, store))
}

/// Unix-time offset of a cold start at Unix time `now`: the spin-up covers the `spinup_s` seconds
/// BEFORE now, with the real seasons and days of that period, so that the warm model lands on
/// the real instant (plus the wall-clock time the spin-up itself took, caught up afterwards).
pub fn cold_start_offset(now_unix: f64, spinup_s: f64) -> f64 {
    now_unix - spinup_s
}

/// How far behind the wall clock a checkpoint is once resumed (model seconds), not counting the
/// spin-up it still has to run: what a catch-up would have to compute beyond the spin-up.
pub fn catchup_needed_s(meta: &CheckpointMeta, now_unix: f64, spinup_s: f64) -> f64 {
    (now_unix - meta.unix_offset_s) - meta.model_time_s.max(spinup_s)
}

/// The model time the model should have reached at Unix time `now`.
pub fn pacing_target(now_unix: f64, unix_offset_s: f64) -> f64 {
    now_unix - unix_offset_s
}

fn set_status(run: &AtmosphereRun, f: impl FnOnce(&mut RunStatus)) {
    if let Ok(mut s) = run.status.write() {
        f(&mut s);
    }
}

fn drive(state: &Shared, run: &AtmosphereRun, mut sc: Scenario, ephemeris: &dyn Fn(f64) -> Arc<Ephemeris>, settings_file: String, store: Box<dyn StateStore>) -> Result<(), String> {
    let cfg = &state.config.atmosphere;
    let label = [run.name.as_str()];
    let day_s = sc.body.day_s;
    let chunk_s = sc.chunk_s();
    let cpd = sc.chunks_per_day();
    let spinup_days = sc.s.run.spinup_days;
    let spinup_chunks = sc.s.chunks(spinup_days);
    let spinup_s = spinup_chunks as f64 * chunk_s;
    let max_lag = cfg.max_catchup_days.unwrap_or(spinup_days) * day_s;
    // Built for a cold start now; the restart files do not depend on the offset, so this model
    // also reads the checkpoints back.
    let now = now_unix();
    let cold_offset = cold_start_offset(now, spinup_s);
    sc.ephemeris = Some(ephemeris(cold_offset));
    let cold = LiveModel::cold_start(sc.clone())?;
    let mut ck = RollingCheckpoints::new(store, &run.name);
    let (found, skipped) = ck.latest_healthy(&cold.model);
    for s in skipped {
        warn!(body = %run.name, "checkpoint skipped: {s}");
    }
    let cold_meta = CheckpointMeta { body: run.name.clone(), chunk: 0, model_time_s: 0.0, written_unix: 0.0, healthy: true, spun_up: false, unix_offset_s: cold_offset };
    let (mut m, mut meta, started_from) = match found {
        Some((st, meta)) if catchup_needed_s(&meta, now, spinup_s) <= max_lag => {
            // Same sky as when it was saved: rebuild with the checkpoint's own offset.
            sc.ephemeris = Some(ephemeris(meta.unix_offset_s));
            let mut m = LiveModel::cold_start(sc.clone())?;
            m.resume_from(st)?;
            let from = format!("checkpoint of model day {:.3} (written at Unix {:.0})", meta.model_time_s / day_s, meta.written_unix);
            (m, meta, from)
        }
        Some((_, meta)) => {
            let behind = catchup_needed_s(&meta, now, spinup_s) / day_s;
            warn!(body = %run.name, behind_days = behind, "atmosphere checkpoint too far behind the real time: cold start instead of catching up");
            (cold, cold_meta, format!("cold start (the checkpoint was {behind:.1} model days behind)"))
        }
        None => (cold, cold_meta, "cold start".to_string()),
    };
    info!(body = %run.name, from = %started_from, unix_offset_s = meta.unix_offset_s, "atmosphere model: ready");
    set_status(run, |s| {
        s.started_from = Some(started_from);
        s.settings_file = settings_file;
        s.grid_level = m.model.grid.level;
        s.layers = m.model.nl;
        s.cells = m.model.grid.n_cells();
        s.day_s = day_s;
        s.chunk_s = chunk_s;
        s.spinup_days = spinup_days;
    });
    let mut last_ckpt = Instant::now();
    let mut have_tables = false;
    if m.st.chunk > 0 {
        // Resumed: show the saved instant at once (state tables only until the next chunk).
        have_tables = publish_tables(run, &m);
        let d = m.diag();
        set_status(run, |s| {
            s.status = if meta.spun_up { "running" } else { "warming" }.into();
            s.chunk = m.st.chunk;
            s.model_time_s = m.st.time;
            s.model_day = m.model_day();
            s.max_wind_m_s = d.max_speed;
            s.min_ps_hpa = d.min_ps / 100.0;
            s.max_ps_hpa = d.max_ps / 100.0;
            s.min_t_k = d.min_t;
            s.max_t_k = d.max_t;
        });
    }
    loop {
        let now = now_unix();
        if !meta.spun_up && m.st.chunk >= spinup_chunks {
            meta.spun_up = true;
            info!(body = %run.name, model_day = m.model_day(), "atmosphere model: spin-up done, now following the wall clock");
            // The warm state is worth keeping at once; the tables of this instant too.
            if m.st.chunk > 0 {
                publish_tables(run, &m);
                save_checkpoint(run, &mut ck, &m, &mut meta);
                last_ckpt = Instant::now();
            }
        }
        let mut lag = 0.0;
        if meta.spun_up {
            // Never skipped: the offset ties model time to the real sky, so the model always
            // catches up to the real instant (a too long lag was turned into a cold start above).
            let target = pacing_target(now, meta.unix_offset_s);
            lag = (target - m.st.time).max(0.0);
            let next_end = (m.st.chunk + 1) as f64 * chunk_s;
            if next_end > target {
                // In pace: wait for the wall clock to reach the end of the next chunk.
                state.metrics.atmo_lag_s.with_label_values(&label).set(0.0);
                set_status(run, |s| {
                    s.status = "running".into();
                    s.lag_s = 0.0;
                });
                std::thread::sleep(Duration::from_secs_f64((next_end - target).clamp(0.05, 5.0)));
                continue;
            }
        }
        match m.step_chunk() {
            Ok(ChunkOutcome::Done(_)) => {
                let d = m.diag();
                // While warming, tables once per model day (rendering costs about a chunk).
                if meta.spun_up || m.st.chunk % cpd == 0 || !have_tables {
                    have_tables |= publish_tables(run, &m);
                }
                if last_ckpt.elapsed().as_secs_f64() >= cfg.checkpoint_every_s {
                    save_checkpoint(run, &mut ck, &m, &mut meta);
                    last_ckpt = Instant::now();
                }
                let status = if !meta.spun_up {
                    "warming"
                } else if lag > 2.0 * chunk_s {
                    "catching_up"
                } else {
                    "running"
                };
                state.metrics.atmo_model_day.with_label_values(&label).set(m.model_day());
                state.metrics.atmo_max_wind.with_label_values(&label).set(d.max_speed);
                state.metrics.atmo_lag_s.with_label_values(&label).set(lag);
                let events = m.guard.events.clone();
                set_status(run, |s| {
                    s.status = status.into();
                    s.chunk = m.st.chunk;
                    s.model_time_s = m.st.time;
                    s.model_day = m.model_day();
                    s.lag_s = lag;
                    s.max_wind_m_s = d.max_speed;
                    s.min_ps_hpa = d.min_ps / 100.0;
                    s.max_ps_hpa = d.max_ps / 100.0;
                    s.min_t_k = d.min_t;
                    s.max_t_k = d.max_t;
                    s.guard_events = events.iter().rev().take(20).rev().cloned().collect();
                });
            }
            Ok(ChunkOutcome::RolledBack(msg)) => {
                warn!(body = %run.name, "atmosphere guard: {msg}");
                state.metrics.atmo_rollbacks.with_label_values(&label).inc();
                set_status(run, |s| {
                    s.rollbacks += 1;
                    s.guard_events.push(msg);
                });
            }
            Err(e) => {
                set_status(run, |s| s.guard_events.push(e.clone()));
                return Err(format!("the guard stopped the model: {e}"));
            }
        }
    }
}

/// Renders the tables of the current instant into memory. True when done.
fn publish_tables(run: &AtmosphereRun, m: &LiveModel) -> bool {
    match m.tables() {
        Ok(list) => {
            let mut set = TableSet { model_time_s: m.st.time, model_day: m.model_day(), rendered_unix: now_unix(), tables: BTreeMap::new() };
            for (name, t) in list {
                set.tables.insert(name.to_string(), TableEntry { png: t.png, json: t.json, min: t.min, max: t.max });
            }
            if let Ok(mut w) = run.tables.write() {
                *w = set;
            }
            true
        }
        Err(e) => {
            warn!(body = %run.name, error = %e, "atmosphere tables not rendered");
            false
        }
    }
}

/// Saves the current state (which the guard has just accepted) into the next rolling slot.
fn save_checkpoint(run: &AtmosphereRun, ck: &mut RollingCheckpoints, m: &LiveModel, meta: &mut CheckpointMeta) {
    meta.chunk = m.st.chunk;
    meta.model_time_s = m.st.time;
    meta.written_unix = now_unix();
    meta.healthy = true;
    match ck.save(&m.model, &m.st, meta) {
        Ok(loc) => {
            let when = meta.written_unix;
            set_status(run, |s| {
                s.last_checkpoint = Some(loc);
                s.last_checkpoint_unix = Some(when);
            });
        }
        Err(e) => warn!(body = %run.name, error = %e, "atmosphere checkpoint not saved"),
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;

    fn meta(model_time_s: f64, unix_offset_s: f64, spun_up: bool) -> CheckpointMeta {
        CheckpointMeta { body: "Sandbox".into(), chunk: 0, model_time_s, written_unix: 0.0, healthy: true, spun_up, unix_offset_s }
    }

    /// A cold start at `now` lands, at the end of its spin-up, exactly on the real instant `now`;
    /// afterwards model time follows the clock one for one.
    #[test]
    fn cold_start_lands_on_the_real_instant() {
        let (now, spinup) = (1_795_600_000.0, 250.0 * 90_551.5);
        let off = cold_start_offset(now, spinup);
        assert_eq!(pacing_target(now, off), spinup);
        assert_eq!(pacing_target(now + 3600.0, off), spinup + 3600.0);
        // Unix time of a model instant = model time + offset (what the ephemeris uses).
        assert_eq!(spinup + off, now);
    }

    /// The catch-up needed by a checkpoint: what lies beyond its remaining spin-up.
    #[test]
    fn catchup_needed_by_a_checkpoint() {
        let (now, spinup) = (1_000_000.0, 50_000.0);
        // Spun up, saved 1 000 s of model time before the clock: 1 000 s to catch up.
        let m = meta(100_000.0, now - 101_000.0, true);
        assert_eq!(catchup_needed_s(&m, now, spinup), 1_000.0);
        // Still warming, the service stopped right after: only the rest of the spin-up, no lag.
        let off = cold_start_offset(now - 60.0, spinup);
        let m = meta(10_000.0, off, false);
        assert_eq!(catchup_needed_s(&m, now, spinup), 60.0);
    }
}

// ------------------------------------------------------------------ HTTP

fn not_enabled(state: &Shared, body: &str) -> Response {
    let enabled: Vec<&str> = state.atmosphere.values().map(|r| r.name.as_str()).collect();
    (StatusCode::NOT_FOUND, Json(json!({ "error": format!("the atmosphere model is not enabled for `{body}` (ATMOSPHERE_BODIES)"), "enabled_bodies": enabled }))).into_response()
}

/// `GET /api/v1/{body}/atmosphere`: status, model time, last guard events, list of tables.
pub async fn status(State(state): State<Shared>, Path(body): Path<String>) -> Response {
    let Some(run) = state.atmosphere.get(&body.to_lowercase()) else { return not_enabled(&state, &body) };
    let s = run.status.read().map(|s| s.clone()).unwrap_or_default();
    let tables = run.tables.read();
    let (list, t_model, t_day, rendered) = match &tables {
        Ok(t) => (
            t.tables
                .iter()
                .map(|(name, e)| {
                    json!({
                        "name": name, "min": e.min, "max": e.max,
                        "png": format!("/api/v1/{}/atmosphere/tables/{name}.png", run.name),
                        "json": format!("/api/v1/{}/atmosphere/tables/{name}.json", run.name),
                    })
                })
                .collect::<Vec<_>>(),
            t.model_time_s,
            t.model_day,
            t.rendered_unix,
        ),
        Err(_) => (Vec::new(), 0.0, 0.0, 0.0),
    };
    Json(json!({
        "body": run.name,
        "model": s,
        "tables": { "model_time_s": t_model, "model_day": t_day, "rendered_unix": rendered, "list": list },
    }))
    .into_response()
}

/// `GET /api/v1/{body}/atmosphere/tables/{name}.png` (the 16-bit table) or `{name}.json` (its
/// sidecar: range, units, projection, model time).
pub async fn table(State(state): State<Shared>, Path((body, file)): Path<(String, String)>) -> Response {
    let Some(run) = state.atmosphere.get(&body.to_lowercase()) else { return not_enabled(&state, &body) };
    let Ok(tables) = run.tables.read() else { return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
    if let Some(name) = file.strip_suffix(".png") {
        if let Some(e) = tables.tables.get(name) {
            return ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "no-cache")], e.png.clone()).into_response();
        }
    } else if let Some(name) = file.strip_suffix(".json") {
        if let Some(e) = tables.tables.get(name) {
            return ([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-cache")], e.json.clone()).into_response();
        }
    }
    (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no table `{file}` (yet)"), "tables": tables.tables.keys().collect::<Vec<_>>() }))).into_response()
}
