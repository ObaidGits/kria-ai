//! Voice turn diagnostics — structured per-turn outcome + latency telemetry.
//!
//! Voice System v3, Wave 7 (observability). A process-global bounded ring
//! buffer of recent voice turns, each carrying a typed outcome/reason and the
//! measured latency breakdown. Exposed to the UI via the `voice_turn_diagnostics`
//! Tauri command so operators can answer "why did this turn fail / time out /
//! return empty?" without trawling logs.
//!
//! Only MEASURED values are recorded — never placeholders. Latency fields are
//! `Option` and remain `None` when the underlying milestone did not occur.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::metrics::VoiceMetrics;

/// Max turns retained in the ring buffer.
pub const MAX_TURN_RECORDS: usize = 64;

/// Typed terminal outcome of a voice turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    /// Completed end-to-end (STT → LLM → TTS → playback).
    Completed,
    /// STT produced an empty/silence transcript; turn bailed cleanly.
    EmptyTranscript,
    /// Cancelled by barge-in.
    BargeIn,
    /// Rejected because another turn was already active.
    Busy,
    /// Hit a recovery watchdog / hard timeout.
    Timeout,
    /// A typed error aborted the turn (see `reason`).
    Error,
}

impl TurnOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::EmptyTranscript => "empty_transcript",
            Self::BargeIn => "barge_in",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::Error => "error",
        }
    }
}

/// Structured failure classification, derived from the error message so the UI
/// can answer "why" without string-matching on the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    SttEmpty,
    SttSidecarUnavailable,
    SttDecode,
    SttTimeout,
    LlmRouting,
    LlmTimeout,
    TtsSynthesis,
    ModelUnavailable,
    GpuLease,
    Capture,
    Playback,
    TurnTimeout,
    Cancelled,
    Other,
}

impl FailureClass {
    /// Classify a free-form error/reason string into a stable category.
    pub fn classify(msg: &str) -> Self {
        let m = msg.to_ascii_lowercase();
        if m.contains("empty utterance") || m.contains("empty transcript") {
            Self::SttEmpty
        } else if m.contains("sidecar not ready") || m.contains("sidecar request failed") {
            Self::SttSidecarUnavailable
        } else if m.contains("failed to encode") || m.contains("decode") || m.contains("whisper") {
            Self::SttDecode
        } else if m.contains("transcribe") && m.contains("timeout") {
            Self::SttTimeout
        } else if m.contains("no backend") || m.contains("no llm") || m.contains("routing") {
            Self::LlmRouting
        } else if m.contains("llm") && m.contains("timeout") {
            Self::LlmTimeout
        } else if m.contains("tts")
            || m.contains("synth")
            || m.contains("piper")
            || m.contains("kokoro")
        {
            Self::TtsSynthesis
        } else if m.contains("model not found")
            || m.contains("model unavailable")
            || m.contains("not loaded")
        {
            Self::ModelUnavailable
        } else if m.contains("gpu lease") || m.contains("lease unavailable") {
            Self::GpuLease
        } else if m.contains("mic") || m.contains("capture") || m.contains("device") {
            Self::Capture
        } else if m.contains("playback") {
            Self::Playback
        } else if m.contains("max duration")
            || m.contains("watchdog")
            || m.contains("turn exceeded")
        {
            Self::TurnTimeout
        } else if m.contains("cancel") {
            Self::Cancelled
        } else {
            Self::Other
        }
    }
}

/// A single recorded voice turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceTurnRecord {
    pub seq: u64,
    pub ended_unix_ms: u64,
    pub outcome: TurnOutcome,
    /// Human-readable reason (error message / note). `None` on clean success.
    pub reason: Option<String>,
    /// Structured failure class when `outcome` indicates failure.
    pub failure_class: Option<FailureClass>,
    pub stt_engine: Option<String>,
    pub tts_engine: Option<String>,
    pub transcript_len: Option<usize>,
    // Derived latencies (ms, speech-end relative); only when measured.
    pub stt_latency_ms: Option<u64>,
    pub partial_latency_ms: Option<u64>,
    pub llm_ttft_ms: Option<u64>,
    pub tts_gen_ms: Option<u64>,
    pub playback_start_ms: Option<u64>,
    pub end_to_end_ms: Option<u64>,
    pub ttfa_budget_ms: Option<u64>,
    pub ttfa_overrun: bool,
    /// Full metric snapshot when available (for the detail view).
    pub metrics: Option<VoiceMetrics>,
}

impl VoiceTurnRecord {
    /// Build a completed-turn record from a finalised [`VoiceMetrics`].
    pub fn from_metrics(seq: u64, outcome: TurnOutcome, m: &VoiceMetrics) -> Self {
        Self {
            seq,
            ended_unix_ms: now_unix_ms(),
            outcome,
            reason: None,
            failure_class: None,
            stt_engine: None,
            tts_engine: None,
            transcript_len: None,
            stt_latency_ms: m.stt_latency_ms(),
            partial_latency_ms: m.partial_latency_ms(),
            llm_ttft_ms: m.llm_ttft_ms(),
            tts_gen_ms: m.tts_gen_ms(),
            playback_start_ms: m.playback_start_ms(),
            end_to_end_ms: m.end_to_end_ms(),
            ttfa_budget_ms: Some(m.ttfa_budget_ms),
            ttfa_overrun: m.ttfa_overrun(),
            metrics: Some(m.clone()),
        }
    }

    /// Build a failure-turn record from an error/reason string.
    pub fn from_error(seq: u64, outcome: TurnOutcome, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let failure_class = Some(FailureClass::classify(&reason));
        Self {
            seq,
            ended_unix_ms: now_unix_ms(),
            outcome,
            reason: Some(reason),
            failure_class,
            stt_engine: None,
            tts_engine: None,
            transcript_len: None,
            stt_latency_ms: None,
            partial_latency_ms: None,
            llm_ttft_ms: None,
            tts_gen_ms: None,
            playback_start_ms: None,
            end_to_end_ms: None,
            ttfa_budget_ms: None,
            ttfa_overrun: false,
            metrics: None,
        }
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn store() -> &'static Mutex<VecDeque<VoiceTurnRecord>> {
    static STORE: OnceLock<Mutex<VecDeque<VoiceTurnRecord>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_TURN_RECORDS)))
}

/// Append a turn record (oldest dropped when full).
pub fn record(rec: VoiceTurnRecord) {
    if let Ok(mut q) = store().lock() {
        if q.len() >= MAX_TURN_RECORDS {
            q.pop_front();
        }
        tracing::info!(
            seq = rec.seq,
            outcome = rec.outcome.as_str(),
            e2e_ms = ?rec.end_to_end_ms,
            failure = ?rec.failure_class,
            "voice turn recorded"
        );
        q.push_back(rec);
    }
}

/// Snapshot the most-recent `limit` records (newest last).
pub fn snapshot(limit: usize) -> Vec<VoiceTurnRecord> {
    match store().lock() {
        Ok(q) => {
            let n = q.len().min(limit.max(1));
            q.iter().skip(q.len() - n).cloned().collect()
        }
        Err(_) => Vec::new(),
    }
}

/// Aggregate health over the retained window.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceHealthAggregate {
    pub turns: usize,
    pub completed: usize,
    pub empty: usize,
    pub errors: usize,
    pub barge_ins: usize,
    pub timeouts: usize,
    /// p50 end-to-end latency (ms) over completed turns with a measurement.
    pub e2e_p50_ms: Option<u64>,
    /// p95 end-to-end latency (ms).
    pub e2e_p95_ms: Option<u64>,
    pub ttfa_overruns: usize,
    /// Most frequent failure class in the window, if any.
    pub top_failure: Option<FailureClass>,
}

/// Compute aggregate health over the retained window.
pub fn aggregate() -> VoiceHealthAggregate {
    let snap = snapshot(MAX_TURN_RECORDS);
    let mut agg = VoiceHealthAggregate {
        turns: snap.len(),
        ..Default::default()
    };
    let mut e2e: Vec<u64> = Vec::new();
    let mut fail_counts: std::collections::HashMap<FailureClass, usize> =
        std::collections::HashMap::new();
    for r in &snap {
        match r.outcome {
            TurnOutcome::Completed => agg.completed += 1,
            TurnOutcome::EmptyTranscript => agg.empty += 1,
            TurnOutcome::Error => agg.errors += 1,
            TurnOutcome::BargeIn => agg.barge_ins += 1,
            TurnOutcome::Timeout => agg.timeouts += 1,
            TurnOutcome::Busy => {}
        }
        if r.ttfa_overrun {
            agg.ttfa_overruns += 1;
        }
        if let Some(e) = r.end_to_end_ms {
            e2e.push(e);
        }
        if let Some(fc) = r.failure_class {
            *fail_counts.entry(fc).or_insert(0) += 1;
        }
    }
    if !e2e.is_empty() {
        e2e.sort_unstable();
        let p = |q: f64| -> u64 {
            let idx = ((e2e.len() as f64 - 1.0) * q).round() as usize;
            e2e[idx.min(e2e.len() - 1)]
        };
        agg.e2e_p50_ms = Some(p(0.5));
        agg.e2e_p95_ms = Some(p(0.95));
    }
    agg.top_failure = fail_counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(fc, _)| fc);
    agg
}

/// Clear all records (test/diagnostic reset).
pub fn clear() {
    if let Ok(mut q) = store().lock() {
        q.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_failures() {
        assert_eq!(
            FailureClass::classify("empty utterance"),
            FailureClass::SttEmpty
        );
        assert_eq!(
            FailureClass::classify("stt sidecar not ready"),
            FailureClass::SttSidecarUnavailable
        );
        assert_eq!(
            FailureClass::classify("whisper-rs inference failed: failed to encode"),
            FailureClass::SttDecode
        );
        assert_eq!(
            FailureClass::classify("speech GPU lease unavailable"),
            FailureClass::GpuLease
        );
        assert_eq!(
            FailureClass::classify("turn exceeded max duration"),
            FailureClass::TurnTimeout
        );
        assert_eq!(
            FailureClass::classify("stt stream cancelled"),
            FailureClass::Cancelled
        );
        assert_eq!(FailureClass::classify("weird"), FailureClass::Other);
    }

    #[test]
    fn ring_buffer_and_aggregate_sequential() {
        // Single test: the store is a process-global singleton, so stateful
        // assertions must not run concurrently with each other.
        clear();
        for i in 0..(MAX_TURN_RECORDS + 10) as u64 {
            record(VoiceTurnRecord::from_error(i, TurnOutcome::Error, "boom"));
        }
        let snap = snapshot(1000);
        assert_eq!(snap.len(), MAX_TURN_RECORDS);
        assert_eq!(snap.first().unwrap().seq, 10);
        assert_eq!(snap.last().unwrap().seq, (MAX_TURN_RECORDS + 9) as u64);

        clear();
        for i in 0..10u64 {
            let mut r = VoiceTurnRecord::from_error(i, TurnOutcome::Completed, "");
            r.outcome = TurnOutcome::Completed;
            r.reason = None;
            r.failure_class = None;
            r.end_to_end_ms = Some((i + 1) * 100);
            record(r);
        }
        let agg = aggregate();
        assert_eq!(agg.turns, 10);
        assert_eq!(agg.completed, 10);
        assert!(agg.e2e_p50_ms.is_some());
        assert!(agg.e2e_p95_ms.unwrap() >= agg.e2e_p50_ms.unwrap());
        clear();
    }
}
