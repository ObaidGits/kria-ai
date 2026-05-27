use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::infra::pipeline_trace;

const DEFAULT_DIAGNOSTIC_RING_CAPACITY: usize = 512;
const MIN_DIAGNOSTIC_RING_CAPACITY: usize = 64;
const MAX_DIAGNOSTIC_RING_CAPACITY: usize = 5_000;
const MAX_FIELD_CHARS: usize = 500;

static DIAGNOSTICS: Lazy<DiagnosticsRing> =
    Lazy::new(|| DiagnosticsRing::new(configured_ring_capacity()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub target: String,
    pub message: Option<String>,
    pub fields: Value,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsSummary {
    pub capacity: usize,
    pub captured_events: usize,
    pub by_level: BTreeMap<String, usize>,
    pub last_event_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct DiagnosticsRing {
    capacity: usize,
    events: Mutex<VecDeque<DiagnosticEvent>>,
}

impl DiagnosticsRing {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            events: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    fn push(&self, event: DiagnosticEvent) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        while events.len() >= self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    fn recent(&self, limit: usize, min_level: Option<&str>) -> Vec<DiagnosticEvent> {
        let limit = limit.clamp(1, self.capacity);
        let min_rank = min_level
            .map(level_rank)
            .unwrap_or_else(|| level_rank("info"));
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events
            .iter()
            .rev()
            .filter(|event| level_rank(&event.level) >= min_rank)
            .take(limit)
            .cloned()
            .collect()
    }

    fn summary(&self) -> DiagnosticsSummary {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let mut by_level = BTreeMap::new();
        for event in events.iter() {
            *by_level.entry(event.level.clone()).or_insert(0) += 1;
        }
        DiagnosticsSummary {
            capacity: self.capacity,
            captured_events: events.len(),
            by_level,
            last_event_at: events.back().map(|event| event.timestamp),
        }
    }
}

pub struct DiagnosticsLayer;

impl<S> Layer<S> for DiagnosticsLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if !should_capture(metadata.level(), metadata.target()) {
            return;
        }

        let mut visitor = DiagnosticFieldVisitor::default();
        event.record(&mut visitor);

        DIAGNOSTICS.push(DiagnosticEvent {
            timestamp: Utc::now(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
            fields: pipeline_trace::sanitize_json_for_logs(
                &Value::Object(visitor.fields),
                MAX_FIELD_CHARS,
                6,
            ),
            file: metadata.file().map(ToOwned::to_owned),
            line: metadata.line(),
        });
    }
}

#[derive(Default)]
struct DiagnosticFieldVisitor {
    message: Option<String>,
    fields: Map<String, Value>,
}

impl DiagnosticFieldVisitor {
    fn insert_text(&mut self, field: &Field, value: String) {
        let key = field.name();
        let sanitized = pipeline_trace::sanitize_text_for_logs(&value, MAX_FIELD_CHARS);
        if key == "message" {
            self.message = Some(strip_debug_quotes(&sanitized));
            return;
        }
        self.fields.insert(
            key.to_string(),
            Value::String(strip_debug_quotes(&sanitized)),
        );
    }

    fn insert_value(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
            return;
        }
        self.fields.insert(field.name().to_string(), value);
    }
}

impl Visit for DiagnosticFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert_text(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert_text(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert_value(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert_value(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert_value(field, Value::from(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.insert_text(field, value.to_string());
    }
}

pub fn recent_diagnostics(limit: usize, min_level: Option<&str>) -> Vec<DiagnosticEvent> {
    DIAGNOSTICS.recent(limit, min_level)
}

pub fn diagnostics_summary() -> DiagnosticsSummary {
    DIAGNOSTICS.summary()
}

fn configured_ring_capacity() -> usize {
    std::env::var("KRIA_DIAGNOSTIC_RING_CAPACITY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_DIAGNOSTIC_RING_CAPACITY)
        .clamp(MIN_DIAGNOSTIC_RING_CAPACITY, MAX_DIAGNOSTIC_RING_CAPACITY)
}

fn should_capture(level: &Level, target: &str) -> bool {
    if matches!(*level, Level::ERROR | Level::WARN) {
        return true;
    }

    if *level == Level::INFO {
        return matches!(
            target,
            "runtime_health"
                | "kria_pipeline"
                | "kria_dashboard"
                | "gui_wiring"
                | "gui_executor"
                | "global_halt"
        ) || target.contains("gui")
            || target.contains("orchestrator");
    }

    std::env::var("KRIA_DIAGNOSTIC_DEBUG")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn level_rank(level: &str) -> usize {
    match level.to_ascii_lowercase().as_str() {
        "error" => 5,
        "warn" | "warning" => 4,
        "info" => 3,
        "debug" => 2,
        "trace" => 1,
        _ => 3,
    }
}

fn strip_debug_quotes(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_filters_by_min_level() {
        let ring = DiagnosticsRing::new(4);
        ring.push(DiagnosticEvent {
            timestamp: Utc::now(),
            level: "INFO".into(),
            target: "runtime_health".into(),
            message: Some("ok".into()),
            fields: Value::Object(Map::new()),
            file: None,
            line: None,
        });
        ring.push(DiagnosticEvent {
            timestamp: Utc::now(),
            level: "WARN".into(),
            target: "gui_executor".into(),
            message: Some("slow".into()),
            fields: Value::Object(Map::new()),
            file: None,
            line: None,
        });

        let recent = ring.recent(10, Some("warn"));
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].level, "WARN");
    }

    #[test]
    fn diagnostics_ring_is_bounded() {
        let ring = DiagnosticsRing::new(2);
        for i in 0..3 {
            ring.push(DiagnosticEvent {
                timestamp: Utc::now(),
                level: "ERROR".into(),
                target: "test".into(),
                message: Some(format!("event {i}")),
                fields: Value::Object(Map::new()),
                file: None,
                line: None,
            });
        }

        let recent = ring.recent(10, Some("trace"));
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].message.as_deref(), Some("event 2"));
        assert_eq!(recent[1].message.as_deref(), Some("event 1"));
    }
}
