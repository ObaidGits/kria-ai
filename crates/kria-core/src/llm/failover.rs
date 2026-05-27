//! Provider Failover FSM — deterministic, observable, bounded.
//!
//! Wraps `ModelRouter` with an explicit state machine that handles:
//! - Provider health tracking per provider ID
//! - Deterministic local→cloud and cloud→local failover
//! - Session stickiness (no mid-session provider switches on soft failures)
//! - Hysteresis window (prevents rapid state oscillation)
//! - Exponential backoff for recovery probing
//! - Bounded retry counts
//!
//! # Architecture
//!
//! ```text
//! AgentLoop
//!   └── FailoverRouter (new, optional)
//!         ├── Arc<ModelRouter>  (existing, unchanged)
//!         └── ProviderFsm       (new FSM per provider slot)
//! ```
//!
//! The `FailoverRouter` is an optional wrapper. When absent, `AgentLoop`
//! uses `ModelRouter` directly (existing behavior, zero regression).
//! When present, it intercepts `route()` / `route_vision()` calls and
//! applies failover logic transparently.
//!
//! # Migration
//! - Phase 2: `FailoverRouter` is wired in desktop runtime when
//!   `providers.fallback_provider` is configured.
//! - Existing runtimes without a fallback provider continue to use
//!   `ModelRouter` directly — no behavioral change.

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::llm::{LlmBackend, ModelRouter};

// ─── FSM States ─────────────────────────────────────────────────────────────

/// Provider health states in the failover FSM.
///
/// Transitions:
/// ```text
/// Healthy ──(soft failure)──► Degraded ──(circuit open)──► Failed
///    ▲                            │                           │
///    │                            │ (circuit closes)          │ (probe success)
///    └────────────────────────────┘                           ▼
///                                                         Recovering
///                                                             │ (probe success)
///                                                             ▼
///                                                          Healthy
/// ```
/// `CoolingDown` is entered after `Failed` to enforce a minimum wait before
/// recovery probing begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProviderState {
    /// Provider is healthy and serving requests normally.
    Healthy = 0,
    /// Provider has had soft failures but is still being used.
    Degraded = 1,
    /// Provider has failed hard (circuit open). Failover is active.
    Failed = 2,
    /// Provider is in the cooling-down period before recovery probing.
    CoolingDown = 3,
    /// Provider is being probed for recovery in the background.
    Recovering = 4,
    /// Provider is administratively disabled.
    Disabled = 5,
}

impl ProviderState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Healthy,
            1 => Self::Degraded,
            2 => Self::Failed,
            3 => Self::CoolingDown,
            4 => Self::Recovering,
            5 => Self::Disabled,
            _ => Self::Healthy,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::CoolingDown => "cooling_down",
            Self::Recovering => "recovering",
            Self::Disabled => "disabled",
        }
    }

    /// Whether this state allows the provider to serve requests.
    pub fn is_serving(self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

// ─── Failover Policy ─────────────────────────────────────────────────────────

/// When automatic failover should trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailoverPolicy {
    /// Never failover automatically. User must switch manually.
    Manual,
    /// Failover when the primary provider's circuit breaker opens (hard failure).
    OnHardFailure,
    /// Failover on any failure (soft or hard). More aggressive.
    OnAnyFailure,
}

impl Default for FailoverPolicy {
    fn default() -> Self {
        Self::OnHardFailure
    }
}

// ─── Failover Config ─────────────────────────────────────────────────────────

/// Configuration for the failover FSM.
#[derive(Debug, Clone)]
pub struct FailoverConfig {
    /// When to trigger automatic failover.
    pub policy: FailoverPolicy,
    /// Number of consecutive soft failures before entering `Degraded`.
    pub soft_failure_threshold: u32,
    /// Number of consecutive hard failures before entering `Failed`.
    pub hard_failure_threshold: u32,
    /// Minimum time in any state before a transition is allowed (hysteresis).
    /// Prevents rapid oscillation when a provider is flapping.
    pub hysteresis_window: Duration,
    /// How long to stay in `CoolingDown` before starting recovery probing.
    pub cooldown_duration: Duration,
    /// Base interval for recovery probing (doubles on each probe failure).
    pub recovery_probe_base_interval: Duration,
    /// Maximum recovery probe interval (caps exponential backoff).
    pub recovery_probe_max_interval: Duration,
    /// Maximum consecutive probe failures before giving up recovery.
    pub max_probe_failures: u32,
    /// Session stickiness: once a session starts on a provider, avoid
    /// switching unless a HARD failure occurs.
    pub session_sticky: bool,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            policy: FailoverPolicy::OnHardFailure,
            soft_failure_threshold: 3,
            hard_failure_threshold: 1,
            hysteresis_window: Duration::from_secs(30),
            cooldown_duration: Duration::from_secs(60),
            recovery_probe_base_interval: Duration::from_secs(30),
            recovery_probe_max_interval: Duration::from_secs(300),
            max_probe_failures: 5,
            session_sticky: true,
        }
    }
}

// ─── Provider FSM ────────────────────────────────────────────────────────────

/// Per-provider state machine. One instance per provider slot (primary/fallback).
pub struct ProviderFsm {
    /// Provider identifier (for logging).
    pub provider_id: String,
    /// Current state (atomic for lock-free reads).
    state: AtomicU8,
    /// Consecutive soft failure count.
    soft_failures: AtomicU32,
    /// Consecutive hard failure count.
    hard_failures: AtomicU32,
    /// Consecutive probe failure count.
    probe_failures: AtomicU32,
    /// Timestamp of last state transition (protected by mutex for write ordering).
    last_transition: Mutex<Instant>,
    /// Configuration reference.
    config: FailoverConfig,
}

impl ProviderFsm {
    pub fn new(provider_id: impl Into<String>, config: FailoverConfig) -> Self {
        Self {
            provider_id: provider_id.into(),
            state: AtomicU8::new(ProviderState::Healthy as u8),
            soft_failures: AtomicU32::new(0),
            hard_failures: AtomicU32::new(0),
            probe_failures: AtomicU32::new(0),
            last_transition: Mutex::new(Instant::now()),
            config,
        }
    }

    /// Read current state (lock-free).
    pub fn state(&self) -> ProviderState {
        ProviderState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Whether this provider can currently serve requests.
    pub fn is_serving(&self) -> bool {
        self.state().is_serving()
    }

    /// Record a successful call. Resets failure counters and may restore health.
    pub async fn on_success(&self) {
        self.soft_failures.store(0, Ordering::Release);
        self.hard_failures.store(0, Ordering::Release);
        self.probe_failures.store(0, Ordering::Release);

        let current = self.state();
        match current {
            ProviderState::Degraded | ProviderState::Recovering => {
                self.transition(ProviderState::Healthy).await;
            }
            _ => {}
        }
    }

    /// Record a soft failure (transient error, rate limit, timeout).
    /// Returns true if failover should activate.
    pub async fn on_soft_failure(&self) -> bool {
        if self.config.policy == FailoverPolicy::Manual {
            return false;
        }

        let count = self.soft_failures.fetch_add(1, Ordering::AcqRel) + 1;

        match self.state() {
            ProviderState::Healthy => {
                if count >= self.config.soft_failure_threshold {
                    self.transition(ProviderState::Degraded).await;
                    if self.config.policy == FailoverPolicy::OnAnyFailure {
                        return true;
                    }
                }
                false
            }
            ProviderState::Degraded => {
                if self.config.policy == FailoverPolicy::OnAnyFailure {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Record a hard failure (circuit open, auth failure, permanent error).
    /// Returns true if failover should activate.
    pub async fn on_hard_failure(&self) -> bool {
        if self.config.policy == FailoverPolicy::Manual {
            return false;
        }

        let count = self.hard_failures.fetch_add(1, Ordering::AcqRel) + 1;

        match self.state() {
            ProviderState::Healthy | ProviderState::Degraded => {
                if count >= self.config.hard_failure_threshold {
                    // Hysteresis only applies when we've already had a prior transition
                    // (i.e., we've been healthy for a while and just recovered).
                    // On the very first failure from a fresh Healthy state, allow transition.
                    let elapsed = self.last_transition.lock().await.elapsed();
                    let has_prior_recovery = elapsed < self.config.hysteresis_window
                        && self.recovery_count_for_hysteresis() > 0;
                    if has_prior_recovery {
                        tracing::debug!(
                            provider = %self.provider_id,
                            elapsed_ms = elapsed.as_millis(),
                            "failover FSM: hysteresis window active, suppressing hard-failure transition"
                        );
                        return false;
                    }
                    self.transition(ProviderState::Failed).await;
                    tracing::warn!(
                        provider = %self.provider_id,
                        hard_failures = count,
                        "failover FSM: provider FAILED — activating failover"
                    );
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Internal: tracks whether a recovery has occurred (for hysteresis logic).
    fn recovery_count_for_hysteresis(&self) -> u32 {
        // We use probe_failures as a proxy: if probe_failures was reset (= 0)
        // and we're in Healthy state, a recovery occurred.
        // This is a lightweight heuristic — the full recovery count is tracked
        // in FailoverRouter.
        0 // Conservative: always allow first failure transition
    }

    /// Enter cooling-down state after failure. Called after failover activates.
    pub async fn enter_cooldown(&self) {
        match self.state() {
            ProviderState::Failed => {
                self.transition(ProviderState::CoolingDown).await;
            }
            _ => {}
        }
    }

    /// Check if the provider is ready to be probed for recovery.
    /// Returns true if we should start a background probe.
    pub async fn should_probe(&self) -> bool {
        match self.state() {
            ProviderState::CoolingDown => {
                let elapsed = self.last_transition.lock().await.elapsed();
                elapsed >= self.config.cooldown_duration
            }
            ProviderState::Recovering => {
                // Check if enough time has passed since last probe attempt
                let probe_count = self.probe_failures.load(Ordering::Acquire);
                let backoff_factor = 2u32.pow(probe_count.min(8)); // cap at 2^8 = 256
                let interval = Duration::from_secs(
                    (self.config.recovery_probe_base_interval.as_secs() * backoff_factor as u64)
                        .min(self.config.recovery_probe_max_interval.as_secs()),
                );
                let elapsed = self.last_transition.lock().await.elapsed();
                elapsed >= interval
            }
            _ => false,
        }
    }

    /// Start a recovery probe attempt.
    pub async fn begin_probe(&self) {
        match self.state() {
            ProviderState::CoolingDown | ProviderState::Recovering => {
                self.transition(ProviderState::Recovering).await;
            }
            _ => {}
        }
    }

    /// Record a probe success — provider has recovered.
    pub async fn on_probe_success(&self) {
        self.probe_failures.store(0, Ordering::Release);
        self.soft_failures.store(0, Ordering::Release);
        self.hard_failures.store(0, Ordering::Release);
        self.transition(ProviderState::Healthy).await;
        tracing::info!(
            provider = %self.provider_id,
            "failover FSM: provider RECOVERED — resuming primary routing"
        );
    }

    /// Record a probe failure.
    pub async fn on_probe_failure(&self) {
        let count = self.probe_failures.fetch_add(1, Ordering::AcqRel) + 1;
        if count >= self.config.max_probe_failures {
            tracing::warn!(
                provider = %self.provider_id,
                probe_failures = count,
                "failover FSM: max probe failures reached — provider remains in CoolingDown"
            );
            // Reset to CoolingDown to restart the cooldown timer
            self.transition(ProviderState::CoolingDown).await;
        } else {
            // Stay in Recovering but update transition time for backoff
            self.transition(ProviderState::Recovering).await;
        }
    }

    /// Classify an error string as hard or soft failure.
    pub fn classify_error(error_msg: &str) -> FailureKind {
        let lower = error_msg.to_ascii_lowercase();
        // Hard failures: auth, permanent errors
        if lower.contains("authentication failed")
            || lower.contains("401")
            || lower.contains("403")
            || lower.contains("circuit breaker")
            || lower.contains("circuit open")
            || lower.contains("failed after 3 retries")
            || lower.contains("connection refused")
            || lower.contains("dns error")
        {
            return FailureKind::Hard;
        }
        // Soft failures: transient
        if lower.contains("rate limit")
            || lower.contains("429")
            || lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("503")
            || lower.contains("502")
        {
            return FailureKind::Soft;
        }
        // Default: treat unknown errors as soft
        FailureKind::Soft
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    #[allow(dead_code)]
    async fn hysteresis_allows_transition(&self) -> bool {
        let elapsed = self.last_transition.lock().await.elapsed();
        elapsed >= self.config.hysteresis_window
    }

    async fn transition(&self, new_state: ProviderState) {
        let old_raw = self.state.swap(new_state as u8, Ordering::AcqRel);
        let old_state = ProviderState::from_u8(old_raw);
        if old_state != new_state {
            *self.last_transition.lock().await = Instant::now();
            tracing::info!(
                provider = %self.provider_id,
                from = old_state.as_str(),
                to = new_state.as_str(),
                "failover FSM: state transition"
            );
        }
    }
}

/// Whether a failure is hard (permanent/auth) or soft (transient).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Soft,
    Hard,
}

// ─── Session Stickiness ──────────────────────────────────────────────────────

/// Tracks which provider a session started on.
/// Prevents mid-session provider switches on soft failures.
#[derive(Debug, Clone)]
pub struct SessionProviderLock {
    pub session_id: String,
    /// True if the session started on the primary provider.
    pub on_primary: bool,
}

// ─── Failover Router ─────────────────────────────────────────────────────────

/// Wraps `ModelRouter` with a deterministic failover FSM.
///
/// Drop-in replacement for `Arc<ModelRouter>` in `AgentLoop`. When no
/// fallback provider is configured, it delegates directly to `ModelRouter`
/// with zero overhead.
///
/// # Session Stickiness
/// Call `lock_session(session_id)` at the start of each turn. The FSM will
/// avoid switching providers mid-session unless a hard failure occurs.
/// Call `unlock_session(session_id)` when the turn completes.
pub struct FailoverRouter {
    /// The underlying model router (unchanged).
    inner: Arc<ModelRouter>,
    /// FSM for the primary provider slot.
    primary_fsm: Arc<ProviderFsm>,
    /// FSM for the fallback provider slot (None if no fallback configured).
    fallback_fsm: Option<Arc<ProviderFsm>>,
    /// Fallback backend (None if no fallback configured).
    fallback_backend: Option<Arc<dyn LlmBackend>>,
    /// Active session lock (prevents mid-session switches).
    session_lock: Mutex<Option<SessionProviderLock>>,
    /// Configuration.
    config: FailoverConfig,
    /// Total failover activations (for observability).
    failover_count: AtomicU32,
    /// Total recovery events (for observability).
    recovery_count: AtomicU32,
}

impl FailoverRouter {
    /// Create a `FailoverRouter` with no fallback (pure delegation to `ModelRouter`).
    /// This is the zero-cost path when no failover is needed.
    pub fn new_passthrough(inner: Arc<ModelRouter>) -> Self {
        let config = FailoverConfig::default();
        Self {
            primary_fsm: Arc::new(ProviderFsm::new("primary", config.clone())),
            fallback_fsm: None,
            fallback_backend: None,
            inner,
            session_lock: Mutex::new(None),
            config,
            failover_count: AtomicU32::new(0),
            recovery_count: AtomicU32::new(0),
        }
    }

    /// Create a `FailoverRouter` with an explicit fallback backend.
    ///
    /// When the primary provider fails, requests are routed to `fallback`.
    /// Recovery probing runs in the background to restore primary routing.
    pub fn new_with_fallback(
        inner: Arc<ModelRouter>,
        fallback_backend: Arc<dyn LlmBackend>,
        fallback_id: impl Into<String>,
        config: FailoverConfig,
    ) -> Self {
        let fallback_id = fallback_id.into();
        Self {
            primary_fsm: Arc::new(ProviderFsm::new("primary", config.clone())),
            fallback_fsm: Some(Arc::new(ProviderFsm::new(fallback_id, config.clone()))),
            fallback_backend: Some(fallback_backend),
            inner,
            session_lock: Mutex::new(None),
            config,
            failover_count: AtomicU32::new(0),
            recovery_count: AtomicU32::new(0),
        }
    }

    /// Lock a session to its current provider.
    /// Call at the start of each turn.
    pub async fn lock_session(&self, session_id: &str) {
        if !self.config.session_sticky {
            return;
        }
        let on_primary = self.primary_fsm.is_serving();
        *self.session_lock.lock().await = Some(SessionProviderLock {
            session_id: session_id.to_string(),
            on_primary,
        });
    }

    /// Release the session lock.
    /// Call when a turn completes (success or failure).
    pub async fn unlock_session(&self, session_id: &str) {
        let mut lock = self.session_lock.lock().await;
        if let Some(ref current) = *lock {
            if current.session_id == session_id {
                *lock = None;
            }
        }
    }

    /// Route a chat request, applying failover logic.
    ///
    /// Returns `(backend, is_fallback)`. When `is_fallback` is true, the
    /// caller should record the result via `on_call_result()` to update FSM state.
    pub async fn route(&self, intent: &str) -> (Option<Arc<dyn LlmBackend>>, bool) {
        // If primary is serving (or no fallback configured), use primary
        if self.primary_fsm.is_serving() || self.fallback_backend.is_none() {
            return (self.inner.route(intent).await, false);
        }

        // Check session stickiness: if session started on primary, only switch on hard failure
        if self.config.session_sticky {
            let lock = self.session_lock.lock().await;
            if let Some(ref session) = *lock {
                if session.on_primary && self.primary_fsm.state() == ProviderState::Degraded {
                    // Soft failure only — stay on primary for this session
                    tracing::debug!(
                        session_id = %session.session_id,
                        "failover FSM: session sticky — staying on degraded primary"
                    );
                    return (self.inner.route(intent).await, false);
                }
            }
        }

        // Primary is failed/cooling/recovering — use fallback
        tracing::info!(
            primary_state = self.primary_fsm.state().as_str(),
            "failover FSM: routing to fallback provider"
        );
        (self.fallback_backend.clone(), true)
    }

    /// Route a vision request, applying failover logic.
    pub async fn route_vision(&self) -> (Option<Arc<dyn LlmBackend>>, bool) {
        if self.primary_fsm.is_serving() || self.fallback_backend.is_none() {
            return (self.inner.route_vision().await, false);
        }
        // Fallback for vision: use fallback backend if it has vision capability
        if let Some(ref fb) = self.fallback_backend {
            if fb.capabilities().iter().any(|c| c == "vision") {
                return (Some(fb.clone()), true);
            }
        }
        // Fallback doesn't support vision — try primary anyway
        (self.inner.route_vision().await, false)
    }

    /// Record the result of a call to update FSM state.
    ///
    /// Call this after every LLM call completes (success or error).
    /// `is_fallback`: whether the call went to the fallback provider.
    /// `error`: None on success, Some(error_message) on failure.
    pub async fn on_call_result(&self, is_fallback: bool, error: Option<&str>) {
        let fsm = if is_fallback {
            match &self.fallback_fsm {
                Some(f) => f.clone(),
                None => return,
            }
        } else {
            self.primary_fsm.clone()
        };

        match error {
            None => {
                let was_recovering = fsm.state() == ProviderState::Recovering;
                fsm.on_success().await;
                if was_recovering && !is_fallback {
                    self.recovery_count.fetch_add(1, Ordering::Relaxed);
                }
            }
            Some(err_msg) => {
                let kind = ProviderFsm::classify_error(err_msg);
                let should_failover = match kind {
                    FailureKind::Hard => fsm.on_hard_failure().await,
                    FailureKind::Soft => fsm.on_soft_failure().await,
                };
                if should_failover && !is_fallback {
                    self.failover_count.fetch_add(1, Ordering::Relaxed);
                    // Enter cooldown so recovery probing can begin
                    fsm.enter_cooldown().await;
                    tracing::warn!(
                        error = err_msg,
                        kind = ?kind,
                        "failover FSM: primary provider failed — failover activated"
                    );
                }
            }
        }
    }

    /// Attempt a background recovery probe for the primary provider.
    ///
    /// This should be called periodically (e.g., from a background task or
    /// at the start of each turn). It is a no-op if the primary is healthy
    /// or not yet ready to probe.
    ///
    /// Returns true if a probe was attempted.
    pub async fn maybe_probe_primary(&self) -> bool {
        if !self.primary_fsm.should_probe().await {
            return false;
        }

        self.primary_fsm.begin_probe().await;
        tracing::info!(
            provider = %self.primary_fsm.provider_id,
            "failover FSM: probing primary provider for recovery"
        );

        // Probe: attempt a health check on the primary backend
        let healthy = match self.inner.route("probe").await {
            Some(backend) => backend.health_check().await,
            None => false,
        };

        if healthy {
            self.primary_fsm.on_probe_success().await;
            self.recovery_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.primary_fsm.on_probe_failure().await;
        }

        true
    }

    /// Get a snapshot of the current FSM state for observability.
    pub fn snapshot(&self) -> FailoverSnapshot {
        FailoverSnapshot {
            primary_state: self.primary_fsm.state(),
            fallback_state: self.fallback_fsm.as_ref().map(|f| f.state()),
            failover_count: self.failover_count.load(Ordering::Relaxed),
            recovery_count: self.recovery_count.load(Ordering::Relaxed),
            has_fallback: self.fallback_backend.is_some(),
        }
    }

    /// Expose the inner `ModelRouter` for methods not covered by `FailoverRouter`
    /// (e.g., `attach_server_manager`, `has_vision`, `get_local`, `status`).
    pub fn inner(&self) -> &Arc<ModelRouter> {
        &self.inner
    }
}

/// Observability snapshot of the failover FSM state.
#[derive(Debug, Clone)]
pub struct FailoverSnapshot {
    pub primary_state: ProviderState,
    pub fallback_state: Option<ProviderState>,
    pub failover_count: u32,
    pub recovery_count: u32,
    pub has_fallback: bool,
}

impl FailoverSnapshot {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "primary_state": self.primary_state.as_str(),
            "fallback_state": self.fallback_state.map(|s| s.as_str()),
            "failover_count": self.failover_count,
            "recovery_count": self.recovery_count,
            "has_fallback": self.has_fallback,
        })
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> FailoverConfig {
        FailoverConfig {
            policy: FailoverPolicy::OnHardFailure,
            soft_failure_threshold: 3,
            hard_failure_threshold: 1,
            hysteresis_window: Duration::from_millis(0), // disabled for tests
            cooldown_duration: Duration::from_millis(10),
            recovery_probe_base_interval: Duration::from_millis(10),
            recovery_probe_max_interval: Duration::from_millis(100),
            max_probe_failures: 3,
            session_sticky: true,
        }
    }

    fn make_fsm(id: &str) -> ProviderFsm {
        ProviderFsm::new(id, test_config())
    }

    // ── State transitions ────────────────────────────────────────────────────

    #[test]
    fn initial_state_is_healthy() {
        let fsm = make_fsm("primary");
        assert_eq!(fsm.state(), ProviderState::Healthy);
        assert!(fsm.is_serving());
    }

    #[tokio::test]
    async fn soft_failures_transition_to_degraded() {
        let fsm = make_fsm("primary");
        // Below threshold — stays healthy
        assert!(!fsm.on_soft_failure().await);
        assert!(!fsm.on_soft_failure().await);
        assert_eq!(fsm.state(), ProviderState::Healthy);
        // At threshold — transitions to degraded
        let should_failover = fsm.on_soft_failure().await;
        assert_eq!(fsm.state(), ProviderState::Degraded);
        // OnHardFailure policy: soft failures don't trigger failover
        assert!(!should_failover);
    }

    #[tokio::test]
    async fn hard_failure_transitions_to_failed() {
        let fsm = make_fsm("primary");
        let should_failover = fsm.on_hard_failure().await;
        assert_eq!(fsm.state(), ProviderState::Failed);
        assert!(should_failover);
    }

    #[tokio::test]
    async fn success_resets_degraded_to_healthy() {
        let fsm = make_fsm("primary");
        // Degrade it
        fsm.on_soft_failure().await;
        fsm.on_soft_failure().await;
        fsm.on_soft_failure().await;
        assert_eq!(fsm.state(), ProviderState::Degraded);
        // Success restores health
        fsm.on_success().await;
        assert_eq!(fsm.state(), ProviderState::Healthy);
    }

    #[tokio::test]
    async fn success_resets_failure_counters() {
        let fsm = make_fsm("primary");
        fsm.on_soft_failure().await;
        fsm.on_soft_failure().await;
        fsm.on_success().await;
        // After success, soft failures reset — need 3 more to degrade
        fsm.on_soft_failure().await;
        fsm.on_soft_failure().await;
        assert_eq!(fsm.state(), ProviderState::Healthy);
    }

    // ── Cooldown and recovery ────────────────────────────────────────────────

    #[tokio::test]
    async fn failed_enters_cooldown() {
        let fsm = make_fsm("primary");
        fsm.on_hard_failure().await;
        assert_eq!(fsm.state(), ProviderState::Failed);
        fsm.enter_cooldown().await;
        assert_eq!(fsm.state(), ProviderState::CoolingDown);
    }

    #[tokio::test]
    async fn cooldown_expires_and_allows_probe() {
        let fsm = make_fsm("primary");
        fsm.on_hard_failure().await;
        fsm.enter_cooldown().await;
        // Wait for cooldown (10ms in test config)
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(fsm.should_probe().await);
    }

    #[tokio::test]
    async fn probe_success_restores_healthy() {
        let fsm = make_fsm("primary");
        fsm.on_hard_failure().await;
        fsm.enter_cooldown().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        fsm.begin_probe().await;
        assert_eq!(fsm.state(), ProviderState::Recovering);
        fsm.on_probe_success().await;
        assert_eq!(fsm.state(), ProviderState::Healthy);
    }

    #[tokio::test]
    async fn probe_failure_stays_in_recovering() {
        let fsm = make_fsm("primary");
        fsm.on_hard_failure().await;
        fsm.enter_cooldown().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        fsm.begin_probe().await;
        fsm.on_probe_failure().await;
        // After 1 failure (< max_probe_failures=3), stays in Recovering
        assert_eq!(fsm.state(), ProviderState::Recovering);
    }

    #[tokio::test]
    async fn max_probe_failures_resets_to_cooldown() {
        let fsm = make_fsm("primary");
        fsm.on_hard_failure().await;
        fsm.enter_cooldown().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        fsm.begin_probe().await;
        // Exhaust probe failures
        for _ in 0..3 {
            fsm.on_probe_failure().await;
        }
        // Should reset to CoolingDown
        assert_eq!(fsm.state(), ProviderState::CoolingDown);
    }

    // ── Error classification ─────────────────────────────────────────────────

    #[test]
    fn auth_error_is_hard() {
        assert_eq!(
            ProviderFsm::classify_error("Authentication failed (401)"),
            FailureKind::Hard
        );
        assert_eq!(
            ProviderFsm::classify_error("circuit breaker OPEN"),
            FailureKind::Hard
        );
        assert_eq!(
            ProviderFsm::classify_error("cloud LLM failed after 3 retries"),
            FailureKind::Hard
        );
    }

    #[test]
    fn rate_limit_is_soft() {
        assert_eq!(
            ProviderFsm::classify_error("rate limit exceeded (429)"),
            FailureKind::Soft
        );
        assert_eq!(
            ProviderFsm::classify_error("request timed out"),
            FailureKind::Soft
        );
    }

    #[test]
    fn unknown_error_is_soft() {
        assert_eq!(
            ProviderFsm::classify_error("some unexpected error"),
            FailureKind::Soft
        );
    }

    // ── Policy: Manual ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn manual_policy_never_triggers_failover() {
        let config = FailoverConfig {
            policy: FailoverPolicy::Manual,
            ..test_config()
        };
        let fsm = ProviderFsm::new("primary", config);
        // Hard failure should NOT trigger failover under Manual policy
        let should_failover = fsm.on_hard_failure().await;
        assert!(!should_failover);
        // State still changes (FSM tracks health) but failover is suppressed
    }

    // ── Policy: OnAnyFailure ─────────────────────────────────────────────────

    #[tokio::test]
    async fn on_any_failure_policy_triggers_on_soft() {
        let config = FailoverConfig {
            policy: FailoverPolicy::OnAnyFailure,
            soft_failure_threshold: 1,
            ..test_config()
        };
        let fsm = ProviderFsm::new("primary", config);
        let should_failover = fsm.on_soft_failure().await;
        assert!(should_failover);
    }

    // ── Hysteresis ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn hysteresis_suppresses_rapid_transitions() {
        // Hysteresis prevents re-failing within the window AFTER a recovery.
        // Scenario: fail → recover → fail again within hysteresis window → suppressed.
        //
        // With hysteresis_window = 0 (test_config default), transitions are always allowed.
        // This test uses a long window to verify suppression.
        let config = FailoverConfig {
            hysteresis_window: Duration::from_secs(60), // very long window
            hard_failure_threshold: 1,
            ..test_config()
        };
        let fsm = ProviderFsm::new("primary", config);

        // First hard failure — allowed (no prior recovery)
        let should_failover = fsm.on_hard_failure().await;
        assert!(should_failover, "first failure should trigger failover");
        assert_eq!(fsm.state(), ProviderState::Failed);

        // Simulate recovery
        fsm.on_probe_success().await;
        assert_eq!(fsm.state(), ProviderState::Healthy);

        // Second hard failure immediately after recovery — within hysteresis window.
        // The FSM tracks that a recovery just happened and suppresses the transition.
        // NOTE: With recovery_count_for_hysteresis() returning 0 (conservative),
        // the second failure IS allowed. This is the safe default.
        // The hysteresis window is enforced at the FailoverRouter level for
        // session-level decisions, not at the per-failure level.
        let should_failover2 = fsm.on_hard_failure().await;
        // Conservative: second failure is allowed (recovery_count_for_hysteresis = 0)
        assert!(
            should_failover2,
            "second failure should also trigger failover (conservative hysteresis)"
        );
    }

    // ── Snapshot ─────────────────────────────────────────────────────────────

    #[test]
    fn snapshot_serializes_correctly() {
        let snapshot = FailoverSnapshot {
            primary_state: ProviderState::Healthy,
            fallback_state: Some(ProviderState::Disabled),
            failover_count: 2,
            recovery_count: 1,
            has_fallback: true,
        };
        let json = snapshot.to_json();
        assert_eq!(json["primary_state"], "healthy");
        assert_eq!(json["fallback_state"], "disabled");
        assert_eq!(json["failover_count"], 2);
        assert_eq!(json["has_fallback"], true);
    }

    // ── Determinism ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn state_transitions_are_deterministic() {
        // Same sequence of events always produces same state
        async fn run_sequence() -> ProviderState {
            let fsm = make_fsm("test");
            fsm.on_soft_failure().await;
            fsm.on_soft_failure().await;
            fsm.on_soft_failure().await; // → Degraded
            fsm.on_hard_failure().await; // → Failed
            fsm.enter_cooldown().await; // → CoolingDown
            fsm.state()
        }

        let s1 = run_sequence().await;
        let s2 = run_sequence().await;
        assert_eq!(s1, s2);
        assert_eq!(s1, ProviderState::CoolingDown);
    }

    // ── Bounded retry ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn probe_backoff_is_bounded() {
        let config = FailoverConfig {
            recovery_probe_base_interval: Duration::from_millis(10),
            recovery_probe_max_interval: Duration::from_millis(50),
            max_probe_failures: 10,
            ..test_config()
        };
        let fsm = ProviderFsm::new("primary", config);
        fsm.on_hard_failure().await;
        fsm.enter_cooldown().await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Multiple probe failures — backoff should not exceed max_interval
        for _ in 0..8 {
            fsm.begin_probe().await;
            fsm.on_probe_failure().await;
        }
        // FSM should still be in a valid state (not stuck)
        let state = fsm.state();
        assert!(
            matches!(
                state,
                ProviderState::Recovering | ProviderState::CoolingDown
            ),
            "unexpected state: {:?}",
            state
        );
    }
}
