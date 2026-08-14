//! Capability Provider Platform (CPP) — the provider-neutral capability boundary.
//!
//! This module is the single dependency surface KRIA's Brain uses to discover,
//! describe, permission, plan, acquire, execute, and learn from capabilities,
//! regardless of which **provider** supplies them (OpenClaw today; MCP servers,
//! GUI/browser/cloud/native providers tomorrow). It is the **anti-corruption
//! boundary**: nothing here references a provider-internal type. Provider-native
//! types live ONLY inside a provider adapter under [`acl`].
//!
//! # Why this exists
//!
//! Before CPP, KRIA-core reached directly into `openclaw::*` from several sites
//! (config, execution, mcp bridge), and the execution seam enumerated providers
//! in a closed `ExecutorKind` enum. That made "add/replace a provider" a
//! KRIA-core change. CPP inverts the dependency: the Brain depends only on
//! [`provider::CapabilityProvider`] + [`descriptor::CapabilityDescriptor`] +
//! [`protocol`] negotiation + the neutral value types, so a new provider is an
//! adapter, not a core edit.
//!
//! # The boundary, in one paragraph
//!
//! A [`provider::CapabilityProvider`] is identified by an open-vocabulary
//! [`ProviderId`] string (never an enum). It [`negotiate`]s a
//! [`protocol::ProtocolSession`] (version + feature intersection), [`describe`]s
//! its capabilities as [`descriptor::CapabilityDescriptor`]s, and [`execute`]s a
//! [`provider::CapabilityRequest`] into a [`provider::CapabilityOutcome`].
//! Optional facets (streaming, lifecycle/acquisition, batch, multi-modal I/O)
//! are negotiated — a provider that lacks one simply does not advertise it, and
//! its absence is never an error.
//!
//! [`negotiate`]: provider::CapabilityProvider::negotiate
//! [`describe`]: provider::CapabilityProvider::describe
//! [`execute`]: provider::CapabilityProvider::execute
//!
//! # Status (Milestone 1)
//!
//! This milestone establishes the boundary types only — nothing is wired to a
//! caller yet, and the whole platform is gated OFF by
//! [`config::CapabilityPlatformConfig::enabled`] (default `false`). With the
//! flag OFF, KRIA behaves byte-for-byte as it does today (the existing CIL /
//! OpenClaw path). Later milestones wire the OpenClaw adapter, the federated
//! index, permissions, planning, and the desktop surfaces.
//!
//! # Invariants (frozen by the CPP spec)
//!
//! - **No provider-native type** appears outside a provider adapter.
//! - **No hardcoded provider or capability names** in KRIA-core; identity is an
//!   open string, domains are open tag strings.
//! - **Registry federation:** each provider owns its authoritative catalog; the
//!   Brain holds only derived, rebuildable views.
//! - **Forward-compatibility:** unknown negotiated features and descriptor
//!   fields are carried in `extensions` maps, never rejected.

pub mod acl;
pub mod config;
pub mod conformance;
pub mod descriptor;
pub mod error;
pub mod events;
/// Deny-live fake provider (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;
pub mod grants;
pub mod index;
pub mod intelligence;
pub mod permission;
pub mod platform;
pub mod protocol;
pub mod provider;
pub mod registry;
pub mod state;


pub use config::{CapabilityIntelligenceConfig, CapabilityPlatformConfig, ProviderConfig};
pub use descriptor::{
    CapabilityDescriptor, CapabilityTag, CostHint, DescriptorVersion, Effect, Effects,
    Expectations, FailureExample, Guidance, IoExample, Modality, QualitySignals, ResourceClass,
    Reversibility, TriggerExample, TrustInfo, UsageStats,
};
pub use error::CapError;
pub use events::{CapabilityEvent, CapabilityEventBus, Outcome, SharedEventBus, Stage};
pub use intelligence::{
    infer_family, infer_kind, CapabilityFamily, CapabilityKind, GoalClass, ReasoningPolicy,
    REASONING_POLICY_VERSION, TELEMETRY_SCHEMA_VERSION,
};
pub use protocol::{
    ClientCapabilities, Feature, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
pub use provider::{
    AcquireRequest, CapabilityChunk, CapabilityOutcome, CapabilityProvider, CapabilityRequest,
    RequestContext,
};
pub use state::{CapabilityState, ProviderState};

pub use acl::{McpProvider, OpenClawProvider};
pub use conformance::{run_conformance, ConformanceCheck, ConformanceReport};
pub use grants::{GrantDecision, GrantStore, ScopeKind, ScopedGrant};
pub use index::{
    Embedder, FederatedIndex, FusionWeights, InMemoryFederatedIndex, MemoryEmbedder,
    ScoredDescriptor,
};
pub use permission::{
    approval_grant, AuthorizeRequest, DefaultPermissionEngine, PermissionDecision,
    PermissionEngine, PermissionTier, PromptSpec,
};
pub use platform::CapabilityPlatform;
pub use registry::{ProviderRefresh, ProviderRegistry, RefreshReport};

/// Open-vocabulary provider identifier (e.g. `"openclaw"`, `"mcp:github"`,
/// `"gui.cognition"`, `"browser"`, `"cloud.dalle"`).
///
/// This is deliberately a `String`, **never an enum**: adding a provider must
/// not require editing a KRIA-core type. It replaces the closed
/// `execution::ExecutorKind` enum at the execution seam (done in Milestone 2).
pub type ProviderId = String;
