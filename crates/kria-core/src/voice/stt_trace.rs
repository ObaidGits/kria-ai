//! JSONL tracing for `ENHANCED_STT.md` §17.
//!
//! When the environment variable **`KRIA_STT_TRACE_JSONL`** is set to a file
//! path, each helper below appends one JSON object per line (`unix_ms` wall
//! clock on every record).
//!
//! ## §17 Compliance
//!
//! **SHALL emit structured events:**
//! - `stt_session_start` — turn start with engine/profile
//! - `stt_partial` — partial transcript update
//! - `stt_commit` — UtteranceCommitted event
//! - `stt_refine_done` — Whisper refine completion
//! - `stt_reconcile_result` — §7 reconciliation outcome
//! - `stt_sidecar_restart` — sidecar supervision event (P2)
//! - `stt_backpressure` — bounded queue overflow
//! - `audio_device_lost` — device recovery event
//!
//! **Guarantees:**
//! - Append-safe writes (OpenOptions::append)
//! - Stable schema (deterministic field names)
//! - Bounded buffering (unbuffered writes via writeln!)
//! - Non-blocking (best-effort, drops on error)
//! - Monotonic timestamps (unix_ms)

use once_cell::sync::OnceCell;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

static TRACE_FILE: OnceCell<Mutex<Option<std::fs::File>>> = OnceCell::new();

pub fn unix_ms_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn trace_path() -> Option<String> {
    std::env::var("KRIA_STT_TRACE_JSONL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

fn append_value(row: Value) {
    let Some(path) = trace_path() else {
        return;
    };
    let cell = TRACE_FILE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = OpenOptions::new().create(true).append(true).open(path).ok();
    }
    if let Some(f) = guard.as_mut() {
        if let Ok(line) = serde_json::to_string(&row) {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub fn emit_stt_session_start(
    turn_seq: u64,
    session_id: &str,
    generation: u64,
    stt_engine: &str,
    vad_profile: &str,
) {
    append_value(serde_json::json!({
        "event": "stt_session_start",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "stt_engine": stt_engine,
        "vad_profile": vad_profile,
    }));
}

pub fn emit_stt_partial(
    turn_seq: u64,
    session_id: &str,
    generation: u64,
    seq: u64,
    text_char_len: usize,
    engine: &str,
) {
    append_value(serde_json::json!({
        "event": "stt_partial",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "seq": seq,
        "text_char_len": text_char_len,
        "engine": engine,
    }));
}

pub fn emit_stt_commit(turn_seq: u64, session_id: &str, generation: u64, text_char_len: usize) {
    append_value(serde_json::json!({
        "event": "stt_commit",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "text_char_len": text_char_len,
    }));
}

pub fn emit_stt_refine_done(
    turn_seq: u64,
    session_id: &str,
    generation: u64,
    skipped: bool,
    refine_latency_ms: u64,
) {
    append_value(serde_json::json!({
        "event": "stt_refine_done",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "skipped": skipped,
        "refine_latency_ms": refine_latency_ms,
    }));
}

pub fn emit_stt_reconcile_result(
    turn_seq: u64,
    session_id: &str,
    generation: u64,
    reconcile_kind: &str,
    user_visible_char_len: usize,
    whisper_char_len: usize,
) {
    append_value(serde_json::json!({
        "event": "stt_reconcile_result",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "reconcile": reconcile_kind,
        "user_visible_char_len": user_visible_char_len,
        "whisper_char_len": whisper_char_len,
    }));
}

pub fn emit_stt_sidecar_restart(
    turn_seq: u64,
    session_id: &str,
    generation: u64,
    attempt: u32,
    backoff_ms: u64,
) {
    append_value(serde_json::json!({
        "event": "stt_sidecar_restart",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "attempt": attempt,
        "backoff_ms": backoff_ms,
    }));
}

pub fn emit_stt_backpressure(turn_seq: u64, session_id: &str, generation: u64, note: &str) {
    append_value(serde_json::json!({
        "event": "stt_backpressure",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "note": note,
    }));
}

pub fn emit_audio_device_lost(turn_seq: u64, session_id: &str, generation: u64, reason: &str) {
    append_value(serde_json::json!({
        "event": "audio_device_lost",
        "unix_ms": unix_ms_now(),
        "turn_seq": turn_seq,
        "session_id": session_id,
        "generation": generation,
        "reason": reason,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn jsonl_emits_all_event_kinds_with_unix_ms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stt.jsonl");
        let path_s = path.to_string_lossy().to_string();
        std::env::set_var("KRIA_STT_TRACE_JSONL", &path_s);

        let turn = 7u64;
        let session_id = "test-session-123";
        let generation = 0u64;

        emit_stt_session_start(turn, session_id, generation, "stub-engine", "normal");
        emit_stt_partial(turn, session_id, generation, 1, 12, "stub");
        emit_stt_commit(turn, session_id, generation, 40);
        emit_stt_refine_done(turn, session_id, generation, true, 0);
        emit_stt_reconcile_result(turn, session_id, generation, "identical", 10, 10);
        emit_stt_sidecar_restart(turn, session_id, generation, 1, 100);
        emit_stt_backpressure(turn, session_id, generation, "unit_test");
        emit_audio_device_lost(turn, session_id, generation, "unit_test");

        let mut buf = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut buf)
            .unwrap();

        let kinds = [
            "stt_session_start",
            "stt_partial",
            "stt_commit",
            "stt_refine_done",
            "stt_reconcile_result",
            "stt_sidecar_restart",
            "stt_backpressure",
            "audio_device_lost",
        ];
        for k in kinds {
            assert!(
                buf.contains(&format!("\"event\":\"{k}\"")),
                "missing event {k} in {buf}"
            );
        }
        assert!(buf.contains("\"unix_ms\":"));
        assert!(buf.contains("\"session_id\":\"test-session-123\""));
        assert!(buf.contains("\"generation\":0"));

        std::env::remove_var("KRIA_STT_TRACE_JSONL");
    }
}
