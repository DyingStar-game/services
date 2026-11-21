//! Tasks given by hand from the admin panel (`npc_tasks`). They pre-empt the
//! schedule: the runtime runs one instead of picking a scheduled action, and
//! nothing else runs until it ends.
//!
//! - `go_to_position {position, uuid?, hold_s?, timeout_s?}`: walk there (the
//!   position is local to object `uuid`, or the NPC's own frame), then stay
//!   `hold_s` seconds (default 60) before the schedule takes over again.
//! - `run_action {action, config?, duration_s?}`: focus on one catalogue action
//!   for up to `duration_s` (default 300), outside the schedule and the NPC's
//!   action profile; `config` overrides that action's config block (e.g.
//!   `drive_to_factory {destination}`).
//! - `wait {seconds}`: stand still.

use std::{sync::Arc, time::Duration};

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::time::{self, Instant};
use tracing::{info, warn};
use uuid::Uuid;

use super::{ActionContext, ActionRegistry};
use crate::{
    db::tasks::{self, TaskRow},
    error::{NpcError, Result},
    events::EventSink,
    horizon::messages::Vec3,
    npc::{spec::NpcSpec, state::NpcStatus},
};

const MAX_S: u64 = 3600;
const POLL: Duration = Duration::from_millis(500);
/// A focused action that has not been eligible for this long after running
/// is done (longer than a walk's arrival watchdog, so a walk is not cut short).
const IDLE_DONE: Duration = Duration::from_secs(150);

/// Actions that cannot be run by hand (stubs).
const NOT_MANUAL: [&str; 2] = ["check_vehicle_components", "go_sell_object"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoToPosition {
    position: Vec3,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default = "default_hold")]
    hold_s: u64,
    #[serde(default = "default_timeout")]
    timeout_s: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunAction {
    action: String,
    #[serde(default)]
    config: Option<Value>,
    #[serde(default = "default_duration")]
    duration_s: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wait {
    seconds: u64,
}

fn default_hold() -> u64 {
    60
}
fn default_timeout() -> u64 {
    300
}
fn default_duration() -> u64 {
    300
}

/// Names of the actions an admin may run by hand.
pub fn manual_actions() -> Vec<&'static str> {
    let spec: NpcSpec = serde_json::from_value(json!({ "uuid": "" })).expect("bare spec");
    ActionRegistry::catalogue(&spec)
        .iter()
        .map(|a| a.name())
        .filter(|n| !NOT_MANUAL.contains(n))
        .collect()
}

/// Check a task before it is queued.
pub fn validate(kind: &str, params: &Value) -> Result<()> {
    let bad = |e: serde_json::Error| NpcError::BadRequest(format!("invalid {kind} params: {e}"));
    match kind {
        "go_to_position" => {
            let p: GoToPosition = serde_json::from_value(params.clone()).map_err(bad)?;
            if p.hold_s > MAX_S || p.timeout_s == 0 || p.timeout_s > MAX_S {
                return Err(NpcError::BadRequest(format!("hold_s and timeout_s must be within 0..{MAX_S}")));
            }
        }
        "run_action" => {
            let p: RunAction = serde_json::from_value(params.clone()).map_err(bad)?;
            if !manual_actions().contains(&p.action.as_str()) {
                return Err(NpcError::BadRequest(format!("unknown action '{}'", p.action)));
            }
            if p.duration_s == 0 || p.duration_s > MAX_S {
                return Err(NpcError::BadRequest(format!("duration_s must be within 1..{MAX_S}")));
            }
        }
        "wait" => {
            let p: Wait = serde_json::from_value(params.clone()).map_err(bad)?;
            if p.seconds == 0 || p.seconds > MAX_S {
                return Err(NpcError::BadRequest(format!("seconds must be within 1..{MAX_S}")));
            }
        }
        other => return Err(NpcError::BadRequest(format!("unknown task kind '{other}'"))),
    }
    Ok(())
}

/// Short label for the "current action" of the admin panel.
pub fn label(task: &TaskRow) -> String {
    match (task.kind.as_str(), task.params.0.get("action").and_then(Value::as_str)) {
        ("run_action", Some(a)) => format!("manual:{a}"),
        (kind, _) => format!("manual:{kind}"),
    }
}

/// Run `task` to its end and record the outcome.
pub async fn execute(
    task: TaskRow,
    mut ctx: ActionContext,
    spec: Arc<NpcSpec>,
    pool: PgPool,
    events: EventSink,
    npc_id: Uuid,
) {
    let result = match task.kind.as_str() {
        "go_to_position" => go_to_position(&mut ctx, &task.params.0).await,
        "run_action" => run_action(&mut ctx, &spec, &task.params.0).await,
        "wait" => wait(&task.params.0).await,
        other => Err(NpcError::BadRequest(format!("unknown task kind '{other}'"))),
    };
    let (status, error) = match &result {
        Ok(()) => ("done", None),
        Err(NpcError::Protocol(m)) if m.starts_with("timeout") => ("timeout", Some(m.clone())),
        Err(e) => ("failed", Some(e.to_string())),
    };
    match &error {
        None => info!(npc = %ctx.npc_name, task = task.id, "manual task done"),
        Some(e) => warn!(npc = %ctx.npc_name, task = task.id, status, error = %e, "manual task ended"),
    }
    if let Err(e) = tasks::finish(&pool, task.id, status, error.as_deref()).await {
        warn!(npc = %ctx.npc_name, task = task.id, error = %e, "cannot record the task outcome");
    }
    events.emit(
        Some(npc_id),
        &format!("task_{status}"),
        if error.is_some() { "warn" } else { "info" },
        format!("task {} ({}) {status}", task.id, task.kind),
        Some(json!({ "task_id": task.id, "error": error })),
    );
    // Let the schedule plan from scratch: the NPC is no longer where its
    // last scheduled status says.
    ctx.state.set_status(NpcStatus::Idle);
}

async fn go_to_position(ctx: &mut ActionContext, params: &Value) -> Result<()> {
    let p: GoToPosition = serde_json::from_value(params.clone())?;
    let uuid = p.uuid.unwrap_or_default();
    ctx.state.set_status(NpcStatus::Idle);
    // Clear a stale arrival first (see GoingToWork::dispatch).
    ctx.send
        .send("player", "client_action", json!({ "action": "npc_arrived_ack", "uuid": uuid }))
        .await?;
    ctx.world.clear_arrived();
    ctx.send
        .send(
            "player",
            "client_action",
            json!({ "action": "npc_go_to_position", "uuid": uuid, "position": p.position }),
        )
        .await?;
    info!(npc = %ctx.npc_name, frame = %uuid, position = ?p.position, "manual walk dispatched");

    let deadline = Instant::now() + Duration::from_secs(p.timeout_s);
    while !ctx.world.npc_arrived() {
        if Instant::now() >= deadline {
            return Err(NpcError::Protocol(format!("timeout: no arrival after {}s", p.timeout_s)));
        }
        time::sleep(POLL).await;
    }
    ctx.send
        .send("player", "client_action", json!({ "action": "npc_arrived_ack", "uuid": uuid }))
        .await?;
    ctx.world.clear_arrived();
    info!(npc = %ctx.npc_name, hold_s = p.hold_s, "manual walk arrived");
    time::sleep(Duration::from_secs(p.hold_s)).await;
    Ok(())
}

async fn run_action(ctx: &mut ActionContext, spec: &NpcSpec, params: &Value) -> Result<()> {
    let p: RunAction = serde_json::from_value(params.clone())?;
    let mut spec = spec.clone();
    if let Some(cfg) = p.config {
        let mut v = serde_json::to_value(&spec)?;
        v[&p.action] = cfg;
        spec = serde_json::from_value(v)
            .map_err(|e| NpcError::BadRequest(format!("invalid config for {}: {e}", p.action)))?;
    }
    let action = ActionRegistry::catalogue(&spec)
        .into_iter()
        .find(|a| a.name() == p.action)
        .ok_or_else(|| NpcError::BadRequest(format!("unknown action '{}'", p.action)))?;
    // No schedule gating while focused.
    ctx.allowed_actions = None;

    let deadline = Instant::now() + Duration::from_secs(p.duration_s);
    let mut runs = 0u32;
    let mut last_active = Instant::now();
    while Instant::now() < deadline {
        if action.can_run(ctx) {
            let left = deadline.saturating_duration_since(Instant::now());
            match time::timeout(left, action.run(ctx)).await {
                Ok(r) => r?,
                Err(_) => break,
            }
            runs += 1;
            last_active = Instant::now();
        } else if runs > 0 && last_active.elapsed() >= IDLE_DONE {
            break;
        }
        time::sleep(POLL).await;
    }
    if runs == 0 {
        return Err(NpcError::Protocol(format!(
            "timeout: '{}' never became eligible in {}s",
            p.action, p.duration_s
        )));
    }
    info!(npc = %ctx.npc_name, action = %p.action, runs, "focused action over");
    Ok(())
}

async fn wait(params: &Value) -> Result<()> {
    let p: Wait = serde_json::from_value(params.clone())?;
    time::sleep(Duration::from_secs(p.seconds)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_params() {
        assert!(validate("go_to_position", &json!({"position": {"x": 1.0, "y": 0.0, "z": 2.0}})).is_ok());
        assert!(validate("go_to_position", &json!({"position": {"x": 1.0}})).is_err());
        assert!(validate("go_to_position", &json!({"position": {"x": 1.0, "y": 0.0, "z": 2.0}, "fly": true})).is_err());
        assert!(validate("run_action", &json!({"action": "take_a_walk"})).is_ok());
        assert!(validate("run_action", &json!({"action": "go_sell_object"})).is_err());
        assert!(validate("run_action", &json!({"action": "take_a_walk", "duration_s": 99999})).is_err());
        assert!(validate("wait", &json!({"seconds": 10})).is_ok());
        assert!(validate("wait", &json!({"seconds": 0})).is_err());
        assert!(validate("teleport", &json!({})).is_err());
    }

    #[test]
    fn manual_actions_list_the_catalogue() {
        let names = manual_actions();
        assert!(names.contains(&"drive_to_factory"));
        assert!(names.contains(&"going_to_work"));
        assert!(!names.contains(&"go_sell_object"));
    }
}
