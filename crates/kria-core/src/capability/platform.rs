//! [`CapabilityPlatform`] — the composition root that ties the CPP pieces
//! together behind one provider-neutral surface.
//!
//! This is what the agent-loop tool entry (the flag-ON branch) will call instead
//! of reaching into any provider: it discovers capabilities across all registered
//! providers via the [`ProviderRegistry`]'s federated index, and executes a chosen
//! capability through the owning provider's adapter. It holds no provider-native
//! type and hardcodes no provider.
//!
//! # Scope (Milestone 3)
//!
//! Discovery + direct execution. Multi-capability planning (composing several
//! descriptors into an `ExecutionGraph`), permission gating, and acquisition are
//! layered on in later milestones — each as a method on this same facade, so the
//! Brain's entry point never changes shape as capability grows.

use std::sync::Arc;

use super::error::CapError;
use super::events::{CapabilityEvent, Outcome, SharedEventBus, Stage};
use super::index::ScoredDescriptor;
use super::provider::{CapabilityOutcome, CapabilityRequest};
use super::registry::{ProviderRegistry, RefreshReport};

/// The single provider-neutral surface the Brain uses for capabilities.
pub struct CapabilityPlatform {
    registry: Arc<ProviderRegistry>,
    events: Option<SharedEventBus>,
}

impl CapabilityPlatform {
    /// Build over a provider registry (which owns the federated index).
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self {
            registry,
            events: None,
        }
    }

    /// Attach an observability event bus (M8): discovery/execution/failure stages
    /// are emitted for the live desktop timeline + tracing.
    pub fn with_events(mut self, events: SharedEventBus) -> Self {
        self.events = Some(events);
        self
    }

    fn emit(&self, ev: CapabilityEvent) {
        if let Some(bus) = &self.events {
            bus.emit(ev);
        }
    }

    /// Access the underlying provider registry (registration, sessions, health).
    pub fn registry(&self) -> &Arc<ProviderRegistry> {
        &self.registry
    }

    /// Negotiate + describe every provider and (re)build the federated index.
    pub async fn refresh(&self) -> RefreshReport {
        self.registry.refresh().await
    }

    /// Discover the top-k capabilities across all providers for a goal query.
    pub fn discover(&self, query: &str, k: usize) -> Result<Vec<ScoredDescriptor>, CapError> {
        self.registry.search(query, k)
    }

    /// Look up a single indexed descriptor by its `(provider_id, capability_id)`
    /// key. Used by the permission/approval path and the Descriptor Viewer, which
    /// need the capability's full declared effects/guidance without re-running a
    /// goal query. Returns `None` when the capability is not currently federated
    /// (e.g. its provider is offline or the id is unknown).
    pub fn descriptor(
        &self,
        provider_id: &str,
        capability_id: &str,
    ) -> Result<Option<super::descriptor::CapabilityDescriptor>, CapError> {
        // The in-memory index has no direct get-by-key; an empty-query scan
        // returns every indexed descriptor (score 0), which we filter. Cheap at
        // current scale and avoids widening the FederatedIndex trait.
        let all = self.registry.search("", 1_000_000)?;
        Ok(all
            .into_iter()
            .map(|s| s.descriptor)
            .find(|d| d.provider_id == provider_id && d.capability_id == capability_id))
    }

    /// Recommend installable capabilities (marketplace catalog federation) for a
    /// goal: gather each provider's catalog (not-yet-installed entries), rank them
    /// against the query, and return the top-k. Pure read — installs nothing (the
    /// caller drives `acquire` on an approved choice).
    pub async fn recommend(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<ScoredDescriptor>, CapError> {
        let mut catalog = Vec::new();
        for pid in self.registry.provider_ids() {
            if let Some(p) = self.registry.get(&pid) {
                match p.catalog().await {
                    Ok(mut c) => catalog.append(&mut c),
                    Err(e) => tracing::debug!("provider {pid} catalog unavailable: {e}"),
                }
            }
        }
        self.registry.index().score_descriptors(query, catalog, k)
    }

    /// Execute a specific capability through its owning provider's adapter.
    ///
    /// The Brain resolves *which* capability to run (via [`discover`] + ranking,
    /// and later permission + planning); this routes the validated request to the
    /// right provider without the Brain touching any provider runtime.
    ///
    /// [`discover`]: CapabilityPlatform::discover
    pub async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        let correlation_id = req.context.correlation_id.clone();
        let provider_id = req.provider_id.clone();
        let capability_id = req.capability_id.clone();

        let provider = match self.registry.get(&provider_id) {
            Some(p) => p,
            None => {
                self.emit(CapabilityEvent::new(
                    &correlation_id,
                    &provider_id,
                    Some(capability_id.clone()),
                    Stage::Execute,
                    Outcome::Failed,
                    "no such provider",
                ));
                return Err(CapError::Execute(format!(
                    "no such provider '{provider_id}'"
                )));
            }
        };

        self.emit(CapabilityEvent::new(
            &correlation_id,
            &provider_id,
            Some(capability_id.clone()),
            Stage::Execute,
            Outcome::Started,
            "executing capability",
        ));

        let result = provider.execute(req).await;

        // Feed the circuit breaker + emit the terminal event.
        let (ok, outcome, detail) = match &result {
            Ok(CapabilityOutcome::Declined { reason }) => (true, Outcome::Declined, reason.clone()),
            Ok(_) => (true, Outcome::Ok, "ok".to_string()),
            Err(e) => (false, Outcome::Failed, e.to_string()),
        };
        self.registry
            .record_execution_outcome(&provider_id, &capability_id, ok);
        self.emit(CapabilityEvent::new(
            &correlation_id,
            &provider_id,
            Some(capability_id),
            Stage::Execute,
            outcome,
            detail,
        ));

        result
    }
}
