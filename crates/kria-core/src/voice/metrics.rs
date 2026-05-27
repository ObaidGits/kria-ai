//! Per-turn voice pipeline telemetry.
//!
//! Records the five canonical timestamps of a voice turn so we can verify
//! the per-tier TTFA (time-to-first-audio-out) budget at runtime and in CI.
//! Without these metrics, the "sub-500ms" goal is unfalsifiable.
//!
//! Timeline (all monotonic, relative to `t_speech_end`):
//!
//! ```text
//! t_speech_end ─┬─► t_first_partial   ─► (partial transcripts shown in UI)
//!                │
//!                ├─► t_final          ─► (VAD end + final whisper pass done)
//!                │
//!                ├─► t_post_edit      ─► (Hinglish fix-pass returned, if run)
//!                │
//!                └─► t_first_audio_out ─► (first PCM chunk hits speakers)
//! ```
//!
//! `t_first_audio_out − t_speech_end` is the TTFA the user perceives.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::tier::VoiceTier;

/// Partial update record for stability/flicker computation (§16).
#[derive(Debug, Clone)]
struct PartialRecord {
    text: String,
    #[allow(dead_code)]
    seq: u64,
    /// Timestamp relative to turn start for latency analysis.
    #[allow(dead_code)]
    elapsed_ms: u64,
}

/// Builder-style metrics collector. Construct one per voice turn at
/// `SpeechEnd`, mutate as later milestones land, then call
/// [`MetricsBuilder::finalise`] to obtain a [`VoiceMetrics`] snapshot.
#[derive(Debug, Clone)]
pub struct MetricsBuilder {
    started: Instant,
    tier: VoiceTier,
    mic_capture: Option<Duration>,
    vad_trigger: Option<Duration>,
    stt_first_token: Option<Duration>,
    llm_first_token: Option<Duration>,
    tts_first_chunk: Option<Duration>,
    first_partial: Option<Duration>,
    final_transcript: Option<Duration>,
    post_edit: Option<Duration>,
    first_audio_out: Option<Duration>,
    post_edit_skipped: bool,

    // ─── §16 Evaluation Tracking ──────────────────────────────────────────
    /// Bounded history of partial updates for stability/flicker computation.
    /// Max 128 entries (8-32s @ 4-15 Hz).
    partial_history: Vec<PartialRecord>,
    /// Reconciliation outcome for this turn (if reconciliation ran).
    reconcile_outcome: Option<crate::voice::reconcile::ReconcileKind>,
}

impl MetricsBuilder {
    /// Begin recording at the moment VAD reports `SpeechEnd`.
    pub fn begin_at_speech_end(tier: VoiceTier) -> Self {
        Self {
            started: Instant::now(),
            tier,
            mic_capture: None,
            vad_trigger: None,
            stt_first_token: None,
            llm_first_token: None,
            tts_first_chunk: None,
            first_partial: None,
            final_transcript: None,
            post_edit: None,
            first_audio_out: None,
            post_edit_skipped: false,
            partial_history: Vec::with_capacity(128),
            reconcile_outcome: None,
        }
    }

    pub fn mark_first_partial(&mut self) {
        if self.first_partial.is_none() {
            self.first_partial = Some(self.started.elapsed());
        }
        if self.stt_first_token.is_none() {
            self.stt_first_token = self.first_partial;
        }
    }

    /// Record a partial update for stability/flicker tracking (§16).
    /// Bounded to 128 entries; oldest dropped when full.
    pub fn record_partial(&mut self, text: String, seq: u64) {
        const MAX_PARTIAL_HISTORY: usize = 128;
        if self.partial_history.len() >= MAX_PARTIAL_HISTORY {
            self.partial_history.remove(0);
        }
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        self.partial_history.push(PartialRecord {
            text,
            seq,
            elapsed_ms,
        });
    }

    /// Record reconciliation outcome for rollback rate tracking (§16).
    pub fn record_reconcile(&mut self, kind: crate::voice::reconcile::ReconcileKind) {
        self.reconcile_outcome = Some(kind);
    }

    pub fn mark_final(&mut self) {
        self.final_transcript = Some(self.started.elapsed());
    }

    pub fn mark_post_edit(&mut self) {
        self.post_edit = Some(self.started.elapsed());
    }

    /// Indicate that no post-edit was run for this turn (high confidence,
    /// pure-English, etc.). Distinct from "post-edit timed out".
    pub fn skip_post_edit(&mut self) {
        self.post_edit_skipped = true;
    }

    pub fn mark_first_audio_out(&mut self) {
        if self.first_audio_out.is_none() {
            self.first_audio_out = Some(self.started.elapsed());
        }
    }

    pub fn mark_mic_capture(&mut self) {
        if self.mic_capture.is_none() {
            self.mic_capture = Some(self.started.elapsed());
        }
    }

    pub fn mark_vad_trigger(&mut self) {
        if self.vad_trigger.is_none() {
            self.vad_trigger = Some(self.started.elapsed());
        }
    }

    pub fn mark_llm_first_token(&mut self) {
        if self.llm_first_token.is_none() {
            self.llm_first_token = Some(self.started.elapsed());
        }
    }

    pub fn mark_tts_first_chunk(&mut self) {
        if self.tts_first_chunk.is_none() {
            self.tts_first_chunk = Some(self.started.elapsed());
        }
    }

    /// Compute partial stability: fraction of updates that are prefix
    /// extensions of the previous update (§16 target ≥ 0.85).
    fn compute_partial_stability(&self) -> Option<f32> {
        if self.partial_history.len() < 2 {
            return None;
        }
        let mut prefix_extends = 0usize;
        let mut total_transitions = 0usize;
        for window in self.partial_history.windows(2) {
            let prev = &window[0].text;
            let curr = &window[1].text;
            total_transitions += 1;
            if curr.starts_with(prev) {
                prefix_extends += 1;
            }
        }
        if total_transitions == 0 {
            return None;
        }
        Some(prefix_extends as f32 / total_transitions as f32)
    }

    /// Compute flicker rate: fraction of updates with edit distance > 6
    /// chars vs previous (§16 target ≤ 0.05).
    fn compute_flicker_rate(&self) -> Option<f32> {
        if self.partial_history.len() < 2 {
            return None;
        }
        let mut flickers = 0usize;
        let mut total_transitions = 0usize;
        for window in self.partial_history.windows(2) {
            let prev = &window[0].text;
            let curr = &window[1].text;
            total_transitions += 1;
            let distance = edit_distance_chars(prev, curr);
            if distance > 6 {
                flickers += 1;
            }
        }
        if total_transitions == 0 {
            return None;
        }
        Some(flickers as f32 / total_transitions as f32)
    }

    pub fn finalise(self) -> VoiceMetrics {
        let partial_stability = self.compute_partial_stability();
        let flicker_rate = self.compute_flicker_rate();
        let rollback_rate = self.reconcile_outcome.map(|kind| {
            if kind == crate::voice::reconcile::ReconcileKind::Reject {
                1.0
            } else {
                0.0
            }
        });

        // Compute commit latency (§16): speech_end → UtteranceCommitted
        let commit_latency_ms = match (self.vad_trigger, self.final_transcript) {
            (Some(vad), Some(final_t)) => {
                Some(final_t.as_millis().saturating_sub(vad.as_millis()) as u64)
            }
            _ => None,
        };

        // Compute refine latency (§16): commit → S4 (post-edit done)
        let refine_latency_ms = if self.post_edit_skipped {
            Some(0) // Skipped = 0 latency
        } else {
            match (self.final_transcript, self.post_edit) {
                (Some(final_t), Some(post)) => {
                    Some(post.as_millis().saturating_sub(final_t.as_millis()) as u64)
                }
                _ => None,
            }
        };

        VoiceMetrics {
            tier: self.tier,
            ttfa_budget_ms: self.tier.ttfa_budget_ms(),
            t_mic_capture_ms: self.mic_capture.map(|d| d.as_millis() as u64),
            t_vad_trigger_ms: self.vad_trigger.map(|d| d.as_millis() as u64),
            t_stt_first_token_ms: self.stt_first_token.map(|d| d.as_millis() as u64),
            t_llm_first_token_ms: self.llm_first_token.map(|d| d.as_millis() as u64),
            t_tts_first_chunk_ms: self.tts_first_chunk.map(|d| d.as_millis() as u64),
            t_first_partial_ms: self.first_partial.map(|d| d.as_millis() as u64),
            t_final_ms: self.final_transcript.map(|d| d.as_millis() as u64),
            t_post_edit_ms: if self.post_edit_skipped {
                None
            } else {
                self.post_edit.map(|d| d.as_millis() as u64)
            },
            t_first_audio_out_ms: self.first_audio_out.map(|d| d.as_millis() as u64),
            post_edit_skipped: self.post_edit_skipped,
            partial_stability,
            flicker_rate,
            rollback_rate,
            wer: None, // eval-only, not runtime
            commit_latency_ms,
            refine_latency_ms,
        }
    }
}

/// Simple character-level edit distance for flicker detection.
/// Not optimized; only used at turn finalization.
fn edit_distance_chars(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();

    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        dp[i][0] = i;
    }
    for j in 0..=m {
        dp[0][j] = j;
    }

    for i in 1..=n {
        for j in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }

    dp[n][m]
}

/// Snapshot emitted as a `voice:metrics` Tauri event after each turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceMetrics {
    pub tier: VoiceTier,
    /// Per-tier TTFA budget that this turn was measured against.
    pub ttfa_budget_ms: u64,
    pub t_mic_capture_ms: Option<u64>,
    pub t_vad_trigger_ms: Option<u64>,
    pub t_stt_first_token_ms: Option<u64>,
    pub t_llm_first_token_ms: Option<u64>,
    pub t_tts_first_chunk_ms: Option<u64>,
    pub t_first_partial_ms: Option<u64>,
    pub t_final_ms: Option<u64>,
    /// `None` if `post_edit_skipped` is true OR if post-edit hasn't fired yet.
    pub t_post_edit_ms: Option<u64>,
    pub t_first_audio_out_ms: Option<u64>,
    pub post_edit_skipped: bool,

    // ─── §16 Evaluation Metrics ───────────────────────────────────────────
    /// Fraction of partial updates that are prefix extensions of previous
    /// (target ≥ 0.85 on English subset). `None` if insufficient data.
    pub partial_stability: Option<f32>,
    /// Count of UI updates where visible string edit distance > 6 chars vs
    /// previous / total updates (target ≤ 0.05). `None` if no partials.
    pub flicker_rate: Option<f32>,
    /// Fraction of turns where §7 reconciliation yields `rejected`.
    /// Tracked across turns, not per-turn. `None` if no reconciliation run.
    pub rollback_rate: Option<f32>,
    /// Word error rate vs reference transcript (eval/CI only, not runtime).
    /// `None` in production; populated by eval harness.
    pub wer: Option<f32>,

    // ─── §16 Latency Metrics ──────────────────────────────────────────────
    /// `speech_end` → `UtteranceCommitted` p50/p95 (commit latency).
    /// Computed from t_final_ms - t_vad_trigger_ms.
    pub commit_latency_ms: Option<u64>,
    /// commit → `S4` p50/p95 (refine latency).
    /// Computed from t_post_edit_ms - t_final_ms (or 0 if skipped).
    pub refine_latency_ms: Option<u64>,
}

impl VoiceMetrics {
    /// `true` when the user-perceived latency exceeded the tier budget.
    pub fn ttfa_overrun(&self) -> bool {
        self.t_first_audio_out_ms
            .map(|t| t > self.ttfa_budget_ms)
            .unwrap_or(false)
    }
}

/// Rolling overrun counter. Three consecutive `ttfa_overrun()` turns trigger
/// a `voice:degraded` event so the UI can offer to demote the tier.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverrunTracker {
    consecutive: u8,
}

impl OverrunTracker {
    /// Record a turn. Returns `true` exactly once when the threshold is
    /// crossed (so the caller can emit the degraded event without spamming).
    pub fn record(&mut self, m: &VoiceMetrics) -> bool {
        if m.ttfa_overrun() {
            self.consecutive = self.consecutive.saturating_add(1);
            if self.consecutive == 3 {
                return true;
            }
        } else {
            self.consecutive = 0;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn builder_records_in_order() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        sleep(Duration::from_millis(2));
        b.mark_first_partial();
        b.mark_final();
        b.skip_post_edit();
        b.mark_first_audio_out();
        let m = b.finalise();
        assert_eq!(m.tier, VoiceTier::A);
        assert_eq!(m.ttfa_budget_ms, 800);
        assert!(m.t_first_partial_ms.is_some());
        assert!(m.t_final_ms.is_some());
        assert!(m.t_post_edit_ms.is_none());
        assert!(m.post_edit_skipped);
    }

    #[test]
    fn overrun_tracker_fires_on_third_overrun_only() {
        let mut t = OverrunTracker::default();
        let bad = VoiceMetrics {
            tier: VoiceTier::S,
            ttfa_budget_ms: 500,
            t_mic_capture_ms: Some(10),
            t_vad_trigger_ms: Some(20),
            t_stt_first_token_ms: Some(100),
            t_llm_first_token_ms: Some(200),
            t_tts_first_chunk_ms: Some(300),
            t_first_partial_ms: Some(100),
            t_final_ms: Some(200),
            t_post_edit_ms: None,
            t_first_audio_out_ms: Some(900),
            post_edit_skipped: true,
            partial_stability: None,
            flicker_rate: None,
            rollback_rate: None,
            wer: None,
            commit_latency_ms: Some(180),
            refine_latency_ms: Some(0),
        };
        assert!(!t.record(&bad));
        assert!(!t.record(&bad));
        assert!(t.record(&bad), "third consecutive overrun should fire");
        // does not refire on the fourth
        assert!(!t.record(&bad));
    }

    #[test]
    fn good_turn_resets_streak() {
        let mut t = OverrunTracker::default();
        let bad = VoiceMetrics {
            tier: VoiceTier::S,
            ttfa_budget_ms: 500,
            t_mic_capture_ms: Some(10),
            t_vad_trigger_ms: Some(20),
            t_stt_first_token_ms: None,
            t_llm_first_token_ms: None,
            t_tts_first_chunk_ms: None,
            t_first_partial_ms: None,
            t_final_ms: None,
            t_post_edit_ms: None,
            t_first_audio_out_ms: Some(900),
            post_edit_skipped: true,
            partial_stability: None,
            flicker_rate: None,
            rollback_rate: None,
            wer: None,
            commit_latency_ms: None,
            refine_latency_ms: Some(0),
        };
        let good = VoiceMetrics {
            t_first_audio_out_ms: Some(300),
            ..bad.clone()
        };
        t.record(&bad);
        t.record(&bad);
        t.record(&good);
        assert!(!t.record(&bad), "streak should have reset");
    }

    #[test]
    fn builder_records_phase1_milestones() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.mark_mic_capture();
        b.mark_vad_trigger();
        b.mark_first_partial();
        b.mark_llm_first_token();
        b.mark_tts_first_chunk();
        b.mark_final();
        b.mark_first_audio_out();
        let m = b.finalise();
        assert!(m.t_mic_capture_ms.is_some());
        assert!(m.t_vad_trigger_ms.is_some());
        assert!(m.t_stt_first_token_ms.is_some());
        assert!(m.t_llm_first_token_ms.is_some());
        assert!(m.t_tts_first_chunk_ms.is_some());
        assert!(m.t_final_ms.is_some());
        assert!(m.t_first_audio_out_ms.is_some());
    }

    #[test]
    fn partial_stability_computed_correctly() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        // All prefix extensions
        b.record_partial("hello".into(), 1);
        b.record_partial("hello world".into(), 2);
        b.record_partial("hello world how".into(), 3);
        let m = b.finalise();
        assert_eq!(m.partial_stability, Some(1.0));
    }

    #[test]
    fn partial_stability_with_non_prefix() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.record_partial("hello".into(), 1);
        b.record_partial("hello world".into(), 2);
        b.record_partial("goodbye".into(), 3); // not a prefix
        let m = b.finalise();
        // 1 prefix extend out of 2 transitions = 0.5
        assert_eq!(m.partial_stability, Some(0.5));
    }

    #[test]
    fn partial_stability_none_when_insufficient_data() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.record_partial("hello".into(), 1);
        let m = b.finalise();
        assert_eq!(m.partial_stability, None);
    }

    #[test]
    fn flicker_rate_computed_correctly() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        // Small changes (≤6 chars edit distance)
        b.record_partial("hello".into(), 1);
        b.record_partial("hello!".into(), 2); // +1 char
        b.record_partial("hello world".into(), 3); // +6 chars
        let m = b.finalise();
        // 0 flickers out of 2 transitions
        assert_eq!(m.flicker_rate, Some(0.0));
    }

    #[test]
    fn flicker_rate_detects_large_changes() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.record_partial("hello".into(), 1);
        b.record_partial("completely different text".into(), 2); // >6 chars
        let m = b.finalise();
        // 1 flicker out of 1 transition
        assert_eq!(m.flicker_rate, Some(1.0));
    }

    #[test]
    fn rollback_rate_tracks_rejection() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.record_reconcile(crate::voice::reconcile::ReconcileKind::Reject);
        let m = b.finalise();
        assert_eq!(m.rollback_rate, Some(1.0));
    }

    #[test]
    fn rollback_rate_tracks_non_rejection() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.record_reconcile(crate::voice::reconcile::ReconcileKind::Identical);
        let m = b.finalise();
        assert_eq!(m.rollback_rate, Some(0.0));
    }

    #[test]
    fn rollback_rate_none_when_no_reconciliation() {
        let b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        let m = b.finalise();
        assert_eq!(m.rollback_rate, None);
    }

    #[test]
    fn partial_history_bounded_to_128() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        for i in 0..200 {
            b.record_partial(format!("text_{}", i), i);
        }
        assert_eq!(b.partial_history.len(), 128);
        // Should have dropped oldest 72 entries
        assert_eq!(b.partial_history[0].seq, 72);
        assert_eq!(b.partial_history[127].seq, 199);
    }

    #[test]
    fn wer_always_none_in_runtime() {
        let b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        let m = b.finalise();
        assert_eq!(m.wer, None);
    }

    #[test]
    fn edit_distance_basic_cases() {
        assert_eq!(edit_distance_chars("", ""), 0);
        assert_eq!(edit_distance_chars("hello", "hello"), 0);
        assert_eq!(edit_distance_chars("hello", "hallo"), 1);
        assert_eq!(edit_distance_chars("hello", ""), 5);
        assert_eq!(edit_distance_chars("", "world"), 5);
        assert_eq!(edit_distance_chars("hello", "world"), 4);
    }

    #[test]
    fn commit_latency_computed_correctly() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        std::thread::sleep(Duration::from_millis(10));
        b.mark_vad_trigger();
        std::thread::sleep(Duration::from_millis(50));
        b.mark_final();
        let m = b.finalise();
        assert!(m.commit_latency_ms.is_some());
        let latency = m.commit_latency_ms.unwrap();
        // Should be approximately 50ms (between vad_trigger and final)
        assert!(latency >= 40 && latency <= 100, "latency was {}", latency);
    }

    #[test]
    fn refine_latency_computed_correctly() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.mark_final();
        std::thread::sleep(Duration::from_millis(20));
        b.mark_post_edit();
        let m = b.finalise();
        assert!(m.refine_latency_ms.is_some());
        let latency = m.refine_latency_ms.unwrap();
        // Should be approximately 20ms (between final and post_edit)
        assert!(latency >= 10 && latency <= 50, "latency was {}", latency);
    }

    #[test]
    fn refine_latency_zero_when_skipped() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.mark_final();
        b.skip_post_edit();
        let m = b.finalise();
        assert_eq!(m.refine_latency_ms, Some(0));
    }

    #[test]
    fn commit_latency_none_when_missing_timestamps() {
        let b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        let m = b.finalise();
        assert_eq!(m.commit_latency_ms, None);
    }

    #[test]
    fn refine_latency_none_when_not_run() {
        let mut b = MetricsBuilder::begin_at_speech_end(VoiceTier::A);
        b.mark_final();
        // Don't mark post_edit or skip it
        let m = b.finalise();
        assert_eq!(m.refine_latency_ms, None);
    }
}
