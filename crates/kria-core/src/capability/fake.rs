//! Deny-live fake [`CapabilityProvider`] (OSC-033), Task 0.4.
//!
//! Compiled only under `os-control-test`. It serves a fixed descriptor list and
//! answers `execute` from memory, so capability-platform tests exercise the real
//! registry/negotiation/index paths without any provider process, container,
//! MCP server, or network call.

use async_trait::async_trait;
use std::sync::Mutex;

use super::descriptor::CapabilityDescriptor;
use super::error::CapError;
use super::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProviderHealth,
};
use super::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest};
use super::ProviderId;

/// A scripted, in-memory capability provider.
pub struct FakeProvider {
    provider_id: ProviderId,
    capabilities: Vec<CapabilityDescriptor>,
    catalog: Vec<CapabilityDescriptor>,
    health: ProviderHealth,
    executed: Mutex<Vec<String>>,
}

impl FakeProvider {
    /// A fake provider advertising `capabilities` under `provider_id`.
    #[must_use]
    pub fn new(provider_id: impl Into<ProviderId>, capabilities: Vec<CapabilityDescriptor>) -> Self {
        Self {
            provider_id: provider_id.into(),
            capabilities,
            catalog: Vec::new(),
            health: ProviderHealth::Ready,
            executed: Mutex::new(Vec::new()),
        }
    }

    /// Builder: advertise installable-but-not-installed catalog entries.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Vec<CapabilityDescriptor>) -> Self {
        self.catalog = catalog;
        self
    }

    /// Builder: report a non-ready health state.
    #[must_use]
    pub fn with_health(mut self, health: ProviderHealth) -> Self {
        self.health = health;
        self
    }

    /// The capability ids executed against this provider, in order.
    #[must_use]
    pub fn executed(&self) -> Vec<String> {
        self.executed.lock().expect("executed mutex").clone()
    }
}

#[async_trait]
impl CapabilityProvider for FakeProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        Ok(ProtocolSession {
            provider_id: self.provider_id.clone(),
            version: client.version,
            // A thin provider advertises the mandatory facets only.
            features: FeatureSet::mandatory(),
            extensions: serde_json::Map::new(),
        })
    }

    async fn describe(
        &self,
        _session: &ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(self.capabilities.clone())
    }

    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(self.catalog.clone())
    }

    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        self.executed
            .lock()
            .expect("executed mutex")
            .push(req.capability_id.clone());
        // Honest: decline an id this fake never advertised rather than fabricate
        // a success.
        if !self
            .capabilities
            .iter()
            .any(|c| c.capability_id == req.capability_id)
        {
            return Ok(CapabilityOutcome::Declined {
                reason: format!("fake provider does not advertise '{}'", req.capability_id),
            });
        }
        // Echo the request back: `capability` names the executed id and `echo`
        // returns the args verbatim, so a test can assert the platform threaded
        // an upstream step's output into the next step's `_upstream` argument.
        Ok(CapabilityOutcome::Value(serde_json::json!({
            "provider": self.provider_id,
            "capability": req.capability_id,
            "echo": req.args,
        })))
    }

    async fn health(&self) -> ProviderHealth {
        self.health
    }
}
