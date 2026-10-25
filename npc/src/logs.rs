//! Recent log lines of each NPC, kept in memory for the admin panel.
//!
//! Every NPC runtime runs inside a tracing span named `npc` carrying the NPC's
//! uuid (`id`). [`NpcLogLayer`] files each event emitted under such a span —
//! including those of the actions it spawns — into that NPC's ring buffer.
//! Lost on restart by design: the durable history is `npc_events`.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use dashmap::DashMap;
use serde::Serialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

/// Name of the span every NPC runtime runs in.
pub const NPC_SPAN: &str = "npc";

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    /// Increasing across all NPCs; poll with `since` = the last one seen.
    pub seq: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: Map<String, Value>,
}

pub struct LogHub {
    buffers: DashMap<String, Mutex<VecDeque<LogLine>>>,
    capacity: usize,
    seq: AtomicU64,
}

impl LogHub {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self { buffers: DashMap::new(), capacity: capacity.max(1), seq: AtomicU64::new(0) })
    }

    fn push(&self, npc: &str, mut line: LogLine) {
        line.seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let buf = self.buffers.entry(npc.to_string()).or_insert_with(|| Mutex::new(VecDeque::new()));
        let mut buf = buf.lock().unwrap();
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(line);
    }

    /// Lines of `npc` newer than `since`, at `min_level` or above.
    pub fn lines(&self, npc: &str, since: u64, min_level: Option<Level>) -> Vec<LogLine> {
        let Some(buf) = self.buffers.get(npc) else { return Vec::new() };
        let buf = buf.lock().unwrap();
        buf.iter()
            .filter(|l| l.seq > since)
            .filter(|l| match min_level {
                Some(min) => l.level.parse::<Level>().is_ok_and(|lv| lv <= min),
                None => true,
            })
            .cloned()
            .collect()
    }

    /// Forget a deleted NPC.
    pub fn remove(&self, npc: &str) {
        self.buffers.remove(npc);
    }
}

/// Extension stored on `npc` spans: the uuid of the NPC.
struct NpcSpanId(String);

pub struct NpcLogLayer {
    hub: Arc<LogHub>,
}

impl NpcLogLayer {
    pub fn new(hub: Arc<LogHub>) -> Self {
        Self { hub }
    }
}

impl<S> Layer<S> for NpcLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().name() != NPC_SPAN {
            return;
        }
        let mut v = FieldCollector::default();
        attrs.record(&mut v);
        if let (Some(Value::String(npc)), Some(span)) = (v.fields.remove("id"), ctx.span(id)) {
            span.extensions_mut().insert(NpcSpanId(npc));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let Some(scope) = ctx.event_scope(event) else { return };
        let Some(npc) = scope
            .from_root()
            .find_map(|span| span.extensions().get::<NpcSpanId>().map(|n| n.0.clone()))
        else {
            return;
        };
        let mut v = FieldCollector::default();
        event.record(&mut v);
        // The span already says which NPC; repeating it on every line is noise.
        v.fields.remove("npc");
        let meta = event.metadata();
        self.hub.push(
            &npc,
            LogLine {
                seq: 0,
                ts: OffsetDateTime::now_utc(),
                level: meta.level().to_string(),
                target: meta.target().to_string(),
                message: v.message,
                fields: v.fields,
            },
        );
    }
}

#[derive(Default)]
struct FieldCollector {
    message: String,
    fields: Map<String, Value>,
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut s = String::new();
        let _ = write!(s, "{value:?}");
        if field.name() == "message" {
            self.message = s;
        } else {
            self.fields.insert(field.name().to_string(), Value::String(s));
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing::{info, info_span, subscriber::with_default, warn};
    use tracing_subscriber::{layer::SubscriberExt, registry};

    use super::*;

    #[test]
    fn files_events_under_npc_spans_only() {
        let hub = LogHub::new(3);
        let sub = registry().with(NpcLogLayer::new(Arc::clone(&hub)));
        with_default(sub, || {
            info!("outside any NPC");
            let span = info_span!("npc", id = "u1", name = "Jean");
            let _g = span.enter();
            info!(npc = "Jean", action = "go_home", "setting off");
            let inner = info_span!("other");
            let _g2 = inner.enter();
            warn!(waiting_s = 3u64, "no arrival notice");
        });
        let lines = hub.lines("u1", 0, None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].message, "setting off");
        assert_eq!(lines[0].fields.get("action"), Some(&Value::from("go_home")));
        assert!(lines[0].fields.get("npc").is_none());
        assert_eq!(lines[1].level, "WARN");

        // Level filter and `since`.
        assert_eq!(hub.lines("u1", 0, Some(Level::WARN)).len(), 1);
        assert_eq!(hub.lines("u1", lines[0].seq, None).len(), 1);
        assert!(hub.lines("nobody", 0, None).is_empty());
    }

    #[test]
    fn ring_buffer_drops_the_oldest() {
        let hub = LogHub::new(2);
        let sub = registry().with(NpcLogLayer::new(Arc::clone(&hub)));
        with_default(sub, || {
            let span = info_span!("npc", id = "u1");
            let _g = span.enter();
            for i in 0..5 {
                info!(i, "line");
            }
        });
        let lines = hub.lines("u1", 0, None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].fields.get("i"), Some(&Value::from(4)));
    }
}
