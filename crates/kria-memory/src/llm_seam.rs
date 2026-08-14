//! The one capability the memory subsystem needs from a language model.
//!
//! # Why a trait here instead of importing the LLM stack
//!
//! `memory/semantic_parser.rs` had the single outbound reference that tied this
//! whole 105,000-line subsystem to the rest of `kria-core`:
//!
//! ```text
//! use crate::llm_seam::SemanticExtractionLlm;
//! ```
//!
//! It made exactly one call — `chat()` — to turn a system prompt plus a user
//! prompt into text. Importing `LlmBackend` to get that dragged in a large trait
//! with its own type chain (`StructuredOutputMode`, grammar modes, streaming) that
//! memory neither uses nor should know about.
//!
//! So the dependency is **inverted**: memory declares the narrow thing it needs,
//! and the composition root supplies an adapter over whatever backend is
//! configured. Memory now compiles with no knowledge of which model you run —
//! which is both architecturally right and the reason this crate can be built
//! independently at all.
//!
//! # Why `Option` rather than `Result`
//!
//! Semantic extraction is **best-effort enrichment**. If the model is unreachable,
//! slow, or returns something unparseable, the correct behaviour is to store the
//! memory without the extracted facts — never to fail the user's write. `None`
//! carries exactly that meaning and makes the failure impossible to accidentally
//! propagate as an error.

use async_trait::async_trait;

/// A model that can turn one system + user exchange into text.
#[async_trait]
pub trait SemanticExtractionLlm: Send + Sync {
    /// Complete one exchange, returning the assistant's raw text.
    ///
    /// Returns `None` for any failure — unreachable model, timeout, refusal, or
    /// empty output. Callers treat that as "no extraction available", never as an
    /// error to surface.
    ///
    /// `max_tokens` bounds the reply. Extraction returns a small JSON object, so a
    /// caller that passes a large bound is wasting model time, not gaining detail.
    async fn extract(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
    ) -> Option<String>;
}
