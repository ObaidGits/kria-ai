//! Capability resolver — Hybrid BM25 + Dense retrieval for skill matching.
//!
//! Solves the "tool soup" problem: a 7B LLM cannot handle 5,400+ tool schemas.
//! The resolver semantically narrows which OpenClaw skills to expose per turn.
//!
//! # Pipeline
//!
//! 1. **IntentClassifier** — fast keyword pre-filter. If the prompt is clearly
//!    "native-only" (file ops, system info), skip OpenClaw entirely.
//! 2. **BM25 keyword retrieval** — fast exact-term matching over skill names
//!    and descriptions. Overfetches (e.g., top 20) for re-ranking.
//! 3. **Dense re-ranking** — cosine similarity over pre-computed embeddings
//!    of the BM25 candidates. Combined score: `0.4 * BM25 + 0.6 * Dense`.
//! 4. **Truncation** — return top `max_oc_tools` matches above threshold.
//!
//! # Thread Safety
//!
//! The `SkillIndex` uses `ArcSwap` for lock-free reads during hot path.
//! Rebuilds (when skills are installed/uninstalled) atomically swap the snapshot.

use super::types::{ResourceClass, SkillDescriptor, TrustTier};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;

// ─── Skill Snapshot (Immutable, Arc-swapped) ─────────────────────────────────

/// Immutable snapshot of the skill index.
/// Readers get an `Arc` clone — zero contention.
#[derive(Debug, Clone)]
pub struct SkillSnapshot {
    /// Pre-computed skill entries for semantic matching.
    pub entries: Vec<SkillEntry>,
    /// BM25 inverted index for keyword matching.
    pub bm25_index: Bm25Index,
    /// Build timestamp.
    pub built_at: std::time::Instant,
}

impl SkillSnapshot {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            bm25_index: Bm25Index::empty(),
            built_at: std::time::Instant::now(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A single skill's pre-computed entry.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub trust_tier: TrustTier,
    pub resource_class: ResourceClass,
    /// Pre-computed embedding vector (from skill name + description + category).
    pub embedding: Vec<f32>,
    /// The full skill descriptor.
    pub descriptor: SkillDescriptor,
}

// ─── Skill Index (ArcSwap-backed) ────────────────────────────────────────────

/// Lock-free skill index using ArcSwap.
pub struct SkillIndex {
    snapshot: ArcSwap<SkillSnapshot>,
}

impl SkillIndex {
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(SkillSnapshot::empty()),
        }
    }

    /// Read the current snapshot. Zero contention — just an Arc clone.
    pub fn load(&self) -> arc_swap::Guard<Arc<SkillSnapshot>> {
        self.snapshot.load()
    }

    /// Rebuild the index from installed skills.
    /// Atomically swaps the snapshot — readers see the old version
    /// until the swap completes, then seamlessly switch.
    pub async fn rebuild(
        &self,
        skills: &[SkillDescriptor],
        embed_fn: &dyn Fn(&str) -> Vec<f32>,
    ) -> Result<(), IndexError> {
        let entries: Vec<SkillEntry> = skills
            .iter()
            .map(|skill| {
                let text = format!(
                    "{} {} {}",
                    skill.name, skill.description, skill.category
                );
                let embedding = embed_fn(&text);
                SkillEntry {
                    skill_id: skill.skill_id.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    category: skill.category.clone(),
                    trust_tier: skill.trust_tier,
                    resource_class: skill.resource_profile.resource_class,
                    embedding,
                    descriptor: skill.clone(),
                }
            })
            .collect();

        let bm25_index = Bm25Index::build(&entries);

        let new_snapshot = Arc::new(SkillSnapshot {
            entries,
            bm25_index,
            built_at: std::time::Instant::now(),
        });

        self.snapshot.store(new_snapshot);
        Ok(())
    }
}

// ─── BM25 Index ──────────────────────────────────────────────────────────────

/// Lightweight BM25 inverted index for keyword matching.
#[derive(Debug, Clone)]
pub struct Bm25Index {
    /// term → [(entry_index, term_frequency), ...]
    inverted_index: HashMap<String, Vec<(usize, f32)>>,
    /// Document lengths (number of terms per entry).
    doc_lengths: Vec<usize>,
    /// Average document length.
    avg_doc_length: f32,
    /// Total number of documents.
    num_docs: usize,
    /// BM25 parameters.
    k1: f32,
    b: f32,
}

/// A BM25 search result.
#[derive(Debug, Clone)]
pub struct Bm25Result {
    pub entry_index: usize,
    pub bm25_score: f32,
}

impl Bm25Index {
    pub fn empty() -> Self {
        Self {
            inverted_index: HashMap::new(),
            doc_lengths: Vec::new(),
            avg_doc_length: 0.0,
            num_docs: 0,
            k1: 1.5,
            b: 0.75,
        }
    }

    /// Build a BM25 index from skill entries.
    pub fn build(entries: &[SkillEntry]) -> Self {
        let mut inverted_index: HashMap<String, Vec<(usize, f32)>> = HashMap::new();
        let mut doc_lengths = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            let tokens = tokenize(&format!(
                "{} {} {}",
                entry.name, entry.description, entry.category
            ));
            doc_lengths.push(tokens.len());

            // Count term frequencies
            let mut tf: HashMap<String, f32> = HashMap::new();
            for token in &tokens {
                *tf.entry(token.clone()).or_insert(0.0) += 1.0;
            }

            for (term, freq) in tf {
                inverted_index
                    .entry(term)
                    .or_default()
                    .push((i, freq));
            }
        }

        let avg_doc_length = if doc_lengths.is_empty() {
            0.0
        } else {
            doc_lengths.iter().sum::<usize>() as f32 / doc_lengths.len() as f32
        };

        Self {
            inverted_index,
            doc_lengths,
            avg_doc_length,
            num_docs: entries.len(),
            k1: 1.5,
            b: 0.75,
        }
    }

    /// Search the BM25 index for the top-K results.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<Bm25Result> {
        if self.num_docs == 0 {
            return Vec::new();
        }

        let query_tokens = tokenize(query);
        let mut scores: HashMap<usize, f32> = HashMap::new();

        for token in &query_tokens {
            if let Some(postings) = self.inverted_index.get(token) {
                // IDF: log((N - n + 0.5) / (n + 0.5) + 1)
                let n = postings.len() as f32;
                let idf = ((self.num_docs as f32 - n + 0.5) / (n + 0.5) + 1.0).ln();

                for &(doc_idx, tf) in postings {
                    let dl = self.doc_lengths[doc_idx] as f32;
                    let tf_norm = (tf * (self.k1 + 1.0))
                        / (tf + self.k1 * (1.0 - self.b + self.b * dl / self.avg_doc_length));
                    *scores.entry(doc_idx).or_insert(0.0) += idf * tf_norm;
                }
            }
        }

        let mut results: Vec<Bm25Result> = scores
            .into_iter()
            .map(|(idx, score)| Bm25Result {
                entry_index: idx,
                bm25_score: score,
            })
            .collect();

        results.sort_by(|a, b| b.bm25_score.partial_cmp(&a.bm25_score).unwrap());
        results.truncate(top_k);
        results
    }
}

/// Check if `text` contains `word` as a whole word (not as a substring).
fn contains_word(text: &str, word: &str) -> bool {
    let idx = match text.find(word) {
        Some(i) => i,
        None => return false,
    };
    let before = if idx == 0 { ' ' } else { text.as_bytes()[idx - 1] as char };
    let after_end = idx + word.len();
    let after = if after_end >= text.len() { ' ' } else { text.as_bytes()[after_end] as char };
    !before.is_alphanumeric() && !after.is_alphanumeric()
}

/// Simple tokenizer: lowercase, split on non-alphanumeric, filter short tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_string())
        .collect()
}

// ─── Intent Classifier ───────────────────────────────────────────────────────

/// Fast keyword pre-filter that determines if a prompt needs OpenClaw at all.
///
/// This is NOT the final routing decision — it's a fast pre-filter to avoid
/// running BM25+dense on prompts that clearly don't need OpenClaw skills.
pub struct IntentClassifier;

/// Classification result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentClass {
    /// KRIA has native tools for this. Skip OpenClaw entirely.
    NativeOnly,
    /// Might need OpenClaw skills. Run the full resolver.
    MayNeedOpenClaw,
}

impl IntentClassifier {
    pub fn new() -> Self {
        Self
    }

    /// Classify a user prompt.
    pub fn classify(&self, prompt: &str) -> IntentClass {
        let lower = prompt.to_lowercase();

        // Native-only patterns: KRIA already has these capabilities.
        // Each entry is (pattern, require_word_boundary).
        // Short patterns (< 6 chars) require word-boundary matching to avoid
        // false positives (e.g., "drive" matching "drivers").
        let native_patterns: &[(&str, bool)] = &[
            // File operations
            ("file", true), ("folder", false), ("directory", false),
            ("read file", false), ("write file", false),
            ("create directory", false), ("delete file", false),
            ("copy file", false), ("move file", false),
            // System info
            ("system", false), ("cpu", true), ("memory", false),
            ("ram", true), ("disk", true), ("battery", false),
            ("uptime", false), ("process", false), ("running app", false),
            // Package management
            ("install package", false), ("uninstall package", false),
            ("update package", false),
            ("apt", true), ("snap", true), ("flatpak", false),
            // System config
            ("brightness", false), ("volume", false), ("wifi", true),
            ("bluetooth", false),
            ("power plan", false), ("display", false), ("screen resolution", false),
            // Clipboard
            ("clipboard", false), ("copy", true), ("paste", false),
            // Window management
            ("window", false), ("minimize", false), ("maximize", false),
            ("focus", false),
            // Google Workspace (already have MCP tools)
            ("gmail", false), ("email", false), ("calendar", false),
            ("drive", true), ("google docs", false),
            // Knowledge/memory
            ("remember", false), ("recall", false), ("fact", true),
            ("snippet", false), ("knowledge", false),
            // Documents
            ("pdf", true), ("docx", false), ("xlsx", false),
            ("csv", true), ("parse document", false),
            // Git
            ("git status", false), ("git log", false),
            ("git diff", false), ("git commit", false),
            // Scheduling
            ("schedule", false), ("reminder", false), ("cron", true),
        ];

        for &(pattern, word_boundary) in native_patterns {
            if word_boundary {
                if contains_word(&lower, pattern) {
                    return IntentClass::NativeOnly;
                }
            } else if lower.contains(pattern) {
                return IntentClass::NativeOnly;
            }
        }

        IntentClass::MayNeedOpenClaw
    }
}

// ─── Capability Resolver ─────────────────────────────────────────────────────

/// Resolves which OpenClaw skills to expose for a given user prompt.
pub struct CapabilityResolver {
    skill_index: Arc<SkillIndex>,
    intent_classifier: IntentClassifier,
    max_oc_tools: usize,
    bm25_top_k: usize,
    dense_threshold: f32,
}

/// A resolved skill match.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    pub skill_id: String,
    pub name: String,
    pub confidence: f32,
    pub descriptor: SkillDescriptor,
}

impl CapabilityResolver {
    pub fn new(
        skill_index: Arc<SkillIndex>,
        max_oc_tools: usize,
        dense_threshold: f32,
    ) -> Self {
        Self {
            skill_index,
            intent_classifier: IntentClassifier::new(),
            max_oc_tools,
            bm25_top_k: max_oc_tools * 3, // Overfetch for re-ranking
            dense_threshold,
        }
    }

    /// Resolve OpenClaw skills for a user prompt.
    /// Returns an empty vec if the prompt is clearly native-only.
    pub async fn resolve(
        &self,
        user_prompt: &str,
        query_embedding: &[f32],
    ) -> Vec<SkillMatch> {
        // Stage 0: Intent pre-classification
        if self.intent_classifier.classify(user_prompt) == IntentClass::NativeOnly {
            return Vec::new();
        }

        let snapshot = self.skill_index.load();

        if snapshot.is_empty() {
            return Vec::new();
        }

        // Stage 1: BM25 keyword retrieval
        let bm25_candidates = snapshot.bm25_index.search(user_prompt, self.bm25_top_k);

        if bm25_candidates.is_empty() {
            return Vec::new();
        }

        // Stage 2: Dense re-ranking of BM25 candidates
        let mut reranked: Vec<SkillMatch> = bm25_candidates
            .iter()
            .filter_map(|candidate| {
                let entry = snapshot.entries.get(candidate.entry_index)?;
                let dense_score = cosine_similarity(query_embedding, &entry.embedding);
                // Combined score: 0.4 * BM25 + 0.6 * Dense
                let combined = 0.4 * normalize_bm25(candidate.bm25_score, &bm25_candidates)
                    + 0.6 * dense_score;

                if combined >= self.dense_threshold {
                    Some(SkillMatch {
                        skill_id: entry.skill_id.clone(),
                        name: entry.name.clone(),
                        confidence: combined,
                        descriptor: entry.descriptor.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        reranked.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        reranked.truncate(self.max_oc_tools);
        reranked
    }
}

/// Normalize a BM25 score to 0..1 range using min-max from the result set.
fn normalize_bm25(score: f32, results: &[Bm25Result]) -> f32 {
    if results.is_empty() {
        return 0.0;
    }
    let max = results
        .iter()
        .map(|r| r.bm25_score)
        .fold(0.0f32, f32::max);
    if max > 0.0 {
        (score / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }
}

/// Errors from the resolver.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("embedding generation failed: {0}")]
    EmbeddingFailed(String),
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::types::*;

    fn make_skill(id: &str, name: &str, desc: &str, cat: &str) -> SkillDescriptor {
        SkillDescriptor {
            skill_id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            category: cat.to_string(),
            parameters: serde_json::json!({}),
            risk_level: crate::safety::RiskLevel::Green,
            network_policy: OpenClawNetworkPolicy::None,
            resource_profile: ResourceProfile::for_category(cat),
            capabilities: SkillCapabilities::default(),
            trust_tier: TrustTier::Community,
            source: SkillSource::Bundled,
            installed_at: chrono::Utc::now(),
            last_used_at: None,
            use_count: 0,
            status: SkillStatus::Active,
        }
    }

    #[test]
    fn bm25_finds_keyword_match() {
        let skills = vec![
            make_skill("oc_web_search", "web_search", "Searches the web for information", "web"),
            make_skill("oc_music", "music_generate", "Generates music tracks from text", "media"),
            make_skill("oc_calculator", "calculator", "Calculates mathematical expressions", "productivity"),
        ];

        let entries: Vec<SkillEntry> = skills
            .iter()
            .map(|s| SkillEntry {
                skill_id: s.skill_id.clone(),
                name: s.name.clone(),
                description: s.description.clone(),
                category: s.category.clone(),
                trust_tier: s.trust_tier,
                resource_class: s.resource_profile.resource_class,
                embedding: vec![0.0; 384],
                descriptor: s.clone(),
            })
            .collect();

        let bm25 = Bm25Index::build(&entries);
        let results = bm25.search("search the web", 10);

        assert!(!results.is_empty());
        assert_eq!(results[0].entry_index, 0); // web_search should be top
    }

    #[test]
    fn intent_classifier_skips_native_only() {
        let classifier = IntentClassifier::new();

        assert_eq!(classifier.classify("read the file at /tmp/test.txt"), IntentClass::NativeOnly);
        assert_eq!(classifier.classify("what's my cpu usage"), IntentClass::NativeOnly);
        assert_eq!(classifier.classify("install package neovim"), IntentClass::NativeOnly);
        assert_eq!(classifier.classify("check my gmail inbox"), IntentClass::NativeOnly);

        assert_eq!(classifier.classify("search the web for CUDA drivers"), IntentClass::MayNeedOpenClaw);
        assert_eq!(classifier.classify("generate some lo-fi music"), IntentClass::MayNeedOpenClaw);
        assert_eq!(classifier.classify("take a screenshot of my screen"), IntentClass::MayNeedOpenClaw);
    }

    #[test]
    fn bm25_empty_index_returns_empty() {
        let bm25 = Bm25Index::empty();
        let results = bm25.search("anything", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b)).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_empty_vectors() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn bm25_multiple_keyword_matches_ranked_correctly() {
        let skills = vec![
            make_skill("oc_search", "web_search", "Search the web for any topic", "web"),
            make_skill("oc_fetch", "web_fetch", "Fetch a webpage and extract content", "web"),
            make_skill("oc_music", "music_gen", "Generate music from description", "media"),
        ];

        let entries: Vec<SkillEntry> = skills
            .iter()
            .map(|s| SkillEntry {
                skill_id: s.skill_id.clone(),
                name: s.name.clone(),
                description: s.description.clone(),
                category: s.category.clone(),
                trust_tier: s.trust_tier,
                resource_class: s.resource_profile.resource_class,
                embedding: vec![0.0; 384],
                descriptor: s.clone(),
            })
            .collect();

        let bm25 = Bm25Index::build(&entries);
        let results = bm25.search("web search information", 10);

        // Both web_search and web_fetch should rank above music_gen
        assert!(results.len() >= 2);
        assert!(results[0].bm25_score > 0.0);
        // music_gen should not be in top results for "web search"
        let music_in_top = results.iter().any(|r| r.entry_index == 2);
        assert!(!music_in_top || results.last().map(|r| r.entry_index) == Some(2));
    }
}
