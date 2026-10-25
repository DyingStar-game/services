//! Key NPC events (spawn, connection, action changes, role, tasks…) for the
//! admin panel, written to `npc_events` in the background: runtimes hand them
//! to a bounded channel and never wait on the database.

use std::time::Duration;

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

use crate::db::events::{self, NewEvent};

const CHANNEL: usize = 10_000;
const BATCH: usize = 200;
const FLUSH: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct EventSink {
    tx: Option<mpsc::Sender<NewEvent>>,
}

impl EventSink {
    /// Start the writer task.
    pub fn spawn(pool: PgPool) -> Self {
        let (tx, mut rx) = mpsc::channel::<NewEvent>(CHANNEL);
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(BATCH);
            loop {
                let first = tokio::select! {
                    e = rx.recv() => e,
                    _ = tokio::time::sleep(FLUSH), if !buf.is_empty() => None,
                };
                match first {
                    Some(e) => {
                        buf.push(e);
                        while buf.len() < BATCH {
                            match rx.try_recv() {
                                Ok(e) => buf.push(e),
                                Err(_) => break,
                            }
                        }
                        if buf.len() < BATCH {
                            continue;
                        }
                    }
                    None if rx.is_closed() && buf.is_empty() => return,
                    None => {}
                }
                if let Err(e) = events::insert_batch(&pool, &buf).await {
                    warn!(error = %e, count = buf.len(), "failed to write NPC events");
                }
                buf.clear();
            }
        });
        Self { tx: Some(tx) }
    }

    /// A sink that drops everything (tests).
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    pub fn emit(
        &self,
        npc_id: Option<Uuid>,
        kind: &str,
        level: &'static str,
        message: impl Into<String>,
        data: Option<Value>,
    ) {
        let Some(tx) = &self.tx else { return };
        let event = NewEvent {
            npc_id,
            ts: OffsetDateTime::now_utc(),
            kind: kind.to_string(),
            level,
            message: message.into(),
            data,
        };
        if tx.try_send(event).is_err() {
            warn!(kind, "NPC event channel full; event dropped");
        }
    }

    pub fn info(&self, npc_id: Uuid, kind: &str, message: impl Into<String>, data: Option<Value>) {
        self.emit(Some(npc_id), kind, "info", message, data);
    }

    pub fn warn(&self, npc_id: Uuid, kind: &str, message: impl Into<String>, data: Option<Value>) {
        self.emit(Some(npc_id), kind, "warn", message, data);
    }

    /// A service-wide event (global switch, schedule import…).
    pub fn global(&self, kind: &str, message: impl Into<String>, data: Option<Value>) {
        self.emit(None, kind, "info", message, data);
    }
}

/// Hourly cleanup: events older than `days`, finished tasks older than 30 days.
pub fn spawn_retention(pool: PgPool, days: u32) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match events::prune(&pool, days as i32).await {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "pruned old NPC events"),
                Ok(_) => {}
                Err(e) => warn!(error = %e, "failed to prune NPC events"),
            }
            if let Err(e) = crate::db::tasks::prune(&pool, 30).await {
                warn!(error = %e, "failed to prune NPC tasks");
            }
        }
    });
}
