//! Capability and provider lifecycle **state machines**.
//!
//! These are the provider-neutral generalizations of the authoritative machines
//! that already exist in the codebase:
//! - [`CapabilityState`] generalizes `openclaw::registry::SkillState` so every
//!   provider's capabilities move through the same lifecycle.
//! - [`ProviderState`] is derived from the negotiated
//!   [`crate::capability::protocol::ProtocolSession`] plus provider health.
//!
//! They exist so lifecycle transitions are **deterministic, observable, and
//! identical across providers**: every transition is validated by
//! [`CapabilityState::can_transition_to`] /
//! [`ProviderState::can_transition_to`], and an illegal transition is a bug the
//! caller must reject (and audit) rather than silently apply.
//!
//! No second, parallel state representation is introduced: the OpenClaw adapter
//! maps `SkillState` ⇄ [`CapabilityState`], and container-level execution state
//! remains owned by `openclaw::runtime_manager::ContainerState` (the execution
//! lifecycle is a view over that, per the design).

use serde::{Deserialize, Serialize};

/// The lifecycle of a single capability, across any provider.
///
/// Generalizes `SkillState`. Transitions:
/// `Discovered → Available → Installed → Validated → Ready → Executing → Ready`
/// with `Ready|Executing → Failed → Recovering → Ready|Deprecated`, and
/// `Ready → Deprecated → Removed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    /// Seen in a catalog/provider but not yet indexed as usable.
    Discovered,
    /// Indexed and offered for acquisition (a descriptor exists).
    Available,
    /// Acquired/installed by the provider (lifecycle facet), not yet verified.
    Installed,
    /// Verified (hash/signature/schema) — safe to enable.
    Validated,
    /// Enabled and usable.
    Ready,
    /// Currently executing.
    Executing,
    /// Errored or timed out.
    Failed,
    /// Undergoing recovery (retry/restart/repair).
    Recovering,
    /// Superseded or explicitly deprecated; discouraged from use.
    Deprecated,
    /// Uninstalled/removed from the system.
    Removed,
}

impl CapabilityState {
    /// Stable lowercase identifier (stable across serialization + telemetry).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Available => "available",
            Self::Installed => "installed",
            Self::Validated => "validated",
            Self::Ready => "ready",
            Self::Executing => "executing",
            Self::Failed => "failed",
            Self::Recovering => "recovering",
            Self::Deprecated => "deprecated",
            Self::Removed => "removed",
        }
    }

    /// Whether the capability is currently usable for execution.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Whether `next` is a permitted transition from `self`.
    ///
    /// The machine is intentionally strict: unknown/illegal jumps (e.g.
    /// `Discovered → Executing`) are rejected so callers cannot leave a
    /// capability in an inconsistent state. `Removed` is terminal.
    pub fn can_transition_to(&self, next: CapabilityState) -> bool {
        use CapabilityState::*;
        match self {
            Discovered => matches!(next, Available | Removed),
            Available => matches!(next, Installed | Deprecated | Removed),
            Installed => matches!(next, Validated | Failed | Removed),
            Validated => matches!(next, Ready | Failed | Removed),
            Ready => matches!(next, Executing | Deprecated | Failed | Removed),
            Executing => matches!(next, Ready | Failed),
            Failed => matches!(next, Recovering | Deprecated | Removed),
            Recovering => matches!(next, Ready | Deprecated | Failed | Removed),
            Deprecated => matches!(next, Removed | Ready),
            Removed => false,
        }
    }
}

/// The lifecycle of a provider connection, derived from negotiation + health.
///
/// `Offline → Connecting → Negotiating → Ready → Syncing → Healthy → Busy`,
/// with `Healthy → Degraded → Healthy`, `Healthy → Updating → Negotiating`, and
/// `Degraded → Disconnected → Offline` (circuit breaker / lost connection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    /// Not connected.
    Offline,
    /// Transport is being established.
    Connecting,
    /// Handshake in progress (version + feature negotiation).
    Negotiating,
    /// Negotiated; ready to describe/sync.
    Ready,
    /// Fetching/describing its capability catalog.
    Syncing,
    /// Fully synced and serving discovery.
    Healthy,
    /// Executing one or more capabilities.
    Busy,
    /// Partially failing or slow; still usable with caution.
    Degraded,
    /// Provider is self-updating; will re-negotiate afterward.
    Updating,
    /// Circuit breaker open / connection lost; excluded from use.
    Disconnected,
}

impl ProviderState {
    /// Stable lowercase identifier for serialization + telemetry + the
    /// `provider_sessions.health` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Connecting => "connecting",
            Self::Negotiating => "negotiating",
            Self::Ready => "ready",
            Self::Syncing => "syncing",
            Self::Healthy => "healthy",
            Self::Busy => "busy",
            Self::Degraded => "degraded",
            Self::Updating => "updating",
            Self::Disconnected => "disconnected",
        }
    }

    /// Whether the provider should currently be offered in discovery/execution.
    pub fn is_serving(&self) -> bool {
        matches!(self, Self::Healthy | Self::Busy | Self::Ready)
    }

    /// Whether `next` is a permitted transition from `self`.
    pub fn can_transition_to(&self, next: ProviderState) -> bool {
        use ProviderState::*;
        match self {
            Offline => matches!(next, Connecting),
            Connecting => matches!(next, Negotiating | Disconnected | Offline),
            Negotiating => matches!(next, Ready | Disconnected | Offline),
            Ready => matches!(next, Syncing | Busy | Degraded | Disconnected),
            Syncing => matches!(next, Healthy | Degraded | Disconnected),
            Healthy => matches!(next, Busy | Degraded | Updating | Syncing | Disconnected),
            Busy => matches!(next, Healthy | Degraded | Disconnected),
            Degraded => matches!(next, Healthy | Disconnected | Offline),
            Updating => matches!(next, Negotiating | Disconnected | Offline),
            Disconnected => matches!(next, Offline | Connecting),
        }
    }
}
