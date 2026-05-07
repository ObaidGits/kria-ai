//! Tool-level semantic matching index.
//!
//! Pre-computes embeddings for all registered tool descriptions.
//! On query, finds the best matching tool via cosine similarity.
//! If confidence ≥ threshold → direct execution (skip LLM).
//!
//! # Architecture
//!
//! ```text
//! ToolRegistry → ToolDef[] → embed_batch() → ToolEmbeddingIndex
//!                                                    ↓
//! User query → embed_one() → match_tool() → Option<ToolMatch>
//!                                                    ↓
//!                                            confidence ≥ 0.85 → DIRECT EXECUTION
//!                                            confidence < 0.85 → LLM with narrowed tools
//! ```
//!
//! # Latency Budget
//!
//! - Index build: <500ms for 100 tools (one-time at startup)
//! - Tool match: <1ms (cosine similarity scan)
//! - Embedding query: ~10ms (fastembed single text)
//!
//! # MCP Integration
//!
//! When MCP servers are reconciled, `rebuild()` is called to update the index
//! with newly registered tools.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::embed;
use crate::config::RoutingConfig;
use crate::tools::registry::ToolDef;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Default confidence threshold for direct tool execution.
const DEFAULT_THRESHOLD: f32 = 0.85;

/// Minimum hardware tier ordering (lower index = less capable).
const TIER_ORDER: &[&str] = &["lite", "standard", "performance", "high"];

// ─── Tool Match Result ──────────────────────────────────────────────────────

/// Result of tool-level semantic matching.
#[derive(Debug, Clone)]
pub struct ToolMatch {
    /// Name of the matched tool.
    pub name: String,
    /// Description of the matched tool.
    pub description: String,
    /// Category of the matched tool.
    pub category: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
    /// Whether this match should trigger direct execution (skip LLM).
    pub direct_execution: bool,
}

// ─── Tool Embedding Entry ───────────────────────────────────────────────────

/// A single tool's pre-computed embedding.
#[derive(Debug, Clone)]
struct ToolEntry {
    /// Tool name.
    name: String,
    /// Tool description (for rich embedding).
    description: String,
    /// Tool category.
    category: String,
    /// Pre-computed L2-normalized embedding vector.
    embedding: Vec<f32>,
    /// Per-tool confidence threshold (overrides global).
    threshold: f32,
    /// Minimum hardware tier required.
    min_tier: String,
}

// ─── Tool Embedding Index ───────────────────────────────────────────────────

/// Tool-level semantic matching index.
///
/// Pre-computes embeddings for all registered tool descriptions.
/// On query, finds the best matching tool via cosine similarity.
#[derive(Debug)]
pub struct ToolEmbeddingIndex {
    /// Pre-computed tool entries.
    entries: Vec<ToolEntry>,
    /// Global fallback threshold.
    global_threshold: f32,
    /// Number of tools indexed.
    tool_count: usize,
}

impl ToolEmbeddingIndex {
    /// Build index from tool definitions.
    ///
    /// This is the primary constructor. It embeds all tool descriptions
    /// using the same multilingual-e5-small model as the domain router.
    ///
    /// If the embedding model is not initialized, returns an empty index
    /// (graceful degradation).
    ///
    /// # Arguments
    ///
    /// * `tools` - Slice of `ToolDef` from the `ToolRegistry`
    /// * `config` - Routing config (for threshold settings)
    ///
    /// # Returns
    ///
    /// `Ok(ToolEmbeddingIndex)` with pre-computed embeddings, or empty index
    /// if embedding model is unavailable.
    pub fn from_tool_defs(tools: &[ToolDef], config: &RoutingConfig) -> Result<Self> {
        let threshold = config.tool_index_threshold;

        if tools.is_empty() {
            return Ok(Self {
                entries: Vec::new(),
                global_threshold: threshold,
                tool_count: 0,
            });
        }

        // Check if embedding model is ready
        if !embed::is_ready() {
            warn!("Embedding model not initialized — tool index will be empty");
            return Ok(Self::empty());
        }

        // Build rich description strings for embedding
        let descriptions: Vec<String> = tools
            .iter()
            .map(|tool| Self::build_rich_description(tool))
            .collect();

        // Embed all tool descriptions in one batch
        let text_refs: Vec<&str> = descriptions.iter().map(|s| s.as_str()).collect();
        let embeddings = match embed::embed_batch(&text_refs) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to embed tool descriptions: {} — starting empty", e);
                return Ok(Self::empty());
            }
        };

        let entries: Vec<ToolEntry> = tools
            .iter()
            .zip(descriptions)
            .zip(embeddings)
            .map(|((tool, desc), embedding)| ToolEntry {
                name: tool.name.clone(),
                description: desc,
                category: tool.category.clone(),
                embedding,
                threshold,
                min_tier: tool.min_tier.to_string(),
            })
            .collect();

        let tool_count = entries.len();
        info!(
            tool_count,
            threshold, "Tool embedding index built successfully"
        );

        Ok(Self {
            entries,
            global_threshold: threshold,
            tool_count,
        })
    }

    /// Build a rich description string for embedding.
    ///
    /// Includes tool name, description, category, and parameter hints
    /// for better semantic matching.
    pub fn build_rich_description(tool: &ToolDef) -> String {
        let param_summary: Vec<String> = tool
            .parameters
            .iter()
            .map(|p| {
                if p.required {
                    format!("{} (required)", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();

        format!(
            "{}: {} Category: {} Parameters: {}",
            tool.name,
            tool.description,
            tool.category,
            param_summary.join(", ")
        )
    }

    /// Match a query embedding against all tool entries.
    ///
    /// Returns the best matching tool if confidence ≥ threshold.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - L2-normalized query vector
    /// * `current_tier` - Current hardware tier for filtering
    ///
    /// # Returns
    ///
    /// `Some(ToolMatch)` if a tool matches above threshold, `None` otherwise.
    pub fn match_tool(&self, query_embedding: &[f32], current_tier: &str) -> Option<ToolMatch> {
        if self.entries.is_empty() {
            return None;
        }

        let tier_idx = TIER_ORDER
            .iter()
            .position(|&t| t == current_tier)
            .unwrap_or(0);

        let mut best: Option<ToolMatch> = None;

        for entry in &self.entries {
            // Skip tools above current hardware tier
            let entry_tier_idx = TIER_ORDER
                .iter()
                .position(|&t| t == entry.min_tier.as_str())
                .unwrap_or(0);
            if tier_idx < entry_tier_idx {
                continue;
            }

            let sim = embed::cosine_sim(query_embedding, &entry.embedding);

            if sim >= entry.threshold {
                if best.as_ref().map_or(true, |b| sim > b.confidence) {
                    best = Some(ToolMatch {
                        name: entry.name.clone(),
                        description: entry.description.clone(),
                        category: entry.category.clone(),
                        confidence: sim,
                        direct_execution: sim >= self.global_threshold,
                    });
                }
            }
        }

        if let Some(ref m) = best {
            debug!(
                tool = %m.name,
                confidence = m.confidence,
                direct = m.direct_execution,
                "Tool semantic match found"
            );
        }

        best
    }

    /// Match by raw text query (embeds the query first).
    ///
    /// Convenience method that combines embedding + matching.
    pub fn match_by_text(&self, text: &str, current_tier: &str) -> Option<ToolMatch> {
        let query_emb = embed::embed_one(text).ok()?;
        self.match_tool(&query_emb, current_tier)
    }

    /// Rebuild index with new tool definitions.
    ///
    /// Called when MCP servers are reconciled or tools are registered/deregistered.
    pub fn rebuild(&mut self, tools: &[ToolDef], config: &RoutingConfig) -> Result<()> {
        *self = Self::from_tool_defs(tools, config)?;
        Ok(())
    }

    /// Number of tools in the index.
    pub fn len(&self) -> usize {
        self.tool_count
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.tool_count == 0
    }

    /// Get all tool names in the index.
    pub fn tool_names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Get top-N matches (for debugging/analysis).
    pub fn top_matches(&self, query_embedding: &[f32], n: usize) -> Vec<ToolMatch> {
        let mut matches: Vec<ToolMatch> = self
            .entries
            .iter()
            .map(|entry| {
                let sim = embed::cosine_sim(query_embedding, &entry.embedding);
                ToolMatch {
                    name: entry.name.clone(),
                    description: entry.description.clone(),
                    category: entry.category.clone(),
                    confidence: sim,
                    direct_execution: sim >= self.global_threshold,
                }
            })
            .collect();

        matches.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        matches.truncate(n);
        matches
    }
}

// ─── Thread-Safe Wrapper ────────────────────────────────────────────────────

/// Thread-safe tool embedding index for concurrent access.
pub struct SharedToolIndex {
    index: RwLock<ToolEmbeddingIndex>,
}

impl SharedToolIndex {
    /// Create a new shared tool index.
    pub async fn new(
        tools: Vec<ToolDef>,
        config: RoutingConfig,
    ) -> Arc<Self> {
        let index = tokio::task::spawn_blocking(move || {
            ToolEmbeddingIndex::from_tool_defs(&tools, &config).unwrap_or_else(|e| {
                warn!("Failed to build tool index: {} — starting empty", e);
                ToolEmbeddingIndex::empty()
            })
        })
        .await
        .unwrap_or_else(|e| {
            warn!("Tool index build task failed: {} — starting empty", e);
            ToolEmbeddingIndex::empty()
        });

        Arc::new(Self {
            index: RwLock::new(index),
        })
    }

    /// Match a query embedding against tools.
    pub async fn match_tool(
        &self,
        query_embedding: &[f32],
        current_tier: &str,
    ) -> Option<ToolMatch> {
        let index = self.index.read().await;
        index.match_tool(query_embedding, current_tier)
    }

    /// Match by raw text query.
    pub async fn match_by_text(&self, text: &str, current_tier: &str) -> Option<ToolMatch> {
        let index = self.index.read().await;
        index.match_by_text(text, current_tier)
    }

    /// Rebuild the index with new tools.
    pub async fn rebuild(
        &self,
        tools: Vec<ToolDef>,
        config: RoutingConfig,
    ) -> Result<()> {
        let mut index = self.index.write().await;
        index.rebuild(&tools, &config)?;
        info!(tool_count = index.len(), "Tool index rebuilt");
        Ok(())
    }

    /// Number of tools indexed.
    pub async fn len(&self) -> usize {
        let index = self.index.read().await;
        index.len()
    }
}

// ─── Empty Index ────────────────────────────────────────────────────────────

impl ToolEmbeddingIndex {
    /// Create an empty index (for degraded mode).
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            global_threshold: DEFAULT_THRESHOLD,
            tool_count: 0,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::RiskLevel;

    fn make_test_tool(name: &str, desc: &str, category: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: desc.to_string(),
            category: category.to_string(),
            parameters: vec![],
            default_tier: RiskLevel::Green,
            min_tier: "lite",
        }
    }

    fn test_config() -> RoutingConfig {
        RoutingConfig::default()
    }

    #[test]
    fn empty_index_is_empty() {
        let index = ToolEmbeddingIndex::empty();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn empty_index_no_match() {
        let index = ToolEmbeddingIndex::empty();
        let emb = vec![0.1; 384];
        assert!(index.match_tool(&emb, "standard").is_none());
    }

    #[test]
    fn tier_filtering_works() {
        // This test verifies tier filtering logic without actual embeddings
        let entry = ToolEntry {
            name: "gpu_tool".into(),
            description: "GPU intensive tool".into(),
            category: "vision".into(),
            embedding: vec![0.1; 384],
            threshold: 0.5,
            min_tier: "performance".into(),
        };

        let index = ToolEmbeddingIndex {
            entries: vec![entry],
            global_threshold: 0.85,
            tool_count: 1,
        };

        // Lite tier should not match performance tool
        let emb = vec![0.1; 384];
        assert!(index.match_tool(&emb, "lite").is_none());

        // Performance tier should match
        assert!(index.match_tool(&emb, "performance").is_some());
    }

    #[test]
    fn tool_names_returns_all() {
        let index = ToolEmbeddingIndex {
            entries: vec![
                ToolEntry {
                    name: "tool_a".into(),
                    description: "A".into(),
                    category: "cat".into(),
                    embedding: vec![],
                    threshold: 0.85,
                    min_tier: "lite".into(),
                },
                ToolEntry {
                    name: "tool_b".into(),
                    description: "B".into(),
                    category: "cat".into(),
                    embedding: vec![],
                    threshold: 0.85,
                    min_tier: "lite".into(),
                },
            ],
            global_threshold: 0.85,
            tool_count: 2,
        };

        let names = index.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_b"));
    }

    #[test]
    fn rich_description_includes_params() {
        let mut tool = make_test_tool("set_volume", "Set system volume", "power");
        tool.parameters = vec![crate::tools::registry::ParamDef {
            name: "level".into(),
            param_type: "integer".into(),
            description: "Volume level 0-100".into(),
            required: true,
            default: None,
        }];

        let desc = ToolEmbeddingIndex::build_rich_description(&tool);
        assert!(desc.contains("set_volume"));
        assert!(desc.contains("Set system volume"));
        assert!(desc.contains("power"));
        assert!(desc.contains("level (required)"));
    }
}
