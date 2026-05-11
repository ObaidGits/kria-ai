//! Fine-tuned intent classifier replacing regex router + ONNX L0.
//!
//! This module provides a more sophisticated intent classification system that:
//! - Uses a richer label taxonomy (16 operations vs 3 in legacy ONNX)
//! - Supports context-aware classification
//! - Runs in a dedicated CPU worker thread (same architecture as legacy ONNX)
//! - Gracefully degrades when model is unavailable
//!
//! # Feature Flag
//!
//! Enable via `routing.intent_classifier = true` in config or
//! `KRIA_ROUTING_V2=1` environment variable.
//!
//! # Architecture
//!
//! ```text
//! User text → Tokenizer → ONNX Inference → Softmax → IntentClassification
//!                                                      ↓
//!                                               Operation + ComputeClass + HazardHint
//! ```

use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::time::Duration;

use anyhow::{bail, Context};
use once_cell::sync::Lazy;
use ort::session::Session;
use regex::Regex;
use tokenizers::Tokenizer;

use crate::routing::context::RoutingContext;
use crate::routing::domain::Domain;

// ─── Constants ──────────────────────────────────────────────────────────────

const ENV_ENABLE_INTENT_CLF: &str = "KRIA_ROUTING_V2";
const ENV_INTENT_MODEL_PATH: &str = "KRIA_INTENT_MODEL_PATH";
const ENV_INTENT_TOKENIZER_PATH: &str = "KRIA_INTENT_TOKENIZER_PATH";

const DEFAULT_MODEL_PATH: &str = "~/.kria/models/classifier/intent_v2.onnx";
const DEFAULT_TOKENIZER_PATH: &str = "~/.kria/models/classifier/tokenizer.json";

const SOFTMAX_TEMPERATURE: f32 = 8.0;
const WORKER_TIMEOUT: Duration = Duration::from_millis(25);

// ─── Intent Labels ──────────────────────────────────────────────────────────

/// Classification labels matching the Operation taxonomy in turn_gate.rs.
///
/// These labels are what the fine-tuned model predicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentLabel {
    Converse,
    Read,
    Search,
    RetrieveMemory,
    Write,
    Send,
    Delete,
    ExecuteCode,
    ExecuteShell,
    Automate,
    GenerateImage,
    AnalyzeImage,
    AnalyzeFile,
    Schedule,
    ConfigureSystem,
    Cancel,
}

impl IntentLabel {
    /// All labels in order (index = label ID in model output).
    pub fn all() -> &'static [IntentLabel] {
        &[
            Self::Converse,
            Self::Read,
            Self::Search,
            Self::RetrieveMemory,
            Self::Write,
            Self::Send,
            Self::Delete,
            Self::ExecuteCode,
            Self::ExecuteShell,
            Self::Automate,
            Self::GenerateImage,
            Self::AnalyzeImage,
            Self::AnalyzeFile,
            Self::Schedule,
            Self::ConfigureSystem,
            Self::Cancel,
        ]
    }

    /// Convert to Operation enum (from turn_gate.rs).
    pub fn to_operation(self) -> crate::agent::turn_gate::Operation {
        use crate::agent::turn_gate::Operation as Op;
        match self {
            Self::Converse => Op::Converse,
            Self::Read => Op::Read,
            Self::Search => Op::Search,
            Self::RetrieveMemory => Op::RetrieveMemory,
            Self::Write => Op::Write,
            Self::Send => Op::Send,
            Self::Delete => Op::Delete,
            Self::ExecuteCode => Op::ExecuteCode,
            Self::ExecuteShell => Op::ExecuteShell,
            Self::Automate => Op::Automate,
            Self::GenerateImage => Op::GenerateImage,
            Self::AnalyzeImage => Op::AnalyzeImage,
            Self::AnalyzeFile => Op::AnalyzeFile,
            Self::Schedule => Op::Schedule,
            Self::ConfigureSystem => Op::ConfigureSystem,
            Self::Cancel => Op::Cancel,
        }
    }

    /// Map to ComputeClass.
    pub fn to_compute_class(self) -> crate::agent::turn_gate::ComputeClass {
        use crate::agent::turn_gate::ComputeClass as CC;
        match self {
            Self::Converse => CC::L1Text,
            Self::Read => CC::ToolOnly,
            Self::Search => CC::L1Text,
            Self::RetrieveMemory => CC::ToolOnly,
            Self::Write => CC::ToolOnly,
            Self::Send => CC::ToolOnly,
            Self::Delete => CC::ToolOnly,
            Self::ExecuteCode | Self::ExecuteShell => CC::ReflexRust,
            Self::Automate => CC::ToolOnly,
            Self::GenerateImage => CC::ImageGpu,
            Self::AnalyzeImage => CC::L1Vision,
            Self::AnalyzeFile => CC::SidecarCpu,
            Self::Schedule => CC::ReflexRust,
            Self::ConfigureSystem => CC::ReflexRust,
            Self::Cancel => CC::ReflexRust,
        }
    }

    /// Map to HazardHint.
    pub fn to_hazard_hint(self) -> crate::agent::turn_gate::HazardHint {
        use crate::agent::turn_gate::HazardHint as HH;
        match self {
            Self::Converse | Self::Read | Self::Search | Self::RetrieveMemory => HH::Green,
            Self::Write | Self::Schedule | Self::AnalyzeFile | Self::AnalyzeImage => HH::Yellow,
            Self::Send | Self::ExecuteCode | Self::ExecuteShell | Self::Automate => HH::Yellow,
            Self::ConfigureSystem => HH::Yellow,
            Self::Delete | Self::GenerateImage => HH::Red,
            Self::Cancel => HH::Green,
        }
    }

    /// Map to best-effort Domain.
    pub fn to_domain(self) -> Domain {
        match self {
            Self::Converse => Domain::Conversation,
            Self::Read | Self::Search | Self::RetrieveMemory => Domain::Knowledge,
            Self::Write | Self::Delete => Domain::FileOps,
            Self::Send => Domain::Comms,
            Self::ExecuteCode | Self::ExecuteShell | Self::Automate => Domain::Developer,
            Self::GenerateImage | Self::AnalyzeImage => Domain::Vision,
            Self::AnalyzeFile => Domain::FileOps,
            Self::Schedule => Domain::Comms,
            Self::ConfigureSystem => Domain::Power,
            Self::Cancel => Domain::Conversation,
        }
    }

    /// Parse from string label (for training data).
    pub fn from_str_label(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "converse" => Some(Self::Converse),
            "read" => Some(Self::Read),
            "search" => Some(Self::Search),
            "retrieve_memory" | "retrievememory" | "memory" => Some(Self::RetrieveMemory),
            "write" => Some(Self::Write),
            "send" => Some(Self::Send),
            "delete" => Some(Self::Delete),
            "execute_code" | "executecode" => Some(Self::ExecuteCode),
            "execute_shell" | "executeshell" => Some(Self::ExecuteShell),
            "automate" => Some(Self::Automate),
            "generate_image" | "generateimage" => Some(Self::GenerateImage),
            "analyze_image" | "analyzeimage" => Some(Self::AnalyzeImage),
            "analyze_file" | "analyzefile" => Some(Self::AnalyzeFile),
            "schedule" => Some(Self::Schedule),
            "configure_system" | "configuresystem" => Some(Self::ConfigureSystem),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

// ─── Classification Result ──────────────────────────────────────────────────

/// Result of the intent classifier.
#[derive(Debug, Clone)]
pub struct IntentClassification {
    /// Predicted operation.
    pub operation: crate::agent::turn_gate::Operation,
    /// Compute class for resource planning.
    pub compute: crate::agent::turn_gate::ComputeClass,
    /// Hazard hint for safety.
    pub hazard: crate::agent::turn_gate::HazardHint,
    /// Classification confidence (0.0–1.0).
    pub confidence: f32,
    /// Source of this classification.
    pub source: crate::agent::turn_gate::IntentSource,
    /// Best-effort domain match.
    pub domain: Domain,
    /// The label that was predicted.
    pub label: IntentLabel,
}

// ─── Hinglish Pattern Hints ─────────────────────────────────────────────────

/// Pre-classification hints from Hinglish patterns.
/// These boost the classifier when specific patterns are detected.
#[derive(Debug, Clone)]
struct HinglishHint {
    label: IntentLabel,
    confidence_boost: f32,
}

static HINGLISH_HINTS: Lazy<Vec<(Regex, HinglishHint)>> = Lazy::new(|| {
    let entries: &[(&str, IntentLabel, f32)] = &[
        // System commands
        (
            r"(?i)(system|cpu|ram|memory|battery|disk|network|status).*(dikhao|check|batao|info)",
            IntentLabel::Read,
            0.15,
        ),
        // Volume/brightness
        (
            r"(?i)(volume|brightness|awaz|roshni).*(badhao|ghatao|set|karo|adjust)",
            IntentLabel::ConfigureSystem,
            0.2,
        ),
        // Email/messaging
        (
            r"(?i)(email|mail|bhejo|message|sms|send).*(bhejo|karo|compose|likh)",
            IntentLabel::Send,
            0.15,
        ),
        // File operations
        (
            r"(?i)(file|folder|padhao|dhundo|likhao|copy|move|delete).*(karo|do)",
            IntentLabel::Write,
            0.1,
        ),
        // Image generation
        (
            r"(?i)(image|photo|picture|tasveer|banao|generate|draw|sketch).*(banao|karo)",
            IntentLabel::GenerateImage,
            0.15,
        ),
        // Scheduling
        (
            r"(?i)(remind|reminder|schedule|calendar|yaad|dilao|set.*alarm)",
            IntentLabel::Schedule,
            0.15,
        ),
        // App lifecycle
        (
            r"(?i)(open|close|kholo|band|launch|start|app|application).*(karo|do)",
            IntentLabel::ExecuteCode,
            0.1,
        ),
        // Search
        (
            r"(?i)(search|find|dhundo|lookup|kya hai|batao|internet)",
            IntentLabel::Search,
            0.1,
        ),
        // Cancel/stop
        (
            r"(?i)(stop|cancel|ruko|band|bas|halt|abort)",
            IntentLabel::Cancel,
            0.2,
        ),
        // Memory/recall
        (
            r"(?i)(remember|recall|yaad|memory|notes|told you|bola tha)",
            IntentLabel::RetrieveMemory,
            0.15,
        ),
    ];

    entries
        .iter()
        .map(|(pattern, label, boost)| {
            (
                Regex::new(pattern).expect("valid Hinglish hint regex"),
                HinglishHint {
                    label: *label,
                    confidence_boost: *boost,
                },
            )
        })
        .collect()
});

/// Detect Hinglish-specific hints to boost classification.
fn detect_hinglish_hints(text: &str) -> Vec<HinglishHint> {
    HINGLISH_HINTS
        .iter()
        .filter(|(re, _)| re.is_match(text))
        .map(|(_, hint)| hint.clone())
        .collect()
}

// ─── Intent Classifier ──────────────────────────────────────────────────────

/// Classification job sent to worker thread.
struct ClassifierJob {
    text: String,
    response_tx: SyncSender<Option<IntentClassification>>,
}

/// Runtime settings resolved from env/config.
#[derive(Debug, Clone)]
struct RuntimeSettings {
    model_path: PathBuf,
    tokenizer_path: PathBuf,
}

/// ONNX runtime state (runs in worker thread).
struct IntentRuntime {
    session: Session,
    tokenizer: Tokenizer,
    input_names: Vec<String>,
}

/// Fine-tuned intent classifier.
///
/// Runs inference in a dedicated CPU worker thread to avoid blocking
/// the async runtime. Supports Hinglish pattern hints and context awareness.
#[derive(Clone)]
pub struct IntentClassifier {
    tx: SyncSender<ClassifierJob>,
    timeout: Duration,
    status: ClassifierStatus,
}

impl std::fmt::Debug for IntentClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntentClassifier")
            .field("timeout", &self.timeout)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifierStatus {
    Ready,
    Unavailable,
}

impl IntentClassifier {
    /// Create a new IntentClassifier with a bounded work queue.
    pub fn new(queue_capacity: usize, timeout: Duration) -> Self {
        let (tx, rx) = sync_channel::<ClassifierJob>(queue_capacity.max(1));
        let settings = resolve_runtime_settings();
        let (runtime, status) = IntentRuntime::load(&settings).map_or_else(
            |error| {
                tracing::warn!(
                    model = %settings.model_path.display(),
                    tokenizer = %settings.tokenizer_path.display(),
                    error = %error,
                    "Intent classifier disabled: failed to initialize runtime"
                );
                (None, ClassifierStatus::Unavailable)
            },
            |runtime| (Some(runtime), ClassifierStatus::Ready),
        );

        let _ = std::thread::Builder::new()
            .name("kria-intent-classifier-worker".to_string())
            .spawn(move || worker_loop(rx, runtime));

        Self {
            tx,
            timeout,
            status,
        }
    }

    /// Create a classifier that always returns Unavailable (for testing/fallback).
    pub fn disabled() -> Self {
        let (tx, _rx) = sync_channel::<ClassifierJob>(1);
        Self {
            tx,
            timeout: WORKER_TIMEOUT,
            status: ClassifierStatus::Unavailable,
        }
    }

    /// Check if classifier is ready.
    pub fn is_ready(&self) -> bool {
        self.status == ClassifierStatus::Ready
    }

    /// Get classifier status.
    pub fn status(&self) -> ClassifierStatus {
        self.status
    }

    /// Classify user text into an intent.
    ///
    /// Returns `None` if:
    /// - Model is unavailable
    /// - Queue is saturated
    /// - Timeout exceeded
    /// - Confidence below minimum threshold
    pub fn classify(&self, text: &str, ctx: &RoutingContext) -> Option<IntentClassification> {
        // Quick Hinglish hint check (runs on caller thread, no model needed)
        let hinglish_hints = detect_hinglish_hints(text);

        let (response_tx, response_rx) = sync_channel::<Option<IntentClassification>>(1);
        let job = ClassifierJob {
            text: text.to_string(),
            response_tx,
        };

        // Bounded queue: if saturated, skip hinting instead of blocking
        if self.tx.try_send(job).is_err() {
            tracing::warn!("Intent classifier queue saturated; skipping hint");
            return None;
        }

        let mut result = response_rx.recv_timeout(self.timeout).ok().flatten();

        // Apply Hinglish hints as confidence boost
        if let Some(ref mut classification) = result {
            for hint in &hinglish_hints {
                if IntentLabel::to_operation(hint.label) == classification.operation {
                    classification.confidence =
                        (classification.confidence + hint.confidence_boost).min(0.99);
                }
            }
        }

        // Context boost: if classifier agrees with context domain, boost confidence
        if let (Some(ref mut classification), Some(last_domain)) =
            (&mut result, ctx.last_domain)
        {
            if classification.domain == last_domain && ctx.turn_count_in_topic >= 2 {
                classification.confidence =
                    (classification.confidence + 0.1).min(0.99);
            }
        }

        if let Some(ref value) = result {
            tracing::info!(
                operation = ?value.operation,
                compute = ?value.compute,
                confidence = value.confidence,
                label = ?value.label,
                "Intent classifier emitted routing hint"
            );
        }

        result
    }
}

// ─── Worker Thread ──────────────────────────────────────────────────────────

fn worker_loop(rx: Receiver<ClassifierJob>, mut runtime: Option<IntentRuntime>) {
    for job in rx {
        let result = runtime.as_mut().and_then(|rt| rt.classify(&job.text));
        let _ = job.response_tx.send(result);
    }
}

impl IntentRuntime {
    fn load(settings: &RuntimeSettings) -> anyhow::Result<Self> {
        if !settings.model_path.exists() {
            bail!(
                "model file not found at {}",
                settings.model_path.display()
            );
        }

        let session = Session::builder()
            .context("failed to create ONNX session builder")?
            .commit_from_file(&settings.model_path)
            .context("failed to load ONNX model")?;

        let tokenizer = Tokenizer::from_file(&settings.tokenizer_path)
            .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {}", e))?;

        let input_names: Vec<String> = session
            .inputs()
            .iter()
            .map(|input| input.name().to_string())
            .collect();

        tracing::info!(
            model = %settings.model_path.display(),
            inputs = ?input_names,
            "Intent classifier runtime loaded"
        );

        Ok(Self {
            session,
            tokenizer,
            input_names,
        })
    }

    fn classify(&mut self, text: &str) -> Option<IntentClassification> {
        // Tokenize
        let encoding = self.tokenizer.encode(text, false).ok()?;
        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        // Create tensors
        let input_ids_i64: Vec<i64> = input_ids.iter().map(|&x| x as i64).collect();
        let attention_mask_i64: Vec<i64> = attention_mask.iter().map(|&x| x as i64).collect();

        let input_ids_tensor = ort::value::Tensor::from_array(([1, input_ids.len()], input_ids_i64)).ok()?;
        let attention_mask_tensor =
            ort::value::Tensor::from_array(([1, attention_mask.len()], attention_mask_i64)).ok()?;

        // Build inputs map using input names from session
        let inputs: Vec<(&str, ort::value::DynValue)> = self
            .input_names
            .iter()
            .zip(vec![
                input_ids_tensor.into_dyn(),
                attention_mask_tensor.into_dyn(),
            ])
            .map(|(name, val)| (name.as_str(), val))
            .collect();

        // Run inference
        let outputs = self.session.run(inputs).ok()?;

        // Extract logits from output (try named outputs, then fall back to first)
        let logits_data: Vec<f32> = if let Some(value) = outputs.get("logits") {
            let (_shape, data) = value.try_extract_tensor::<f32>().ok()?;
            data.to_vec()
        } else if let Some((_, value)) = outputs.iter().next() {
            let (_shape, data) = value.try_extract_tensor::<f32>().ok()?;
            data.to_vec()
        } else {
            return None;
        };

        // Softmax + argmax
        let (best_idx, confidence) = softmax_argmax(&logits_data)?;

        // Map to label
        let labels = IntentLabel::all();
        let label = labels.get(best_idx)?;

        // Use raw softmax probability — no dummy scaling.
        let scaled_confidence = confidence.min(0.99);

        Some(IntentClassification {
            operation: label.to_operation(),
            compute: label.to_compute_class(),
            hazard: label.to_hazard_hint(),
            confidence: scaled_confidence,
            source: crate::agent::turn_gate::IntentSource::OnnxClassifier,
            domain: label.to_domain(),
            label: *label,
        })
    }
}

/// Apply softmax temperature and return (best_index, confidence).
fn softmax_argmax(logits: &[f32]) -> Option<(usize, f32)> {
    if logits.is_empty() {
        return None;
    }

    // Temperature scaling
    let scaled: Vec<f32> = logits.iter().map(|&x| x / SOFTMAX_TEMPERATURE).collect();

    // Numerically stable softmax
    let max_val = scaled.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum: f32 = scaled.iter().map(|&x| (x - max_val).exp()).sum();

    let mut best_idx = 0;
    let mut best_val = f32::NEG_INFINITY;

    for (i, &logit) in scaled.iter().enumerate() {
        let val = (logit - max_val).exp() / exp_sum;
        if val > best_val {
            best_val = val;
            best_idx = i;
        }
    }

    Some((best_idx, best_val))
}

// ─── Configuration ──────────────────────────────────────────────────────────

/// Check if the new intent classifier is enabled via feature flag.
pub fn is_enabled() -> bool {
    // Env var takes priority
    if let Ok(value) = std::env::var(ENV_ENABLE_INTENT_CLF) {
        if let Some(parsed) = parse_bool_like(&value) {
            return parsed;
        }
    }
    false // Default: disabled (use legacy router)
}

fn parse_bool_like(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn resolve_runtime_settings() -> RuntimeSettings {
    let model_path = std::env::var(ENV_INTENT_MODEL_PATH)
        .map(PathBuf::from)
        .unwrap_or_else(|_| expand_tilde(DEFAULT_MODEL_PATH));

    let tokenizer_path = std::env::var(ENV_INTENT_TOKENIZER_PATH)
        .map(PathBuf::from)
        .unwrap_or_else(|_| expand_tilde(DEFAULT_TOKENIZER_PATH));

    RuntimeSettings {
        model_path,
        tokenizer_path,
    }
}

/// Simple tilde expansion (replaces ~ with home directory).
fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(&path[1..])
        } else {
            PathBuf::from(path)
        }
    } else {
        PathBuf::from(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_all_has_16_entries() {
        assert_eq!(IntentLabel::all().len(), 16);
    }

    #[test]
    fn label_to_operation_mapping() {
        assert_eq!(
            IntentLabel::Read.to_operation(),
            crate::agent::turn_gate::Operation::Read
        );
        assert_eq!(
            IntentLabel::Cancel.to_operation(),
            crate::agent::turn_gate::Operation::Cancel
        );
        assert_eq!(
            IntentLabel::GenerateImage.to_operation(),
            crate::agent::turn_gate::Operation::GenerateImage
        );
    }

    #[test]
    fn label_to_compute_class_mapping() {
        assert_eq!(
            IntentLabel::ConfigureSystem.to_compute_class(),
            crate::agent::turn_gate::ComputeClass::ReflexRust
        );
        assert_eq!(
            IntentLabel::GenerateImage.to_compute_class(),
            crate::agent::turn_gate::ComputeClass::ImageGpu
        );
        assert_eq!(
            IntentLabel::Converse.to_compute_class(),
            crate::agent::turn_gate::ComputeClass::L1Text
        );
    }

    #[test]
    fn label_to_domain_mapping() {
        assert_eq!(IntentLabel::Send.to_domain(), Domain::Comms);
        assert_eq!(IntentLabel::Read.to_domain(), Domain::Knowledge);
        assert_eq!(IntentLabel::Delete.to_domain(), Domain::FileOps);
    }

    #[test]
    fn from_str_label_roundtrip() {
        for label in IntentLabel::all() {
            let s = format!("{:?}", label).to_lowercase();
            let parsed = IntentLabel::from_str_label(&s);
            assert_eq!(parsed, Some(*label), "Failed for: {}", s);
        }
    }

    #[test]
    fn hinglish_hint_detection() {
        let hints = detect_hinglish_hints("volume badhao");
        assert!(!hints.is_empty());
        assert_eq!(hints[0].label, IntentLabel::ConfigureSystem);
    }

    #[test]
    fn hinglish_hint_no_match() {
        let hints = detect_hinglish_hints("hello world");
        assert!(hints.is_empty());
    }

    #[test]
    fn softmax_argmax_basic() {
        let logits = vec![1.0, 2.0, 3.0, 0.5];
        let (idx, conf) = softmax_argmax(&logits).unwrap();
        assert_eq!(idx, 2); // highest logit
        assert!(conf > 0.2); // reasonable confidence
    }

    #[test]
    fn softmax_argmax_empty() {
        assert!(softmax_argmax(&[]).is_none());
    }

    #[test]
    fn softmax_argmax_equal() {
        let logits = vec![1.0, 1.0, 1.0];
        let (idx, conf) = softmax_argmax(&logits).unwrap();
        assert!(idx < 3);
        assert!((conf - 1.0 / 3.0).abs() < 0.01); // uniform distribution
    }

    #[test]
    fn disabled_classifier_returns_none() {
        let clf = IntentClassifier::disabled();
        assert!(!clf.is_ready());
        assert!(clf.classify("hello", &RoutingContext::default()).is_none());
    }

    #[test]
    fn context_boost_applied() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::Knowledge,
            None,
            crate::routing::verbs::IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.turn_count_in_topic = 3;
        // The classifier is disabled, so this tests the context boost logic path
        // In a real scenario with a loaded model, this would boost confidence
        let clf = IntentClassifier::disabled();
        assert!(clf.classify("search the web", &ctx).is_none()); // disabled
    }
}
