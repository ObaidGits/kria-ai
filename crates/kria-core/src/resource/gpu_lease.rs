// ============================================================================
// LEGACY COMPONENT
//
// This single-holder GPU lease manager has been replaced by the Hardware
// Resource Authority (co-residency admission + priority preemption).
//
// Runtime Status:
//     INACTIVE. The single-holder state machine, recovery workers, queueing,
//     and telemetry reconciliation that used to live here have been physically
//     removed (Task 62). What remains is a THIN COMPATIBILITY SHELL:
//       * `acquire_guard_gated` — the ONE production admission entry point.
//         Always routes through `resource::authority::HraService::admit_gpu`
//         when an HRA is registered (production desktop runtime always is).
//       * A handful of trivial stub methods (`acquire_token` / `release_token`
//         / `reconcile` / `refresh` / `state` / `set_resource_telemetry` /
//         `clear_resource_telemetry`) kept ONLY so existing callers compile
//         unchanged. They perform NO arbitration — HRA owns all of it.
//
// Ownership:
//     Historical compatibility only. HRA owns GPU residency + arbitration.
//
// DO NOT ADD NEW FEATURES HERE.
// DO NOT FIX NEW BUGS HERE. (Fix them in resource::authority::*.)
//
// Safe for deletion once every caller migrates off the compatibility stubs
// and takes an `AdmissionGuard` directly. See tasks.md Task 62.
//
// Replacement:
//     resource::authority::{service::HraService, co_residency::CoResidencyManager,
//                           scheduler::Scheduler, ra::LocalAuthority}
// ============================================================================

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::telemetry::{ResourceSnapshot, ResourceTelemetry};

/// Process-wide GPU lease shell. Every GPU consumer (LLM, image, vision, speech) acquires through
/// THIS one instance via [`acquire_guard_gated`](GpuLeaseManager::acquire_guard_gated), which routes
/// admission to the Hardware Resource Authority. Lazily created on first use.
static GLOBAL_GPU_LEASE: OnceLock<Arc<GpuLeaseManager>> = OnceLock::new();

/// Get the shared, process-wide GPU lease shell.
pub fn global_gpu_lease() -> Arc<GpuLeaseManager> {
    GLOBAL_GPU_LEASE
        .get_or_init(|| Arc::new(GpuLeaseManager::new()))
        .clone()
}

/// Opaque lease token. Retained only for API compatibility with legacy callers; carries no state
/// now that arbitration lives in HRA.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LeaseToken(u64);

impl LeaseToken {
    pub fn value(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageLeaseBackendId {
    ComfyUi,
    CloudFallback,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuOwner {
    L1Worker,
    ImageBackend(ImageLeaseBackendId),
    Vision,
    Speech,
    Maintenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryReason {
    LeaseExpired,
    TelemetryMismatch(String),
    OwnerReleaseRequested,
    GuardReleasedAwaitingTelemetry,
    ShutdownRequested,
    Unknown,
}

/// Lease state. Kept for callers that pattern-match on it (e.g. image status reporting). The shell
/// always reports `Idle` because HRA — not this manager — tracks real residency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuLeaseState {
    Idle,
    Held {
        owner: GpuOwner,
        turn_id: String,
    },
    Recovering {
        owner: Option<GpuOwner>,
        reason: RecoveryReason,
    },
    Degraded {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GpuLeaseError {
    #[error("gpu lease busy: currently held by {owner:?}")]
    Busy { owner: GpuOwner },
    #[error("gpu lease recovering: {reason:?}")]
    Recovering { reason: RecoveryReason },
    #[error("gpu lease degraded: {reason}")]
    Degraded { reason: String },
}

pub type LeaseGuard = GpuLeaseGuard;
pub type LeaseError = GpuLeaseError;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GpuPathSnapshot {
    pub gpu_active: bool,
    pub note: Option<String>,
}

/// Thin GPU lease compatibility shell — see the LEGACY COMPONENT banner above.
///
/// All real GPU arbitration (admission, priority preemption, co-residency, recovery) lives in
/// `resource::authority`. This type exists only so pre-HRA call sites keep compiling while they
/// migrate to taking an `AdmissionGuard` directly.
pub struct GpuLeaseManager {
    _priv: (),
}

impl Default for GpuLeaseManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuLeaseManager {
    pub fn new() -> Self {
        Self { _priv: () }
    }

    /// Compatibility constructor. The `default_ttl` / `recovery_timeout` args are ignored — the
    /// shell holds no lease state — but the signature is preserved for existing callers.
    pub fn shared(_default_ttl: Duration, _recovery_timeout: Duration) -> Arc<Self> {
        Arc::new(Self::new())
    }

    // --- Compatibility stubs (no-ops). HRA owns arbitration; these keep callers compiling. ---

    /// STUB: telemetry now flows to HRA, not the lease shell. No-op.
    pub fn set_resource_telemetry(&self, _telemetry: Arc<dyn ResourceTelemetry>) {}

    /// STUB: no-op (see [`set_resource_telemetry`](Self::set_resource_telemetry)).
    pub fn clear_resource_telemetry(&self) {}

    /// STUB: the shell holds no state; always reports `Idle`. HRA tracks real residency.
    pub fn state(&self) -> GpuLeaseState {
        GpuLeaseState::Idle
    }

    /// STUB: legacy single-holder token acquire. Always succeeds with a sentinel token; performs no
    /// arbitration. Real admission goes through [`acquire_guard_gated`](Self::acquire_guard_gated).
    pub fn acquire_token(
        &self,
        _owner: GpuOwner,
        _turn_id: impl Into<String>,
        _ttl: Option<Duration>,
    ) -> Result<LeaseToken, GpuLeaseError> {
        Ok(LeaseToken(0))
    }

    /// STUB: legacy token release. No-op; returns true for callers that check success.
    pub fn release_token(self: &Arc<Self>, _token: &LeaseToken, _reason: RecoveryReason) -> bool {
        true
    }

    /// STUB: legacy TTL refresh. No-op; returns true.
    pub fn refresh(&self, _token: &LeaseToken, _ttl: Option<Duration>) -> bool {
        true
    }

    /// STUB: legacy telemetry reconciliation. No-op — HRA reconciles residency itself.
    pub fn reconcile(&self, _snapshot: &ResourceSnapshot) {}

    /// Runtime-ownership acquire — the SINGLE admission entry point for every GPU consumer.
    ///
    /// **Fix-Forward (HRA is the only architecture):** admission always routes through the Hardware
    /// & Resource Authority (`HraService::admit_gpu` → Co-Residency manager). HRA owns the decision
    /// (budget + priority preemption + co-residency); the consumer executes its own model load
    /// (admission-only ownership). On denial the consumer falls back (CPU/cloud/Tier-B) via `Busy`.
    ///
    /// When no HRA is registered (headless unit tests / very early boot) this grants an ungated
    /// guard so tests can proceed without an authority. In the production desktop runtime the HRA is
    /// always registered.
    ///
    /// `vram_hint_mb` is the consumer's estimated VRAM need for budget accounting.
    pub async fn acquire_guard_gated(
        self: &Arc<Self>,
        owner: GpuOwner,
        turn_id: impl Into<String>,
        _ttl: Option<Duration>,
        vram_hint_mb: u64,
    ) -> Result<GpuLeaseGuard, GpuLeaseError> {
        use crate::resource::authority::{global_hra, ResidencyTarget};
        let turn_id = turn_id.into();

        if let Some(hra) = global_hra() {
            let (consumer, class) = map_owner_to_hra(&owner);
            let req = build_hra_request(consumer, class, vram_hint_mb, &turn_id);
            tracing::info!(
                target: "hra",
                consumer = ?consumer, owner = ?owner, class = class.as_str(),
                vram_mb = vram_hint_mb,
                "[HRA][{:?}] Admission Requested (sole runtime owner)", consumer
            );
            return match hra.admit_gpu(&req, ResidencyTarget::Hot).await {
                Ok(g) => {
                    tracing::info!(
                        target: "hra",
                        consumer = ?consumer, owner = ?owner,
                        "[HRA][{:?}] Admission Granted", consumer
                    );
                    Ok(GpuLeaseGuard::hra(g))
                }
                Err(e) => {
                    tracing::warn!(
                        target: "hra",
                        consumer = ?consumer, owner = ?owner, reason = ?e,
                        "[HRA][{:?}] Admission Denied — consumer falls back (CPU/Tier-B/cloud)", consumer
                    );
                    Err(GpuLeaseError::Busy { owner })
                }
            };
        }

        // No HRA registered → headless test / pre-init only. Ungated pass-through.
        Ok(GpuLeaseGuard::ungated())
    }
}

/// Map a legacy `GpuOwner` to the HRA `(ConsumerId, PriorityClass)` used for admission. Centralizes
/// the priority policy: LLM is foreground-interactive (preempts others), voice is realtime-protected,
/// image/vision are interactive-background (yield to chat/voice), maintenance is lowest.
fn map_owner_to_hra(
    owner: &GpuOwner,
) -> (
    crate::resource::authority::ConsumerId,
    crate::resource::authority::PriorityClass,
) {
    use crate::resource::authority::{ConsumerId, PriorityClass};
    match owner {
        GpuOwner::L1Worker => (ConsumerId::Llm, PriorityClass::InteractiveFg),
        GpuOwner::ImageBackend(_) => (ConsumerId::Image, PriorityClass::InteractiveBg),
        GpuOwner::Vision => (ConsumerId::Vision, PriorityClass::InteractiveBg),
        GpuOwner::Speech => (ConsumerId::Stt, PriorityClass::RealtimeVoice),
        GpuOwner::Maintenance => (ConsumerId::Agent, PriorityClass::Maintenance),
    }
}

/// Build an admission-only HRA request (model_id `None` → HRA owns the reservation/preemption
/// decision; the consumer still executes its own model load — no duplicate loading).
fn build_hra_request(
    consumer: crate::resource::authority::ConsumerId,
    class: crate::resource::authority::PriorityClass,
    vram_mb: u64,
    turn_id: &str,
) -> crate::resource::authority::ResourceRequest {
    use crate::resource::authority::{Constraints, ResourceNeed, ResourceRequest, TurnId};
    ResourceRequest {
        consumer,
        class,
        need: ResourceNeed {
            vram_mb,
            ram_mb: 0,
            cpu_threads: 0,
            exclusivity: false,
            model_id: None,
            est_ms: 0,
        },
        constraints: Constraints::default(),
        turn_id: TurnId(turn_id.to_string()),
    }
}

/// RAII GPU admission guard. In the production runtime it wraps an HRA `AdmissionGuard`; dropping it
/// releases the co-residency reservation. In headless tests with no HRA it is an inert pass-through.
pub struct GpuLeaseGuard {
    /// The HRA admission backing this guard. `None` = ungated test/pre-init pass-through.
    hra_guard: Option<crate::resource::authority::AdmissionGuard>,
    released: bool,
}

impl GpuLeaseGuard {
    /// Construct a guard backed by an HRA admission (production path — HRA owns the residency).
    fn hra(admission: crate::resource::authority::AdmissionGuard) -> Self {
        Self {
            hra_guard: Some(admission),
            released: false,
        }
    }

    /// Construct an inert pass-through guard (headless test / pre-init, no HRA registered).
    fn ungated() -> Self {
        Self {
            hra_guard: None,
            released: false,
        }
    }

    /// Sentinel token for legacy callers. The shell tracks no per-token state.
    pub fn token(&self) -> LeaseToken {
        LeaseToken(0)
    }

    /// True while the underlying residency is still valid. For HRA-owned guards this reflects
    /// cooperative preemption (a higher-priority request may have revoked it); ungated guards are
    /// always valid.
    pub fn is_valid(&self) -> bool {
        match &self.hra_guard {
            Some(g) => g.is_valid(),
            None => true,
        }
    }

    /// Whether this guard is owned by HRA (production) vs an ungated test pass-through.
    pub fn is_hra_owned(&self) -> bool {
        self.hra_guard
            .as_ref()
            .map(|g| g.is_enforced())
            .unwrap_or(false)
    }

    pub fn release(&mut self, _reason: RecoveryReason) {
        if self.released {
            return;
        }
        self.released = true;
        // HRA-owned guards release by dropping `hra_guard`; ungated guards have nothing to release.
        self.hra_guard = None;
    }
}

impl Drop for GpuLeaseGuard {
    fn drop(&mut self) {
        self.release(RecoveryReason::GuardReleasedAwaitingTelemetry);
        // `hra_guard` (if any) drops here, releasing the co-residency reservation.
    }
}
