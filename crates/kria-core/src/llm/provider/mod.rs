//! Universal Model Provider Abstraction Layer
//!
//! This module implements a provider-agnostic runtime that allows KRIA to
//! seamlessly operate across local models (Ollama, llama.cpp), cloud APIs
//! (OpenAI, Gemini, Anthropic, OpenRouter), and future providers without
//! leaking provider-specific logic into the orchestration layer.
//!
//! Architecture:
//! - `ProviderCapability` — normalized capability contracts
//! - `ProviderConfig` — per-provider configuration
//! - `ProviderRegistry` — runtime provider management
//! - `ProviderBackend` trait — unified provider interface
//! - `ProviderError` — normalized error types
//! - `ConnectionTest` — instant provider validation

pub mod capabilities;
pub mod config;
pub mod connection_test;
pub mod error;
pub mod registry;
pub mod streaming;
pub mod types;

// Provider implementations
pub mod anthropic;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod openrouter;

pub use capabilities::{ModelCapabilities, ProviderCapability};
pub use config::{ProviderConfig, ProviderEndpointConfig, ProviderType};
pub use connection_test::{ConnectionTestResult, ConnectionTestStatus};
pub use error::{ProviderError, ProviderErrorKind};
pub use registry::ProviderRegistry;
pub use streaming::UnifiedStream;
pub use types::{ModelInfo, ProviderStatus};

#[cfg(test)]
mod tests;
