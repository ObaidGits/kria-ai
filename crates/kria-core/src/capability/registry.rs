//! The [`ProviderRegistry`] — the set of registered capability providers and the
//! single [`FederatedIndex`] built from their descriptors.
//!
//! This is the Brain's entry point for cross-provider discovery: providers are
//! registered by open-vocabulary `provider_id`, [`refresh`](ProviderRegistry::refresh)
//! negotiates + describes every provider and rebuilds the federated index, and
//! [`search`](ProviderRegistry::search) retrieves the top capabilities for a goal
//! across all of them.
//!
//! Federation with a single source of truth per provider: the registry never
//! stores an authoritative catalog. Each provider owns its own catalog
//! (`describe()`); the federated index is a **derived, rebuildable** view. A full
//! rebuild from `describe()` yields identical results (idempotent), so the
//! in-memory index needs no persistence for correctness. (Durable descriptor/
//! session caching is added with the Milestone-8 caching layer for cold-start
//! speed + observability; it is an optimization over this rebuildable view, not a
//! second source of truth.)

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use std::time::{Duration, Instant};

use super::error::CapError;
use super::index::{FederatedIndex, ScoredDescriptor};
use super::protocol::{ClientCapabilities, ProtocolSession, ProviderHealth};
use super::provider::CapabilityProvider;
use super::state::ProviderState;
use super::ProviderId;

/// Per-provider circuit breaker: after `threshold` consecutive execution
/// failures a provider's breaker opens for `cooldown`, excluding it from
/// discovery/execution so one failing provider cannot stall the platform (R17.5).
/// A single success (or cooldown elapse → half-open probe) closes it.
#[derive(Debug, Clone, Default)]
struct BreakerState {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

/// Per-provider result of a refresh: the negotiated session, health, descriptor
/// count, and derived provider state.
#[derive(Debug, Clone)]
pub struct ProviderRefresh {
    pub provider_id: ProviderId,
    pub health: ProviderHealth,
    pub state: ProviderState,
    pub descriptor_count: usize,
    /// Present when negotiation + describe succeeded.
    pub version: Option<super::protocol::ProtocolVersion>,
    /// Failure reason when the provider could not be refreshed (honest degrade).
    pub error: Option<String>,
}

/// Summary of a whole-registry refresh.
#[derive(Debug, Clone, Default)]
pub struct RefreshReport {
    pub providers: Vec<ProviderRefresh>,
    pub total_descriptors: usize,
}

impl RefreshReport {
    /// Providers that are serving discovery after the refresh.
    pub fn healthy_count(&self) -> usize {
        self.providers.iter().filter(|p| p.error.is_none()).count()
    }
}

/// Holds registered providers and the derived federated index.
pub struct ProviderRegistry {
    providers: RwLock<HashMap<ProviderId, Arc<dyn CapabilityProvider>>>,
    /// Last negotiated session per provider (observability + facet gating).
    sessions: RwLock<HashMap<ProviderId, ProtocolSession>>,
    /// Per-provider circuit breakers (execution-failure resilience).
    breakers: RwLock<HashMap<ProviderId, BreakerState>>,
    index: Arc<dyn FederatedIndex>,
    client: ClientCapabilities,
    breaker_threshold: u32,
    breaker_cooldown: Duration,
}

impl ProviderRegistry {
    /// Create a registry over the given federated index.
    pub fn new(index: Arc<dyn FederatedIndex>) -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            breakers: RwLock::new(HashMap::new()),
            index,
            client: ClientCapabilities::default(),
            breaker_threshold: 3,
            breaker_cooldown: Duration::from_secs(30),
        }
    }

    /// Record an execution outcome for a `(provider, capability)`, updating both
    /// the provider circuit breaker (resilience) and the federated index's
    /// learned success stats (M6 learning loop → ranking).
    pub fn record_execution_outcome(&self, provider_id: &str, capability_id: &str, ok: bool) {
        self.index.record_outcome(provider_id, capability_id, ok);
        if let Ok(mut guard) = self.breakers.write() {
            let state = guard.entry(provider_id.to_string()).or_default();
            if ok {
                state.consecutive_failures = 0;
                state.open_until = None;
            } else {
                state.consecutive_failures += 1;
                if state.consecutive_failures >= self.breaker_threshold {
                    state.open_until = Some(Instant::now() + self.breaker_cooldown);
                }
            }
        }
    }

    /// Whether a provider's circuit breaker is currently open (excluded from use).
    /// Transitions to half-open (closed for a probe) once the cooldown elapses.
    pub fn is_breaker_open(&self, provider_id: &str) -> bool {
        if let Ok(mut guard) = self.breakers.write() {
            if let Some(state) = guard.get_mut(provider_id) {
                if let Some(open_until) = state.open_until {
                    if Instant::now() >= open_until {
                        // Half-open: allow a probe; reset the timer window.
                        state.open_until = None;
                        state.consecutive_failures = 0;
                        return false;
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Register (or replace) a provider by its id.
    pub fn register(&self, provider: Arc<dyn CapabilityProvider>) {
        let id = provider.provider_id().clone();
        if let Ok(mut guard) = self.providers.write() {
            guard.insert(id, provider);
        }
    }

    /// Look up a registered provider by id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn CapabilityProvider>> {
        self.providers.read().ok()?.get(id).cloned()
    }

    /// All registered provider ids.
    pub fn provider_ids(&self) -> Vec<ProviderId> {
        self.providers
            .read()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The negotiated session for a provider, if it has been refreshed.
    pub fn session(&self, id: &str) -> Option<ProtocolSession> {
        self.sessions.read().ok()?.get(id).cloned()
    }

    /// The federated index (for direct search or wiring into the CIL).
    pub fn index(&self) -> Arc<dyn FederatedIndex> {
        Arc::clone(&self.index)
    }

    /// Negotiate + describe every registered provider and rebuild the index.
    ///
    /// A provider that fails to negotiate/describe is recorded as errored in the
    /// report and excluded from the index (honest degrade) — one bad provider
    /// never fails the whole refresh (design R2.5 / circuit-breaker seed).
    pub async fn refresh(&self) -> RefreshReport {
        let providers: Vec<Arc<dyn CapabilityProvider>> = self
            .providers
            .read()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default();

        let mut report = RefreshReport::default();
        let mut all_descriptors = Vec::new();

        for provider in providers {
            let pid = provider.provider_id().clone();
            // Circuit breaker: skip providers whose breaker is open (excluded
            // from discovery until the cooldown elapses).
            if self.is_breaker_open(&pid) {
                report.providers.push(ProviderRefresh {
                    provider_id: pid,
                    health: ProviderHealth::Offline,
                    state: ProviderState::Disconnected,
                    descriptor_count: 0,
                    version: None,
                    error: Some("circuit breaker open (excluded)".to_string()),
                });
                continue;
            }
            match provider.negotiate(&self.client).await {
                Ok(session) => {
                    let health = provider.health().await;
                    match provider.describe(&session).await {
                        Ok(descs) => {
                            let count = descs.len();
                            all_descriptors.extend(descs);
                            if let Ok(mut s) = self.sessions.write() {
                                s.insert(pid.clone(), session.clone());
                            }
                            report.providers.push(ProviderRefresh {
                                provider_id: pid,
                                health,
                                state: ProviderState::Healthy,
                                descriptor_count: count,
                                version: Some(session.version),
                                error: None,
                            });
                        }
                        Err(e) => report.providers.push(ProviderRefresh {
                            provider_id: pid,
                            health: ProviderHealth::Degraded,
                            state: ProviderState::Degraded,
                            descriptor_count: 0,
                            version: Some(session.version),
                            error: Some(format!("describe failed: {e}")),
                        }),
                    }
                }
                Err(e) => report.providers.push(ProviderRefresh {
                    provider_id: pid,
                    health: ProviderHealth::Offline,
                    state: ProviderState::Disconnected,
                    descriptor_count: 0,
                    version: None,
                    error: Some(format!("negotiation failed: {e}")),
                }),
            }
        }

        report.total_descriptors = all_descriptors.len();
        if let Err(e) = self.index.rebuild(&all_descriptors) {
            tracing::warn!("federated index rebuild failed: {e}");
        }
        report
    }

    /// Retrieve the top-k capabilities across all providers for a goal query.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<ScoredDescriptor>, CapError> {
        self.index.search(query, k)
    }
}
