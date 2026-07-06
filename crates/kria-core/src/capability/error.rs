//! The single error type for the Capability Provider Platform.
//!
//! Every fallible CPP operation returns [`CapError`]. Variants are chosen so the
//! failure is **user-actionable** and honest: no variant silently swallows a
//! failure, and each maps to a clear message the Brain can surface or audit.
//! Provider adapters translate their internal errors into these neutral
//! variants so the Brain never sees a provider-specific error type.

use thiserror::Error;

/// A provider-neutral capability-platform error.
///
/// The variants mirror the stages of the capability flow (negotiate → describe →
/// discover → permission → acquire → execute) plus the two cross-cutting honest
/// failure modes (`Degraded`, `ProviderOffline`) and `Io`.
#[derive(Debug, Error)]
pub enum CapError {
    /// The negotiation handshake with a provider failed (no mutually supported
    /// version, malformed handshake, or timeout).
    #[error("provider negotiation failed: {0}")]
    Negotiation(String),

    /// The provider does not support the requested optional protocol facet
    /// (e.g. `"lifecycle"`, `"streaming"`, `"batch"`). This is the honest,
    /// expected result for a read-only provider — callers must treat it as
    /// "facet absent", not a hard failure.
    #[error("protocol facet not supported: {0}")]
    Unsupported(String),

    /// A capability descriptor was missing required fields or failed validation.
    #[error("invalid capability descriptor: {0}")]
    Descriptor(String),

    /// Discovery / retrieval over the federated index failed.
    #[error("capability discovery failed: {0}")]
    Discovery(String),

    /// The permission engine denied the request or requires approval that was
    /// not granted.
    #[error("permission denied or approval required: {0}")]
    Permission(String),

    /// Acquisition (install/generate/update via a provider's lifecycle facet)
    /// failed or was declined. Never returned as a fake success.
    #[error("capability acquisition failed: {0}")]
    Acquire(String),

    /// Execution of a capability failed.
    #[error("capability execution failed: {0}")]
    Execute(String),

    /// The platform is operating in a degraded mode (e.g. embedder unavailable,
    /// network down). Results, if any, are explicitly lower-fidelity and must be
    /// reported as such — never presented as full-fidelity.
    #[error("degraded mode: {0}")]
    Degraded(String),

    /// A provider is offline/unreachable (or its circuit breaker is open).
    #[error("provider offline: {0}")]
    ProviderOffline(String),

    /// An underlying I/O / persistence error.
    #[error("i/o error: {0}")]
    Io(String),
}

impl CapError {
    /// True when the error is the honest "provider lacks this optional facet"
    /// signal, which callers handle by degrading rather than surfacing an error.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, CapError::Unsupported(_))
    }

    /// True when the error reflects a transient availability problem
    /// (degraded/offline) rather than a hard, permanent failure.
    pub fn is_transient(&self) -> bool {
        matches!(self, CapError::Degraded(_) | CapError::ProviderOffline(_))
    }
}
