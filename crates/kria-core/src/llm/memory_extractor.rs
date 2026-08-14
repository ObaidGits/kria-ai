//! Adapter satisfying `kria-memory`'s narrow language-model seam.
//!
//! # Why this file exists
//!
//! `kria-memory` needs one capability — turn a system + user prompt into text — and
//! declares it as [`SemanticExtractionLlm`] rather than importing this crate's
//! `LlmBackend`. That inversion is what lets a 105,000-line subsystem compile
//! without knowing which model is configured.
//!
//! The adapter is the price of that: exactly one place translates between the two.
//! It lives here, in the crate that owns the LLM stack, because this is the side
//! that knows about `ChatMessage`, temperature and token budgets.

use std::sync::Arc;

use async_trait::async_trait;
use kria_memory::llm_seam::SemanticExtractionLlm;

use crate::llm::{ChatMessage, LlmBackend};

/// Wraps any configured backend as the memory subsystem's extraction model.
pub struct LlmBackendExtractor {
    backend: Arc<dyn LlmBackend>,
}

impl LlmBackendExtractor {
    /// Wrap a backend.
    #[must_use]
    pub fn new(backend: Arc<dyn LlmBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl SemanticExtractionLlm for LlmBackendExtractor {
    async fn extract(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
    ) -> Option<String> {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
                name: None,
                images: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
                name: None,
                images: None,
            },
        ];
        // Temperature 0: extraction must be repeatable, not creative. Two identical
        // turns should yield the same facts, or the memory store fills with
        // near-duplicate variants of the same statement.
        let response = self
            .backend
            .chat(&messages, None, 0.0, max_tokens)
            .await
            .ok()?;
        // An empty reply is reported as "no extraction", not as an empty fact set:
        // the caller must not store a successful-looking extraction of nothing.
        let content = response.content.trim();
        (!content.is_empty()).then(|| content.to_string())
    }
}
