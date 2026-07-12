//! Provider conformance harness — the Provider SDK's validation surface.
//!
//! [`run_conformance`] checks that ANY [`CapabilityProvider`] adapter honors the
//! protocol contract, independent of the provider's internals. It is the tool a
//! provider author uses to prove a new adapter plugs into KRIA correctly — and
//! the guard that keeps the boundary honest (a provider that violates the
//! contract fails here, not in production).
//!
//! It validates the **contract shape** (negotiate / describe / lifecycle-gating
//! / health / descriptor validity), not live side-effecting execution — real
//! execution is validated per-provider by the Docker/real integration tests.
//! This keeps conformance runnable for any provider with no side effects.

use crate::capability::descriptor::CapabilityDescriptor;
use crate::capability::protocol::ClientCapabilities;
use crate::capability::provider::{AcquireRequest, CapabilityProvider, RequestContext};

/// One conformance check result.
#[derive(Debug, Clone)]
pub struct ConformanceCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// The full conformance report for a provider.
#[derive(Debug, Clone)]
pub struct ConformanceReport {
    pub provider_id: String,
    pub checks: Vec<ConformanceCheck>,
    /// Descriptors the provider described (for downstream inspection).
    pub descriptor_count: usize,
}

impl ConformanceReport {
    /// Whether every check passed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// The names of failed checks.
    pub fn failures(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.name)
            .collect()
    }
}

fn check(
    checks: &mut Vec<ConformanceCheck>,
    name: &'static str,
    passed: bool,
    detail: impl Into<String>,
) {
    checks.push(ConformanceCheck {
        name,
        passed,
        detail: detail.into(),
    });
}

/// Run the provider conformance suite. Side-effect-free (no live execution).
pub async fn run_conformance(provider: &dyn CapabilityProvider) -> ConformanceReport {
    let mut checks = Vec::new();
    let provider_id = provider.provider_id().clone();

    // 1. Non-empty, stable provider id.
    check(
        &mut checks,
        "provider_id_non_empty",
        !provider_id.trim().is_empty(),
        format!("provider_id = '{provider_id}'"),
    );

    // 2. Negotiation yields a session with all mandatory facets.
    let client = ClientCapabilities::default();
    let session = match provider.negotiate(&client).await {
        Ok(s) => Some(s),
        Err(e) => {
            check(
                &mut checks,
                "negotiate_ok",
                false,
                format!("negotiate failed: {e}"),
            );
            None
        }
    };
    if let Some(session) = &session {
        check(
            &mut checks,
            "negotiate_mandatory_facets",
            session.has_mandatory(),
            format!("agreed features present; version {:?}", session.version),
        );
        check(
            &mut checks,
            "negotiate_provider_id_matches",
            session.provider_id == provider_id,
            format!("session.provider_id = '{}'", session.provider_id),
        );
    }

    // 3. describe → all descriptors valid + carry this provider id.
    let mut descriptor_count = 0;
    if let Some(session) = &session {
        match provider.describe(session).await {
            Ok(descs) => {
                descriptor_count = descs.len();
                let all_valid = descs.iter().all(|d| d.validate().is_ok());
                check(
                    &mut checks,
                    "descriptors_valid",
                    all_valid,
                    format!("{} descriptors, all valid = {all_valid}", descs.len()),
                );
                let all_owned = descs
                    .iter()
                    .all(|d: &CapabilityDescriptor| d.provider_id == provider_id);
                check(
                    &mut checks,
                    "descriptors_owned_by_provider",
                    all_owned,
                    "every descriptor.provider_id matches the provider",
                );
            }
            Err(e) => check(
                &mut checks,
                "describe_ok",
                false,
                format!("describe failed: {e}"),
            ),
        }

        // 4. Lifecycle gating: if the provider did NOT negotiate LIFECYCLE, its
        //    acquire() must honestly report Unsupported (never a fake install).
        if !session.supports_lifecycle() {
            let acq = provider
                .acquire(&AcquireRequest {
                    capability_tag: "conformance.probe".to_string(),
                    hint: None,
                    capability_id: None,
                    proposed_graph: None,
                    context: RequestContext::new(),
                })
                .await;
            let gated = matches!(&acq, Err(e) if e.is_unsupported());
            check(
                &mut checks,
                "lifecycle_gated_when_unadvertised",
                gated,
                "acquire() returns Unsupported when LIFECYCLE not negotiated",
            );
        }
    }

    // 5. health() responds.
    let _health = provider.health().await;
    check(&mut checks, "health_responds", true, "health() returned");

    ConformanceReport {
        provider_id,
        checks,
        descriptor_count,
    }
}
