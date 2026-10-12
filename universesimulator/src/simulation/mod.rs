//! Simulation tick (ADR 0010): one writer at a time (PostgreSQL advisory lock), per federation:
//! political cache, stocks, flows, reconciliation, prices, planning, publication, treasury,
//! history.

pub mod flows;
pub mod planner;
pub mod prices;
pub mod publisher;
pub mod reconciler;
pub mod stocks;
pub mod treasury;

use std::time::{Duration, Instant};

use serde::Serialize;
use time::OffsetDateTime;
use tracing::{info, warn};

use crate::db::{catalog, history};
use crate::error::{AppError, Result};
use crate::state::AppState;

/// Advisory lock key of the tick ("usim" in ASCII).
const TICK_LOCK: i64 = 0x7573_696d;
/// Elapsed time applied by a single tick is capped (service down for days).
const MAX_ELAPSED_H: f64 = 24.0;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TickReport {
    pub skipped: Option<String>,
    pub federations: usize,
    pub elapsed_game_hours: f64,
    pub flows_applied: usize,
    pub shortages: usize,
    pub legs_completed: usize,
    pub legs_retried: usize,
    pub orders_failed: usize,
    pub orders_created: usize,
    pub legs_published: usize,
    pub minted: i64,
    pub errors: Vec<String>,
}

impl TickReport {
    pub fn error(&mut self, step: &str, e: impl std::fmt::Display) {
        warn!(step, error = %e, "tick step failed");
        self.errors.push(format!("{step}: {e}"));
    }
}

/// Runs the tick forever (when `SIM_ENABLED`).
pub async fn run_loop(st: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(st.cfg.sim_tick_seconds.max(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await; // first tick fires immediately; let the service settle first
    loop {
        interval.tick().await;
        match tick(&st, false).await {
            Ok(r) if r.skipped.is_none() => info!(
                orders_created = r.orders_created,
                legs_published = r.legs_published,
                legs_completed = r.legs_completed,
                errors = r.errors.len(),
                "tick done"
            ),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "tick failed"),
        }
    }
}

/// Runs one tick. `force` ignores the pause (admin "force tick").
pub async fn tick(st: &AppState, force: bool) -> Result<TickReport> {
    let mut conn = st.pool.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)").bind(TICK_LOCK).fetch_one(&mut *conn).await?;
    if !locked {
        return Err(AppError::conflict("TICK_RUNNING", "a tick is already running"));
    }
    let result = run_locked(st, force).await;
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)").bind(TICK_LOCK).execute(&mut *conn).await;
    result
}

async fn run_locked(st: &AppState, force: bool) -> Result<TickReport> {
    let mut report = TickReport::default();
    let state = history::simulation_state(&st.pool).await?;
    if state.paused && !force {
        report.skipped = Some("paused".into());
        return Ok(report);
    }
    let started = Instant::now();
    let now = OffsetDateTime::now_utc();
    let real_h = state
        .last_tick_at
        .map(|t| (now - t).as_seconds_f64() / 3600.0)
        .unwrap_or(st.cfg.sim_tick_seconds as f64 / 3600.0)
        .clamp(0.0, MAX_ELAPSED_H);
    report.elapsed_game_hours = real_h * st.cfg.sim_time_scale;

    // A forced tick also resynchronises the world (new villages and depots).
    let refresh_political = force
        || state
            .last_political_refresh
            .is_none_or(|t| (now - t).whole_minutes() >= st.cfg.sim_political_refresh_minutes);

    let federations = catalog::list_federations(&st.pool).await?;
    report.federations = federations.len();
    for fed in &federations {
        if refresh_political {
            if let Err(e) = stocks::refresh_political_cache(st, fed).await {
                report.error("political", e);
            }
        }
        if refresh_political {
            // Replays the last world file (new villages, new depots); without one, only
            // attaches new depots to the simulated villages.
            let synced = match crate::import::replay(st, fed).await {
                Ok(Some(r)) => Ok(r.villages),
                Ok(None) => crate::import::sync_warehouses(st, fed).await,
                Err(e) => Err(e),
            };
            if let Err(e) = synced {
                report.error("world sync", e);
            }
        }
        let mut levels = match stocks::refresh(st, fed).await {
            Ok(l) => l,
            Err(e) => {
                report.error("stocks", e);
                continue;
            }
        };
        if let Err(e) = flows::apply(st, fed, report.elapsed_game_hours, &mut levels, &mut report).await {
            report.error("flows", e);
        }
        if let Err(e) = reconciler::run(st, fed, &mut report).await {
            report.error("reconcile", e);
        }
        // Reconciliation moved goods: read the stocks again before pricing and planning.
        match stocks::refresh(st, fed).await {
            Ok(l) => levels = l,
            Err(e) => report.error("stocks", e),
        }
        if let Err(e) = prices::run(st, fed, &levels).await {
            report.error("prices", e);
        }
        if let Err(e) = planner::run(st, fed, &levels, &mut report).await {
            report.error("planner", e);
        }
        let funds_short = match publisher::run(st, fed, &mut report).await {
            Ok(short) => short,
            Err(e) => {
                report.error("publish", e);
                false
            }
        };
        if let Err(e) = treasury::run(st, fed, funds_short, &mut report).await {
            report.error("treasury", e);
        }
        if let Err(e) = history::snapshot_stocks(&st.pool, fed.id).await {
            report.error("history", e);
        }
    }
    if refresh_political {
        history::record_political_refresh(&st.pool).await?;
    }
    history::purge_history(&st.pool, st.cfg.sim_history_days).await?;
    let error = (!report.errors.is_empty()).then(|| report.errors.join("; "));
    history::record_tick(&st.pool, now, started.elapsed().as_millis() as i64, error.as_deref()).await?;
    Ok(report)
}
