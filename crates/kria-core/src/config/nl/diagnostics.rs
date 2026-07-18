//! Durable-in-process decision observability for the settings pipeline
//! (settings-nl-intelligence Wave 2 / R7.3 / L4). Every settings-stage
//! classification records a compact, inspectable trace (prompt preview, decision,
//! confidence, resolved field, evidence signals) into a bounded ring so production
//! misroutes are diagnosable from logs/introspection instead of guesswork.
//!
//! Privacy: the prompt is truncated and never stores secret VALUES (the pipeline
//! only sees the raw text; we cap length and rely on the handler/audit to redact
//! secrets — traces here are for routing diagnosis, not value capture).

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::config::nl::pipeline::SettingsIntentTrace;

const RING_CAP: usize = 200;
const PROMPT_PREVIEW_MAX: usize = 160;
/// Cap the durable JSONL file; when exceeded it is truncated (keep it bounded).
const PERSIST_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Optional durable sink path. When set (by the desktop at startup), every trace
/// is also appended as JSONL so routing decisions survive restart for debugging
/// (Task 11 observability). Unset in tests → in-memory ring only.
fn persist_path() -> &'static std::sync::RwLock<Option<PathBuf>> {
    static PATH: OnceLock<std::sync::RwLock<Option<PathBuf>>> = OnceLock::new();
    PATH.get_or_init(|| std::sync::RwLock::new(None))
}

/// Wire the durable diagnostics file. Called once at startup by the desktop.
pub fn set_persist_path(path: PathBuf) {
    if let Ok(mut p) = persist_path().write() {
        *p = Some(path);
    }
}

fn current_persist_path() -> Option<PathBuf> {
    persist_path().read().ok().and_then(|p| p.clone())
}

fn persist(rec: &TraceRecord) {
    let Some(path) = current_persist_path() else {
        return;
    };
    let path = &path;
    // Bound the file: truncate if it grew past the cap.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > PERSIST_MAX_BYTES {
            let _ = std::fs::write(path, b"");
        }
    } else if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        if let Ok(line) = serde_json::to_string(&rec.to_json()) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// One recorded routing decision.
#[derive(Clone, Debug)]
pub struct TraceRecord {
    pub ts_ms: u128,
    pub session_id: String,
    pub prompt_preview: String,
    pub decision: String,
    pub confidence: f32,
    pub best_field: Option<(String, String)>,
    pub subject: Option<&'static str>,
    pub value_grounded: bool,
    pub conversation_topic: f32,
    pub memory_topic: f32,
    pub embeddings_used: bool,
}

impl TraceRecord {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ts_ms": self.ts_ms.to_string(),
            "session_id": self.session_id,
            "prompt": self.prompt_preview,
            "decision": self.decision,
            "confidence": self.confidence,
            "field": self.best_field.as_ref().map(|(s, f)| format!("{s}.{f}")),
            "subject": self.subject,
            "value_grounded": self.value_grounded,
            "conversation_topic": self.conversation_topic,
            "memory_topic": self.memory_topic,
            "embeddings_used": self.embeddings_used,
        })
    }
}

fn ring() -> &'static Mutex<VecDeque<TraceRecord>> {
    static RING: std::sync::OnceLock<Mutex<VecDeque<TraceRecord>>> = std::sync::OnceLock::new();
    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(RING_CAP)))
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn preview(prompt: &str) -> String {
    let p = prompt.trim();
    if p.chars().count() <= PROMPT_PREVIEW_MAX {
        p.to_string()
    } else {
        let cut: String = p.chars().take(PROMPT_PREVIEW_MAX).collect();
        format!("{cut}…")
    }
}

/// Record a classification decision + its trace into the bounded ring.
pub fn record(session_id: &str, prompt: &str, trace: &SettingsIntentTrace) {
    let rec = TraceRecord {
        ts_ms: now_ms(),
        session_id: session_id.to_string(),
        prompt_preview: preview(prompt),
        decision: trace.decision.to_string(),
        confidence: trace.confidence,
        best_field: trace.best_field.clone(),
        subject: trace.subject,
        value_grounded: trace.value_grounded,
        conversation_topic: trace.conversation_topic,
        memory_topic: trace.memory_topic,
        embeddings_used: trace.embeddings_used,
    };
    tracing::debug!(
        target: "settings_intent",
        decision = %rec.decision,
        confidence = rec.confidence,
        field = ?rec.best_field,
        subject = ?rec.subject,
        value_grounded = rec.value_grounded,
        "settings intent decision"
    );
    persist(&rec);
    if let Ok(mut r) = ring().lock() {
        if r.len() >= RING_CAP {
            r.pop_front();
        }
        r.push_back(rec);
    }
}

/// The most recent `n` recorded decisions (newest last), as JSON for a debug UI.
pub fn recent(n: usize) -> Vec<serde_json::Value> {
    ring()
        .lock()
        .map(|r| {
            let skip = r.len().saturating_sub(n);
            r.iter().skip(skip).map(|t| t.to_json()).collect()
        })
        .unwrap_or_default()
}

/// Most recent `n` decisions for one session (newest last). Session-scoped
/// reads avoid cross-session contamination in diagnostics consumers.
pub fn recent_for_session(session_id: &str, n: usize) -> Vec<serde_json::Value> {
    ring()
        .lock()
        .map(|r| {
            let matching: Vec<_> = r
                .iter()
                .filter(|trace| trace.session_id == session_id)
                .collect();
            let skip = matching.len().saturating_sub(n);
            matching
                .into_iter()
                .skip(skip)
                .map(|trace| trace.to_json())
                .collect()
        })
        .unwrap_or_default()
}

/// Test-only: clear the ring so bounded-capacity assertions are deterministic.
#[cfg(test)]
pub fn clear() {
    if let Ok(mut r) = ring().lock() {
        r.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::nl::pipeline::SettingsIntentTrace;

    #[test]
    fn records_and_reads_back_recent_traces() {
        let mut t = SettingsIntentTrace::default();
        t.decision = "change";
        t.confidence = 0.91;
        t.best_field = Some(("ui".into(), "theme".into()));
        t.value_grounded = true;
        record("diagnostics-test-session", "switch to dark mode", &t);

        let recent = recent_for_session("diagnostics-test-session", 10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["decision"], "change");
        assert_eq!(recent[0]["field"], "ui.theme");
        assert_eq!(recent[0]["session_id"], "diagnostics-test-session");
    }

    #[test]
    fn persists_traces_to_jsonl_when_path_set() {
        let dir = std::env::temp_dir().join(format!("kria-diag-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings_intent.jsonl");
        let _ = std::fs::remove_file(&path);
        set_persist_path(path.clone());
        let mut t = SettingsIntentTrace::default();
        t.decision = "change";
        t.confidence = 0.9;
        record("s", "switch to dark mode", &t);
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            content.contains("\"decision\":\"change\""),
            "got: {content}"
        );
        // Reset so other tests aren't affected by the durable sink.
        if let Ok(mut p) = persist_path().write() {
            *p = None;
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ring_is_bounded() {
        clear();
        for i in 0..(RING_CAP + 50) {
            let mut t = SettingsIntentTrace::default();
            t.decision = "not_settings";
            record("s", &format!("msg {i}"), &t);
        }
        assert!(recent(10_000).len() <= RING_CAP);
    }
}
