//! OpenClaw HRA admission (resource-contract INV-5).
//!
//! All OpenClaw execution is admitted through the Hardware & Resource Authority — the same
//! authority that governs LLM/voice/vision. OpenClaw runs as `ConsumerId::Ext` at
//! `PriorityClass::InteractiveBg` (below realtime voice; yields to foreground chat), so a heavy
//! skill can never starve a voice turn.
//!
//! A1 scope: CPU/RAM admission (no GPU). The lease is bound to the running instance and released
//! on drop (`OpenClawLease`), so cancellation/cleanup always frees the resource.

use super::types::ResourceClass;
use crate::resource::authority::{
    global_hra, Constraints, ConsumerId, HraService, LeaseToken, PriorityClass, RaOutcome,
    ResourceNeed, ResourceRequest, TurnId,
};
use std::sync::Arc;

/// Why admission failed (maps to `event::FailureKind::AdmissionDenied`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionError {
    /// Device/authority contended by equal-or-higher priority work.
    Busy,
    /// Shed due to overload (queue full).
    Shed,
    /// A higher-priority request needs this victim preempted first (A1: treated as busy).
    PreemptRequired,
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("HRA admission busy"),
            Self::Shed => f.write_str("HRA admission shed (overloaded)"),
            Self::PreemptRequired => f.write_str("HRA admission requires preemption"),
        }
    }
}

/// RAII HRA lease for one OpenClaw invocation. Dropping it releases the lease back to the HRA.
/// When no HRA is registered (headless tests / very early boot) this is an inert pass-through.
pub struct OpenClawLease {
    inner: Option<(Arc<HraService>, LeaseToken)>,
    released: bool,
}

impl OpenClawLease {
    fn granted(hra: Arc<HraService>, token: LeaseToken) -> Self {
        Self {
            inner: Some((hra, token)),
            released: false,
        }
    }

    fn ungated() -> Self {
        Self {
            inner: None,
            released: false,
        }
    }

    /// Explicitly release the lease early (idempotent). Also called on drop.
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        if let Some((hra, token)) = self.inner.take() {
            hra.release(token);
        }
    }
}

impl Drop for OpenClawLease {
    fn drop(&mut self) {
        self.release();
    }
}

/// Admit one OpenClaw invocation of the given resource class. Returns a lease guard on success.
pub fn admit(
    resource_class: ResourceClass,
    correlation_id: &str,
) -> Result<OpenClawLease, AdmissionError> {
    let Some(hra) = global_hra() else {
        // No authority registered (unit tests / pre-init). Proceed ungated.
        return Ok(OpenClawLease::ungated());
    };

    let (ram_mb, cpu_threads) = match resource_class {
        ResourceClass::Light => (256, 1),
        ResourceClass::Medium => (512, 1),
        ResourceClass::Heavy => (2048, 2),
    };

    let req = ResourceRequest {
        consumer: ConsumerId::Ext,
        class: PriorityClass::InteractiveBg,
        need: ResourceNeed {
            vram_mb: 0, // A1: CPU/RAM only. GPU skills arrive in a later phase.
            ram_mb,
            cpu_threads,
            exclusivity: false,
            model_id: None,
            est_ms: 0,
        },
        constraints: Constraints::default(),
        turn_id: TurnId(correlation_id.to_string()),
    };

    match hra.request(&req) {
        RaOutcome::Granted(lease) => Ok(OpenClawLease::granted(hra, lease.token)),
        RaOutcome::Busy => Err(AdmissionError::Busy),
        RaOutcome::Shed => Err(AdmissionError::Shed),
        RaOutcome::PreemptThenRetry { .. } => Err(AdmissionError::PreemptRequired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_ungated_without_hra() {
        // No global HRA registered in unit tests → ungated pass-through, releasable.
        let mut lease = admit(ResourceClass::Light, "corr-test").expect("ungated admit");
        lease.release();
        lease.release(); // idempotent
    }
}
