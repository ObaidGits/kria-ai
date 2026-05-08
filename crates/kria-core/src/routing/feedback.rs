//! Online learning feedback collection and centroid adjustment.
//!
//! Collects routing outcomes from user behavior signals and periodically
//! adjusts domain centroids to improve routing accuracy over time.
//!
//! # Architecture
//!
//! ```text
//! User completes turn → detect_outcome() → record(RoutingFeedback)
//!                                              ↓
//!                                    FeedbackCollector buffer
//!                                              ↓ (flush at max_buffer)
//!                                    ~/.kria/feedback/routing_feedback.jsonl
//!                                              ↓ (nightly or on-demand)
//!                                    adjust_centroids()
//!                                              ↓
//!                                    save_centroids() → domain_centroids.v1.bin
//! ```
//!
//! # Learning Signals
//!
//! | Signal | Meaning | Centroid Effect |
//! |--------|---------|-----------------|
//! | Success | Tool worked, user moved on | Pull centroid toward embedding (+1.0x) |
//! | Rephrased | User rephrased same request | Push centroid away (-0.5x) |
//! | Corrected | User explicitly corrected | Push away from wrong, pull toward correct (±2.0x) |
//! | BargedIn | User interrupted response | Weak negative signal (-0.3x) |
//! | HitlDenied | Safety denied the action | No centroid change |
//! | ToolError | Tool execution failed | No centroid change |

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::domain::Domain;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Default learning rate for centroid adjustment.
const DEFAULT_LEARNING_RATE: f32 = 0.01;

/// Default maximum buffer size before flush.
const DEFAULT_MAX_BUFFER: usize = 1000;

/// Default feedback directory.
const DEFAULT_FEEDBACK_DIR: &str = "~/.kria/feedback";

/// Feedback file name.
const FEEDBACK_FILENAME: &str = "routing_feedback.jsonl";

// ─── Routing Feedback ───────────────────────────────────────────────────────

/// A single routing feedback entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingFeedback {
    /// Hash of the input text (for dedup, not the text itself).
    pub input_text_hash: u64,
    /// Domain that was selected for routing.
    pub domain_selected: Domain,
    /// Tool name that was selected (if any).
    pub tool_selected: Option<String>,
    /// Source of the routing decision.
    pub intent_source: String,
    /// Confidence of the routing decision.
    pub confidence: f32,
    /// Detected outcome of the routing.
    pub outcome: RoutingOutcome,
    /// Unix timestamp of the feedback.
    pub timestamp: u64,
    /// Session ID for grouping.
    pub session_id: String,
    /// The query embedding (for centroid nudging).
    #[serde(skip)]
    pub embedding: Vec<f32>,
}

/// Detected outcome of a routing decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoutingOutcome {
    /// Tool executed successfully, user moved on.
    Success,
    /// User rephrased the same request (routing was wrong).
    Rephrased,
    /// User explicitly corrected the routing.
    Corrected {
        /// The correct domain after correction.
        correct_domain: Domain,
        /// The correct tool after correction (if known).
        correct_tool: Option<String>,
    },
    /// User barged in during response (wrong tool or style).
    BargedIn,
    /// HITL denied the action.
    HitlDenied,
    /// Tool execution failed.
    ToolError {
        /// Error message from tool.
        error: String,
    },
    /// Unknown outcome (no signal detected).
    Unknown,
}

// ─── Centroid Adjustment Report ─────────────────────────────────────────────

/// Report of centroid adjustments made.
#[derive(Debug, Clone, Default)]
pub struct CentroidAdjustmentReport {
    /// Number of success nudges applied.
    pub success_nudges: usize,
    /// Number of rephrase pushes applied.
    pub rephrase_pushes: usize,
    /// Number of correction pushes (away from wrong domain).
    pub correction_pushes: usize,
    /// Number of correction pulls (toward correct domain).
    pub correction_pulls: usize,
    /// Total centroids adjusted.
    pub total_adjusted: usize,
}

// ─── Feedback Collector ─────────────────────────────────────────────────────

/// Collects routing feedback and manages persistence.
pub struct FeedbackCollector {
    /// Pending feedback entries.
    buffer: Vec<RoutingFeedback>,
    /// Maximum buffer size before flush.
    max_buffer: usize,
    /// Path to feedback directory.
    feedback_dir: PathBuf,
}

impl FeedbackCollector {
    /// Create a new feedback collector.
    pub fn new(feedback_dir: &str, max_buffer: usize, _learning_rate: f32) -> Self {
        let dir = if feedback_dir.starts_with('~') {
            if let Some(home) = std::env::var_os("HOME") {
                PathBuf::from(home).join(&feedback_dir[1..])
            } else {
                PathBuf::from(feedback_dir)
            }
        } else {
            PathBuf::from(feedback_dir)
        };

        Self {
            buffer: Vec::new(),
            max_buffer,
            feedback_dir: dir,
        }
    }

    /// Create with default settings.
    pub fn default_config() -> Self {
        Self::new(DEFAULT_FEEDBACK_DIR, DEFAULT_MAX_BUFFER, DEFAULT_LEARNING_RATE)
    }

    /// Record a routing feedback entry.
    pub fn record(&mut self, feedback: RoutingFeedback) {
        self.buffer.push(feedback);

        if self.buffer.len() >= self.max_buffer {
            self.flush_to_disk();
        }
    }

    /// Flush buffer to persistent storage.
    pub fn flush_to_disk(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        // Ensure directory exists
        if let Err(e) = fs::create_dir_all(&self.feedback_dir) {
            warn!("Failed to create feedback directory: {}", e);
            return;
        }

        let path = self.feedback_dir.join(FEEDBACK_FILENAME);
        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to open feedback file: {}", e);
                return;
            }
        };

        for entry in &self.buffer {
            if let Ok(json) = serde_json::to_string(entry) {
                if let Err(e) = writeln!(file, "{}", json) {
                    warn!("Failed to write feedback entry: {}", e);
                }
            }
        }

        let count = self.buffer.len();
        self.buffer.clear();
        debug!("Flushed {} feedback entries to disk", count);
    }

    /// Load all historical feedback from disk.
    pub fn load_history(&self) -> Vec<RoutingFeedback> {
        let path = self.feedback_dir.join(FEEDBACK_FILENAME);
        if !path.exists() {
            return Vec::new();
        }

        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to open feedback history: {}", e);
                return Vec::new();
            }
        };

        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            if let Ok(line) = line {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<RoutingFeedback>(&line) {
                    entries.push(entry);
                }
            }
        }

        entries
    }

    /// Number of entries in the buffer.
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }
}

// ─── Centroid Adjustment ────────────────────────────────────────────────────

/// Adjust domain centroids based on feedback.
///
/// This is the core learning algorithm. It nudges centroids toward successful
/// embeddings and away from incorrect ones.
///
/// # Arguments
///
/// * `feedback` - Slice of feedback entries to process
/// * `centroids` - Mutable reference to domain centroids (will be modified)
/// * `learning_rate` - Base learning rate for adjustments
///
/// # Returns
///
/// `CentroidAdjustmentReport` with statistics about adjustments made.
pub fn adjust_centroids(
    feedback: &[RoutingFeedback],
    centroids: &mut HashMap<Domain, Vec<f32>>,
    learning_rate: f32,
) -> CentroidAdjustmentReport {
    let mut report = CentroidAdjustmentReport::default();

    for entry in feedback {
        if entry.embedding.is_empty() {
            continue;
        }

        match &entry.outcome {
            RoutingOutcome::Success => {
                // Pull centroid toward successful embedding
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, learning_rate);
                    report.success_nudges += 1;
                    report.total_adjusted += 1;
                }
            }
            RoutingOutcome::Rephrased => {
                // Weak negative: push centroid away
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, -learning_rate * 0.5);
                    report.rephrase_pushes += 1;
                    report.total_adjusted += 1;
                }
            }
            RoutingOutcome::Corrected {
                correct_domain, ..
            } => {
                // Strong signal: push away from wrong, pull toward correct
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, -learning_rate * 2.0);
                    report.correction_pushes += 1;
                    report.total_adjusted += 1;
                }
                if let Some(centroid) = centroids.get_mut(correct_domain) {
                    nudge_centroid(centroid, &entry.embedding, learning_rate * 2.0);
                    report.correction_pulls += 1;
                    report.total_adjusted += 1;
                }
            }
            RoutingOutcome::BargedIn => {
                // Weak negative signal
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, -learning_rate * 0.3);
                    report.total_adjusted += 1;
                }
            }
            // HitlDenied, ToolError, Unknown — no centroid change
            _ => {}
        }
    }

    report
}

/// Nudge a centroid toward or away from an embedding.
///
/// Formula: `centroid[i] += rate * (embedding[i] - centroid[i])`
/// Then re-normalize to maintain L2 unit length.
fn nudge_centroid(centroid: &mut [f32], embedding: &[f32], rate: f32) {
    let len = centroid.len().min(embedding.len());
    for i in 0..len {
        centroid[i] += rate * (embedding[i] - centroid[i]);
    }
    // Re-normalize to unit length
    l2_normalize(centroid);
}

/// In-place L2 normalization.
fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

// ─── Outcome Detection ──────────────────────────────────────────────────────

/// Detect routing outcome from user behavior signals.
///
/// This function analyzes the current and next turn to determine
/// whether the routing was successful.
///
/// # Arguments
///
/// * `current_domain` - Domain that was routed to
/// * `current_tool` - Tool that was executed (if any)
/// * `next_text` - Text of the next user turn (if available)
/// * `tool_success` - Whether the tool execution succeeded
/// * `tool_error` - Error message if tool failed
///
/// # Returns
///
/// Detected `RoutingOutcome`.
pub fn detect_outcome(
    _current_domain: Domain,
    current_tool: Option<&str>,
    next_text: Option<&str>,
    tool_success: bool,
    tool_error: Option<&str>,
) -> RoutingOutcome {
    // Check tool error first
    if !tool_success {
        if let Some(err) = tool_error {
            return RoutingOutcome::ToolError {
                error: err.to_string(),
            };
        }
        return RoutingOutcome::ToolError {
            error: "unknown error".into(),
        };
    }

    // Check for correction phrases in next turn
    if let Some(text) = next_text {
        if is_correction_phrase(text) {
            return RoutingOutcome::Corrected {
                correct_domain: Domain::Conversation, // Will be resolved by context
                correct_tool: None,
            };
        }

        // Check for rephrasing (similar meaning, different words)
        if is_rephrase(text, current_tool) {
            return RoutingOutcome::Rephrased;
        }
    }

    // Default: success
    RoutingOutcome::Success
}

/// Check if text contains a correction phrase.
fn is_correction_phrase(text: &str) -> bool {
    let lower = text.to_lowercase();
    let correction_patterns = [
        "no i meant",
        "no i mean",
        "actually",
        "not that",
        "not this",
        "the other one",
        "i meant",
        "i mean",
        "wrong",
        "nahi",
        "nahin",
        "galat",
        "nahi ye",
        "wo nahi",
    ];

    correction_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// Check if text is a rephrase of the current request.
fn is_rephrase(text: &str, _current_tool: Option<&str>) -> bool {
    // Simple heuristic: if text is very short and contains similar verbs
    // but different nouns, it's likely a rephrase
    let lower = text.to_lowercase();
    let rephrase_patterns = [
        "same thing",
        "again",
        "do it again",
        "try again",
        "once more",
        "wapas",
        "phir se",
    ];

    rephrase_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_collector_records_entries() {
        let mut collector = FeedbackCollector::default_config();
        let feedback = RoutingFeedback {
            input_text_hash: 12345,
            domain_selected: Domain::SystemInfo,
            tool_selected: Some("check_health".into()),
            intent_source: "FastEmbedSemanticRouter".into(),
            confidence: 0.85,
            outcome: RoutingOutcome::Success,
            timestamp: 1000000,
            session_id: "test".into(),
            embedding: vec![0.1; 10],
        };
        collector.record(feedback);
        assert_eq!(collector.buffer_len(), 1);
    }

    #[test]
    fn adjustment_report_default() {
        let report = CentroidAdjustmentReport::default();
        assert_eq!(report.success_nudges, 0);
        assert_eq!(report.total_adjusted, 0);
    }

    #[test]
    fn nudge_centroid_moves_toward_embedding() {
        let mut centroid = vec![1.0, 0.0, 0.0];
        let embedding = vec![0.0, 1.0, 0.0];
        nudge_centroid(&mut centroid, &embedding, 0.5);
        // Centroid should move toward embedding
        assert!(centroid[0] < 1.0);
        assert!(centroid[1] > 0.0);
    }

    #[test]
    fn l2_normalize_produces_unit_vector() {
        let mut v = vec![3.0, 4.0, 0.0];
        l2_normalize(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn detect_outcome_success() {
        let outcome = detect_outcome(
            Domain::SystemInfo,
            Some("check_health"),
            Some("what about memory"),
            true,
            None,
        );
        // Should be success (no correction phrase)
        assert!(matches!(outcome, RoutingOutcome::Success));
    }

    #[test]
    fn detect_outcome_tool_error() {
        let outcome = detect_outcome(
            Domain::SystemInfo,
            Some("check_health"),
            None,
            false,
            Some("permission denied".into()),
        );
        assert!(matches!(outcome, RoutingOutcome::ToolError { .. }));
    }

    #[test]
    fn detect_outcome_correction() {
        let outcome = detect_outcome(
            Domain::SystemInfo,
            Some("check_health"),
            Some("no i meant the network"),
            true,
            None,
        );
        assert!(matches!(outcome, RoutingOutcome::Corrected { .. }));
    }

    #[test]
    fn is_correction_phrase_works() {
        assert!(is_correction_phrase("no i meant the network"));
        assert!(is_correction_phrase("actually i want the other one"));
        assert!(is_correction_phrase("nahi ye wala"));
        assert!(!is_correction_phrase("check system health"));
    }

    #[test]
    fn is_rephrase_works() {
        assert!(is_rephrase("do it again", None));
        assert!(is_rephrase("try once more", None));
        assert!(is_rephrase("phir se karo", None));
        assert!(!is_rephrase("check system health", None));
    }

    #[test]
    fn adjust_centroids_success() {
        let mut centroids = HashMap::new();
        centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
        let feedback = vec![RoutingFeedback {
            input_text_hash: 1,
            domain_selected: Domain::SystemInfo,
            tool_selected: None,
            intent_source: "test".into(),
            confidence: 0.9,
            outcome: RoutingOutcome::Success,
            timestamp: 1000,
            session_id: "test".into(),
            embedding: vec![0.0, 1.0, 0.0],
        }];
        let report = adjust_centroids(&feedback, &mut centroids, 0.1);
        assert_eq!(report.success_nudges, 1);
        // Centroid should have moved toward embedding
        let c = &centroids[&Domain::SystemInfo];
        assert!(c[0] < 1.0);
        assert!(c[1] > 0.0);
    }

    #[test]
    fn adjust_centroids_correction() {
        let mut centroids = HashMap::new();
        centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
        centroids.insert(Domain::Knowledge, vec![0.0, 1.0, 0.0]);
        let feedback = vec![RoutingFeedback {
            input_text_hash: 1,
            domain_selected: Domain::SystemInfo,
            tool_selected: None,
            intent_source: "test".into(),
            confidence: 0.9,
            outcome: RoutingOutcome::Corrected {
                correct_domain: Domain::Knowledge,
                correct_tool: None,
            },
            timestamp: 1000,
            session_id: "test".into(),
            embedding: vec![0.5, 0.5, 0.0],
        }];
        let report = adjust_centroids(&feedback, &mut centroids, 0.1);
        assert_eq!(report.correction_pushes, 1);
        assert_eq!(report.correction_pulls, 1);
    }

    #[test]
    fn flush_to_disk_creates_file() {
        let dir = std::env::temp_dir().join("kria_feedback_test");
        let mut collector = FeedbackCollector::new(
            dir.to_str().unwrap(),
            2,
            0.01,
        );
        let feedback = RoutingFeedback {
            input_text_hash: 1,
            domain_selected: Domain::SystemInfo,
            tool_selected: None,
            intent_source: "test".into(),
            confidence: 0.9,
            outcome: RoutingOutcome::Success,
            timestamp: 1000,
            session_id: "test".into(),
            embedding: vec![],
        };
        collector.record(feedback);
        collector.flush_to_disk();

        // Verify file was created and can be read back
        let history = collector.load_history();
        assert_eq!(history.len(), 1);

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_flush_at_capacity() {
        let dir = std::env::temp_dir().join("kria_feedback_test_capacity");
        let mut collector = FeedbackCollector::new(
            dir.to_str().unwrap(),
            3,
            0.01,
        );
        for i in 0..5 {
            let feedback = RoutingFeedback {
                input_text_hash: i,
                domain_selected: Domain::SystemInfo,
                tool_selected: None,
                intent_source: "test".into(),
                confidence: 0.9,
                outcome: RoutingOutcome::Success,
                timestamp: 1000,
                session_id: "test".into(),
                embedding: vec![],
            };
            collector.record(feedback);
        }
        // Buffer should have been flushed at capacity 3
        // After 5 records, buffer should have 2 remaining (5 % 3 = 2)
        assert_eq!(collector.buffer_len(), 2);

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }
}
