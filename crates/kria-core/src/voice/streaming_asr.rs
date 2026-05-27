//! Dual-Layer Streaming ASR — Fast speculative partials + accurate refinement.
//!
//! ## Architecture
//!
//! ```text
//! Audio chunks (100ms, 16kHz)
//!     │
//!     ├─► Layer 1: FAST (whisper-tiny, 1s window, 200ms cadence)
//!     │   └─► Speculative partials → UI (immediate feel)
//!     │
//!     └─► Layer 2: ACCURATE (whisper-base/medium, full buffer, on commit)
//!         └─► Final transcript → execution (correctness)
//! ```
//!
//! ## Design Principles
//! - Layer 1 optimizes for SPEED (tiny model, short window, frequent decodes)
//! - Layer 2 optimizes for ACCURACY (larger model, full audio, single decode)
//! - Execution ALWAYS uses Layer 2 output (transcript authority preserved)
//! - Layer 1 partials are advisory/speculative (UI only)
//! - Both layers are cancellation-safe and generation-safe
//!
//! ## Key Insight
//! Humans tolerate imperfect partials. Humans do NOT tolerate silence.
//! Layer 1 gives immediate feedback. Layer 2 gives correct execution.

use serde::{Deserialize, Serialize};
use std::time::Instant;

// ─── Streaming Decode Config ──────────────────────────────────────────────

/// Configuration for the fast streaming layer.
#[derive(Debug, Clone)]
pub struct StreamingAsrConfig {
    /// Rolling window size for fast partials (ms). Smaller = faster, less context.
    pub fast_window_ms: u64,
    /// Decode cadence for fast layer (ms). How often to produce a partial.
    pub fast_cadence_ms: u64,
    /// Minimum audio before first decode attempt (ms).
    pub min_audio_for_decode_ms: u64,
    /// Maximum concurrent fast decodes (should be 1 for whisper).
    pub max_concurrent_decodes: usize,
    /// Whether to use incremental stabilization (prefix-hold).
    pub incremental_stabilization: bool,
    /// Stability threshold: consecutive matching prefixes before "stable".
    pub stability_threshold: usize,
}

impl Default for StreamingAsrConfig {
    fn default() -> Self {
        Self {
            fast_window_ms: 1_500,        // 1.5s rolling window for fast layer
            fast_cadence_ms: 200,         // Decode every 200ms
            min_audio_for_decode_ms: 300, // Wait 300ms before first decode
            max_concurrent_decodes: 1,
            incremental_stabilization: true,
            stability_threshold: 2, // 2 consecutive matching prefixes = stable
        }
    }
}

impl StreamingAsrConfig {
    /// Ultra-responsive config for powerful hardware (CUDA).
    pub fn fast() -> Self {
        Self {
            fast_window_ms: 1_000,
            fast_cadence_ms: 150,
            min_audio_for_decode_ms: 200,
            ..Default::default()
        }
    }

    /// Conservative config for CPU-only or loaded systems.
    pub fn conservative() -> Self {
        Self {
            fast_window_ms: 2_000,
            fast_cadence_ms: 350,
            min_audio_for_decode_ms: 500,
            ..Default::default()
        }
    }
}

// ─── Streaming Partial ────────────────────────────────────────────────────

/// A streaming partial from the fast layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingPartial {
    /// Speculative text (may change on next decode).
    pub text: String,
    /// Stable prefix (won't change — confirmed by consecutive decodes).
    pub stable_prefix: String,
    /// Volatile suffix (may change on next decode).
    pub volatile_suffix: String,
    /// Monotonic sequence number.
    pub seq: u64,
    /// Decode latency for this partial (ms).
    pub decode_ms: u64,
    /// Audio duration decoded (ms).
    pub audio_ms: u64,
    /// Whether this partial is considered stable.
    pub is_stable: bool,
    /// Layer that produced this partial.
    pub layer: AsrLayer,
}

/// Which ASR layer produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrLayer {
    /// Fast speculative layer (tiny model, short window).
    Fast,
    /// Accurate refinement layer (larger model, full buffer).
    Accurate,
}

// ─── Incremental Stabilizer ───────────────────────────────────────────────

/// Tracks partial stability across consecutive decodes.
///
/// A prefix becomes "stable" when it appears unchanged in N consecutive
/// partial decodes. This gives the UI a stable region to display without
/// flicker, while the volatile tail updates freely.
#[derive(Debug, Clone)]
pub struct IncrementalStabilizer {
    /// Current stable prefix (confirmed by consecutive matches).
    stable_prefix: String,
    /// Last full partial text.
    last_text: String,
    /// Consecutive decodes where the prefix matched.
    consecutive_matches: usize,
    /// Threshold for stability.
    threshold: usize,
    /// Total partials processed.
    total_partials: u64,
    /// Total stability transitions.
    stability_transitions: u64,
}

impl IncrementalStabilizer {
    pub fn new(threshold: usize) -> Self {
        Self {
            stable_prefix: String::new(),
            last_text: String::new(),
            consecutive_matches: 0,
            threshold: threshold.max(1),
            total_partials: 0,
            stability_transitions: 0,
        }
    }

    /// Process a new partial and return the stable/volatile split.
    pub fn process(&mut self, text: &str) -> (String, String, bool) {
        self.total_partials += 1;

        if text.is_empty() {
            return (
                self.stable_prefix.clone(),
                String::new(),
                !self.stable_prefix.is_empty(),
            );
        }

        // Check if new text shares a word-prefix with the last text
        let common_prefix = common_word_prefix(&self.last_text, text);

        if !common_prefix.is_empty() {
            self.consecutive_matches += 1;

            // Promote to stable if threshold met and prefix is longer than current stable
            if self.consecutive_matches >= self.threshold
                && common_prefix.len() > self.stable_prefix.len()
            {
                self.stable_prefix = common_prefix;
                self.stability_transitions += 1;
            }
        } else if !self.last_text.is_empty() {
            // Complete divergence — reset consecutive counter but keep stable prefix
            self.consecutive_matches = 0;
        }

        self.last_text = text.to_string();

        let volatile =
            if text.len() > self.stable_prefix.len() && text.starts_with(&self.stable_prefix) {
                text[self.stable_prefix.len()..].to_string()
            } else {
                text.to_string()
            };

        let is_stable = !self.stable_prefix.is_empty();
        (self.stable_prefix.clone(), volatile, is_stable)
    }

    /// Reset for new utterance.
    pub fn reset(&mut self) {
        self.stable_prefix.clear();
        self.last_text.clear();
        self.consecutive_matches = 0;
    }

    /// Current stable prefix.
    pub fn stable_prefix(&self) -> &str {
        &self.stable_prefix
    }

    /// Stability rate (transitions / total partials).
    pub fn stability_rate(&self) -> f64 {
        if self.total_partials == 0 {
            return 0.0;
        }
        self.stability_transitions as f64 / self.total_partials as f64
    }
}

/// Find the longest common word-boundary prefix between two strings.
fn common_word_prefix(a: &str, b: &str) -> String {
    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();

    let mut common_words = 0;
    for (wa, wb) in a_words.iter().zip(b_words.iter()) {
        if wa == wb {
            common_words += 1;
        } else {
            break;
        }
    }

    if common_words == 0 {
        return String::new();
    }

    a_words[..common_words].join(" ")
}

// ─── Decode Cadence Controller ────────────────────────────────────────────

/// Adaptive decode cadence — speeds up when speech is active, slows during silence.
#[derive(Debug, Clone)]
pub struct DecodeCadenceController {
    /// Base cadence (ms).
    base_cadence_ms: u64,
    /// Current adaptive cadence (ms).
    current_cadence_ms: u64,
    /// Minimum cadence (fastest decode rate).
    min_cadence_ms: u64,
    /// Maximum cadence (slowest decode rate during silence).
    max_cadence_ms: u64,
    /// Last decode timestamp.
    last_decode_at: Option<Instant>,
    /// Last decode duration (ms) — used to avoid scheduling faster than decode can run.
    last_decode_duration_ms: u64,
    /// Consecutive silent frames (no speech detected).
    silent_frames: u64,
}

impl DecodeCadenceController {
    pub fn new(base_cadence_ms: u64) -> Self {
        Self {
            base_cadence_ms,
            current_cadence_ms: base_cadence_ms,
            min_cadence_ms: 100,
            max_cadence_ms: 500,
            last_decode_at: None,
            last_decode_duration_ms: 0,
            silent_frames: 0,
        }
    }

    /// Should we decode now?
    pub fn should_decode(&self, audio_duration_ms: u64, min_audio_ms: u64) -> bool {
        // Don't decode if not enough audio
        if audio_duration_ms < min_audio_ms {
            return false;
        }

        // Don't decode faster than the last decode took
        let effective_cadence = self
            .current_cadence_ms
            .max(self.last_decode_duration_ms + 50);

        match self.last_decode_at {
            Some(last) => last.elapsed().as_millis() as u64 >= effective_cadence,
            None => true, // First decode
        }
    }

    /// Record that a decode just completed.
    pub fn record_decode(&mut self, duration_ms: u64) {
        self.last_decode_at = Some(Instant::now());
        self.last_decode_duration_ms = duration_ms;
        self.silent_frames = 0;

        // Speed up cadence when actively decoding
        self.current_cadence_ms = self.base_cadence_ms.max(self.min_cadence_ms);
    }

    /// Record a silent frame (no speech activity).
    pub fn record_silence(&mut self) {
        self.silent_frames += 1;
        // Slow down cadence during silence (save CPU)
        if self.silent_frames > 5 {
            self.current_cadence_ms = (self.current_cadence_ms + 50).min(self.max_cadence_ms);
        }
    }

    /// Reset for new utterance.
    pub fn reset(&mut self) {
        self.last_decode_at = None;
        self.last_decode_duration_ms = 0;
        self.silent_frames = 0;
        self.current_cadence_ms = self.base_cadence_ms;
    }

    /// Current effective cadence.
    pub fn current_cadence_ms(&self) -> u64 {
        self.current_cadence_ms
    }
}

// ─── Streaming Latency Tracker ────────────────────────────────────────────

/// Tracks realtime streaming latency metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingLatencyMetrics {
    /// Time from first audio to first partial (ms).
    pub first_partial_ms: Option<u64>,
    /// Time from first audio to first stable partial (ms).
    pub first_stable_ms: Option<u64>,
    /// Average decode latency (ms).
    pub avg_decode_ms: u64,
    /// Peak decode latency (ms).
    pub peak_decode_ms: u64,
    /// Total partials emitted.
    pub total_partials: u64,
    /// Total stable transitions.
    pub stable_transitions: u64,
    /// Decode cadence achieved (Hz).
    pub decode_hz: f64,
}

/// Collects streaming latency measurements during a turn.
#[derive(Debug)]
pub struct StreamingLatencyCollector {
    turn_start: Instant,
    first_partial_at: Option<Instant>,
    first_stable_at: Option<Instant>,
    decode_durations: Vec<u64>,
    partial_count: u64,
    stable_count: u64,
}

impl StreamingLatencyCollector {
    pub fn new() -> Self {
        Self {
            turn_start: Instant::now(),
            first_partial_at: None,
            first_stable_at: None,
            decode_durations: Vec::with_capacity(64),
            partial_count: 0,
            stable_count: 0,
        }
    }

    /// Record a partial emission.
    pub fn record_partial(&mut self, decode_ms: u64, is_stable: bool) {
        self.partial_count += 1;
        self.decode_durations.push(decode_ms);

        if self.first_partial_at.is_none() {
            self.first_partial_at = Some(Instant::now());
        }

        if is_stable {
            self.stable_count += 1;
            if self.first_stable_at.is_none() {
                self.first_stable_at = Some(Instant::now());
            }
        }
    }

    /// Finalize and produce metrics.
    pub fn finalize(&self) -> StreamingLatencyMetrics {
        let avg_decode = if self.decode_durations.is_empty() {
            0
        } else {
            self.decode_durations.iter().sum::<u64>() / self.decode_durations.len() as u64
        };

        let peak_decode = self.decode_durations.iter().copied().max().unwrap_or(0);

        let elapsed_s = self.turn_start.elapsed().as_secs_f64().max(0.001);
        let decode_hz = self.partial_count as f64 / elapsed_s;

        StreamingLatencyMetrics {
            first_partial_ms: self
                .first_partial_at
                .map(|t| t.duration_since(self.turn_start).as_millis() as u64),
            first_stable_ms: self
                .first_stable_at
                .map(|t| t.duration_since(self.turn_start).as_millis() as u64),
            avg_decode_ms: avg_decode,
            peak_decode_ms: peak_decode,
            total_partials: self.partial_count,
            stable_transitions: self.stable_count,
            decode_hz,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stabilizer_promotes_after_threshold() {
        let mut s = IncrementalStabilizer::new(2);

        let (stable, _volatile, is_stable) = s.process("hello world");
        assert_eq!(stable, "");
        assert!(!is_stable);

        let (stable, _volatile, _is_stable) = s.process("hello world how");
        // "hello world" is common prefix, 1 consecutive match — not yet promoted
        assert_eq!(stable, "");

        let (stable, _volatile, is_stable) = s.process("hello world how are");
        // "hello world how" is common between prev and current, 2 matches → promote
        assert_eq!(stable, "hello world how");
        assert!(is_stable);
    }

    #[test]
    fn stabilizer_doesnt_shrink_prefix() {
        let mut s = IncrementalStabilizer::new(2);
        s.process("hello world");
        s.process("hello world how");
        s.process("hello world how are"); // stable = "hello world how"
        assert_eq!(s.stable_prefix(), "hello world how");

        // Even if next partial diverges, stable prefix doesn't shrink
        s.process("goodbye");
        assert_eq!(s.stable_prefix(), "hello world how");
    }

    #[test]
    fn stabilizer_reset() {
        let mut s = IncrementalStabilizer::new(2);
        s.process("hello");
        s.process("hello world");
        s.process("hello world how");
        s.reset();
        assert_eq!(s.stable_prefix(), "");
    }

    #[test]
    fn common_word_prefix_basic() {
        assert_eq!(
            common_word_prefix("hello world", "hello world how"),
            "hello world"
        );
        assert_eq!(common_word_prefix("hello", "goodbye"), "");
        assert_eq!(common_word_prefix("a b c", "a b d"), "a b");
        assert_eq!(common_word_prefix("", "hello"), "");
    }

    #[test]
    fn cadence_controller_first_decode() {
        let c = DecodeCadenceController::new(200);
        assert!(c.should_decode(500, 300)); // enough audio, no previous decode
    }

    #[test]
    fn cadence_controller_respects_min_audio() {
        let c = DecodeCadenceController::new(200);
        assert!(!c.should_decode(100, 300)); // not enough audio
    }

    #[test]
    fn cadence_controller_respects_cadence() {
        let mut c = DecodeCadenceController::new(200);
        c.record_decode(50);
        // Immediately after decode, should not decode again
        assert!(!c.should_decode(500, 300));
    }

    #[test]
    fn cadence_controller_slows_on_silence() {
        let mut c = DecodeCadenceController::new(200);
        for _ in 0..10 {
            c.record_silence();
        }
        assert!(c.current_cadence_ms() > 200);
    }

    #[test]
    fn cadence_controller_reset() {
        let mut c = DecodeCadenceController::new(200);
        c.record_decode(100);
        c.record_silence();
        c.reset();
        assert_eq!(c.current_cadence_ms(), 200);
        assert!(c.should_decode(500, 300));
    }

    #[test]
    fn streaming_config_presets() {
        let fast = StreamingAsrConfig::fast();
        assert_eq!(fast.fast_cadence_ms, 150);
        assert_eq!(fast.fast_window_ms, 1_000);

        let conservative = StreamingAsrConfig::conservative();
        assert_eq!(conservative.fast_cadence_ms, 350);
        assert_eq!(conservative.fast_window_ms, 2_000);
    }

    #[test]
    fn latency_collector_records() {
        let mut c = StreamingLatencyCollector::new();
        std::thread::sleep(Duration::from_millis(10));
        c.record_partial(50, false);
        c.record_partial(60, true);

        let metrics = c.finalize();
        assert_eq!(metrics.total_partials, 2);
        assert_eq!(metrics.stable_transitions, 1);
        assert!(metrics.first_partial_ms.is_some());
        assert!(metrics.first_stable_ms.is_some());
        assert_eq!(metrics.avg_decode_ms, 55);
        assert_eq!(metrics.peak_decode_ms, 60);
    }

    #[test]
    fn streaming_partial_serialization() {
        let partial = StreamingPartial {
            text: "hello world".to_string(),
            stable_prefix: "hello".to_string(),
            volatile_suffix: " world".to_string(),
            seq: 3,
            decode_ms: 45,
            audio_ms: 1500,
            is_stable: true,
            layer: AsrLayer::Fast,
        };
        let json = serde_json::to_string(&partial).unwrap();
        assert!(json.contains("\"layer\":\"fast\""));
        let deserialized: StreamingPartial = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.seq, 3);
    }
}
