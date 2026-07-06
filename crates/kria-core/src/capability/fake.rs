//! [`FakeProvider`] — an in-memory [`CapabilityProvider`] test double.
//!
//! Used by CPP unit tests (and, later, by higher-milestone tests) to exercise
//! the boundary without a real provider, Docker, or network. It is configurable:
//! a caller sets its id, the features it advertises, the descriptors it exposes,
//! and how `execute` responds.
//!
//! Test-only: gated behind `#[cfg(test)]` so it is never compiled into a
//! production build (keeping the "no dead code in prod" policy).

use async_trait::async_trait;

use super::descriptor::CapabilityDescriptor;
use super::error::CapError;
use super::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use super::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest};
use super::ProviderId;

/// A configurable in-memory provider for tests.
pub struct FakeProvider {
    id: ProviderId,
    version: ProtocolVersion,
    features: FeatureSet,
    extensions: serde_json::Map<String, serde_json::Value>,
    descriptors: Vec<CapabilityDescriptor>,
    catalog: Vec<CapabilityDescriptor>,
    health: ProviderHealth,
}

impl FakeProvider {
    /// A provider advertising only the mandatory facets, exposing `descriptors`.
    pub fn new(id: impl Into<ProviderId>, descriptors: Vec<CapabilityDescriptor>) -> Self {
        Self {
            id: id.into(),
            version: ProtocolVersion::CURRENT,
            features: FeatureSet::mandatory(),
            extensions: serde_json::Map::new(),
            descriptors,
            catalog: Vec::new(),
            health: ProviderHealth::Ready,
        }
    }

    /// Set the installable marketplace catalog this provider advertises.
    pub fn with_catalog(mut self, catalog: Vec<CapabilityDescriptor>) -> Self {
        self.catalog = catalog;
        self
    }

    /// Override the advertised feature set (e.g. to add lifecycle/streaming).
    pub fn with_features(mut self, features: FeatureSet) -> Self {
        self.features = features;
        self
    }

    /// Attach forward-compat extension data the provider "advertises".
    pub fn with_extensions(
        mut self,
        extensions: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        self.extensions = extensions;
        self
    }

    /// Override advertised protocol version.
    pub fn with_version(mut self, version: ProtocolVersion) -> Self {
        self.version = version;
        self
    }

    /// Override reported health.
    pub fn with_health(mut self, health: ProviderHealth) -> Self {
        self.health = health;
        self
    }
}

#[async_trait]
impl CapabilityProvider for FakeProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        Ok(client.negotiate(
            self.id.clone(),
            self.version,
            self.features,
            self.extensions.clone(),
        ))
    }

    async fn describe(
        &self,
        _session: &ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(self.descriptors.clone())
    }

    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(self.catalog.clone())
    }

    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        // Echo the args back so tests can assert the request round-tripped.
        if self
            .descriptors
            .iter()
            .any(|d| d.capability_id == req.capability_id)
        {
            Ok(CapabilityOutcome::Value(serde_json::json!({
                "provider": self.id,
                "capability": req.capability_id,
                "echo": req.args,
            })))
        } else {
            Ok(CapabilityOutcome::Declined {
                reason: format!("no such capability '{}'", req.capability_id),
            })
        }
    }

    async fn health(&self) -> ProviderHealth {
        self.health
    }
}
