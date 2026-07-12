//! The [`CapabilityProvider`] trait — the anti-corruption boundary itself — plus
//! the neutral execution request/result types.
//!
//! Every capability source KRIA can use (OpenClaw, MCP servers, GUI/browser/
//! cloud/native providers) implements this trait behind an adapter under
//! `capability::acl::*`. The Brain depends only on this trait and the neutral
//! value types; it never sees a provider-internal type.
//!
//! # Mandatory vs optional
//!
//! `negotiate`, `describe`, and `execute` are mandatory. `acquire`/`remove`
//! (the lifecycle facet) default to [`CapError::Unsupported`], so a read-only
//! provider (e.g. a plain MCP server) is valid without implementing them — the
//! platform only *calls* them when the negotiated session advertised
//! [`Feature::Lifecycle`](super::protocol::Feature::Lifecycle).

use async_trait::async_trait;
use futures::stream::BoxStream;

use super::descriptor::{CapabilityDescriptor, Effect};
use super::error::CapError;
use super::protocol::{ClientCapabilities, ProtocolSession, ProviderHealth};
use super::ProviderId;

/// Per-request context threaded from the Brain to a provider: correlation for
/// telemetry, and the scope the permission decision was made against.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// Correlation id linking every telemetry/audit record for this goal.
    pub correlation_id: String,
    /// Active chat session id, if any (for session-scoped grants).
    pub session_id: Option<String>,
    /// Active workspace id, if any (for workspace-scoped grants).
    pub workspace_id: Option<String>,
}

impl RequestContext {
    /// Create a context with a fresh correlation id.
    pub fn new() -> Self {
        Self {
            correlation_id: uuid::Uuid::new_v4().to_string(),
            session_id: None,
            workspace_id: None,
        }
    }
}

/// A neutral, validated request to execute one capability.
#[derive(Debug, Clone)]
pub struct CapabilityRequest {
    pub provider_id: ProviderId,
    pub capability_id: String,
    /// Arguments, validated by the caller against the descriptor `input_schema`.
    pub args: serde_json::Value,
    pub context: RequestContext,
    /// The effect classes the permission engine actually granted for this run.
    /// The provider must not exceed these.
    pub granted_effects: Vec<Effect>,
}

/// One chunk of a streamed capability result (only used when the `Streaming`
/// facet was negotiated).
#[derive(Debug, Clone)]
pub struct CapabilityChunk {
    pub data: serde_json::Value,
}

/// The outcome of executing a capability. Honest by construction: a provider
/// that did not actually produce a result returns [`CapabilityOutcome::Declined`],
/// never a fabricated success.
pub enum CapabilityOutcome {
    /// A single, final result value.
    Value(serde_json::Value),
    /// A stream of chunks (only if the `Streaming` facet was negotiated).
    Stream(BoxStream<'static, CapabilityChunk>),
    /// The provider honestly declined (with a reason). Not an error — an
    /// explicit "did not run / cannot run" outcome.
    Declined { reason: String },
}

impl std::fmt::Debug for CapabilityOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityOutcome::Value(v) => f.debug_tuple("Value").field(v).finish(),
            CapabilityOutcome::Stream(_) => f.write_str("Stream(..)"),
            CapabilityOutcome::Declined { reason } => {
                f.debug_struct("Declined").field("reason", reason).finish()
            }
        }
    }
}

/// A request to acquire a capability that satisfies a needed tag, via a
/// provider's lifecycle facet (install from marketplace / generate).
#[derive(Debug, Clone)]
pub struct AcquireRequest {
    /// The capability tag the goal needs (open vocabulary).
    pub capability_tag: String,
    /// Optional free-text hint (e.g. the original goal) to guide selection.
    pub hint: Option<String>,
    /// The **Brain-selected** specific catalog capability id to install, when the
    /// Brain has already ranked the provider's catalog and chosen one (spec R8/
    /// R9.4 — match-selection is the Brain's job, not the provider's). When
    /// `Some`, the provider MUST install exactly this capability and MUST NOT
    /// re-resolve a different match. When `None`, the provider may resolve the
    /// best match itself (backward-compatible / thin-caller path).
    pub capability_id: Option<String>,
    /// A **Brain-proposed Capability-Graph IR** (neutral serialized JSON) for a
    /// synthesizing provider to persist + run (Wave 9, W9-R11). The Brain owns
    /// *which* IR to synthesize (deterministic or LLM-assisted proposer); the
    /// provider re-validates it (safety, not cognition) and executes. `None` ⇒
    /// the provider derives a deterministic spec from the goal itself.
    pub proposed_graph: Option<serde_json::Value>,
    pub context: RequestContext,
}

impl AcquireRequest {
    /// Convenience constructor for a goal-driven acquisition (no pre-selected id,
    /// no proposed graph). Keeps call sites concise + forward-compatible as new
    /// optional fields are added.
    pub fn for_goal(goal: impl Into<String>) -> Self {
        let goal = goal.into();
        Self {
            capability_tag: goal.clone(),
            hint: Some(goal),
            capability_id: None,
            proposed_graph: None,
            context: RequestContext::new(),
        }
    }
}

/// The anti-corruption boundary. Implemented once per provider, inside an
/// adapter under `capability::acl::*`.
///
/// Implementors MUST:
/// - be honest (no fabricated success; use [`CapabilityOutcome::Declined`] or an
///   error),
/// - keep all provider-native types inside the adapter,
/// - advertise only facets they actually support in [`Self::negotiate`].
#[async_trait]
pub trait CapabilityProvider: Send + Sync {
    /// The provider's open-vocabulary id (stable for the process lifetime).
    fn provider_id(&self) -> &ProviderId;

    /// Negotiate protocol version + feature set with the client. Mandatory.
    /// Baseline for a thin provider: the mandatory facets only.
    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError>;

    /// Return the provider's current (installed/available-now) capability
    /// descriptors. Mandatory. A thin provider derives conservative defaults
    /// (see [`CapabilityDescriptor::minimal`]).
    async fn describe(
        &self,
        session: &ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError>;

    /// Return the provider's **catalog** — capabilities that are installable but
    /// not yet installed (marketplace federation). Used for recommendations on a
    /// goal miss; each catalog descriptor carries `extensions["installed"] =
    /// false`. Optional: default empty (a provider with no marketplace).
    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(Vec::new())
    }

    /// Execute one capability. Mandatory.
    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError>;

    /// Acquire a capability (install/generate). Optional lifecycle facet —
    /// the platform calls this ONLY when the session advertised
    /// [`Feature::Lifecycle`](super::protocol::Feature::Lifecycle). Default:
    /// unsupported (read-only provider).
    async fn acquire(&self, _req: &AcquireRequest) -> Result<CapabilityDescriptor, CapError> {
        Err(CapError::Unsupported("lifecycle".into()))
    }

    /// Remove/uninstall a capability. Optional lifecycle facet. Default:
    /// unsupported.
    async fn remove(&self, _capability_id: &str) -> Result<(), CapError> {
        Err(CapError::Unsupported("lifecycle".into()))
    }

    /// Current provider health, for the provider state machine + degraded
    /// handling.
    async fn health(&self) -> ProviderHealth;
}
