//! ResidencyManager (HRA Task 42 / R24).
//!
//! The SINGLE executor of load/warm/cool/evict/swap/restore for every model. Engines (RA,
//! Pressure, WPE) request a residency target through this manager instead of calling
//! `ModelLifecycle` methods directly (Property 15). Transitions are serialized per model — at most
//! one in-flight transition per model; concurrent requests for the same model are rejected with
//! `Busy` so callers retry rather than racing.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use super::lifecycle::{ModelLifecycle, ResidencyState};
use super::types::Residency;

// ── G3: Resident Lock state machine (redesign) ────────────────────────────────────────────────
//
// The UX keystone. Once a model settles on the GPU and stabilizes it becomes `ResidentLocked`:
// NO resize, NO optimization, NO migration, NO automatic restart for performance. This is the
// steady state for ~all of a session and is what permanently kills the between-session
// "Optimizing GPU layers" flapping. A restart only happens on an explicit *break condition*
// (correctness/safety or an explicit user workflow); after any break+reload the model returns to
// `ResidentLocked`. Optimization is a transition EVENT, never a steady-state loop.

/// States of the Resident Lock machine. `Cold → Loading → Resident → Stabilizing → ResidentLocked`
/// is the happy path; the remaining variants are branches/overlays reached via break conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// No model loaded.
    Cold,
    /// Spawn in progress (initial load or post-break reload).
    Loading,
    /// On GPU but not yet locked (the post-load settle window).
    Resident,
    /// Brief micro-state after Resident before locking (warmup/first-token verified).
    Stabilizing,
    /// Locked in place. No performance restart possible.
    ResidentLocked,
    /// Locked + explicitly pinned (anti-thrash / user pin). Stronger than ResidentLocked.
    PinnedResident,
    /// Restoring last-good after a crash/restart.
    Recovering,
    /// OOM / driver fault — shrinking to safe.
    Emergency,
    /// Cross-device move in progress.
    Migrating,
    /// LLM evicted for an explicit image-generation workflow (will be restored).
    ImageOverride,
    /// Running on cloud because local is unavailable.
    CloudFallback,
}

impl LockState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Loading => "loading",
            Self::Resident => "resident",
            Self::Stabilizing => "stabilizing",
            Self::ResidentLocked => "resident_locked",
            Self::PinnedResident => "pinned_resident",
            Self::Recovering => "recovering",
            Self::Emergency => "emergency",
            Self::Migrating => "migrating",
            Self::ImageOverride => "image_override",
            Self::CloudFallback => "cloud_fallback",
        }
    }

    /// True when the model is in a settled, locked state where a performance restart is forbidden.
    pub fn is_locked(&self) -> bool {
        matches!(self, Self::ResidentLocked | Self::PinnedResident)
    }

    /// User-facing banner text for this state (redesign G10). Returns `None` for the locked steady
    /// states — a locked model is silent (NO banner), which is the whole point: the user never sees
    /// a "Optimizing GPU layers" flash during normal work. Every other (transient) state names the
    /// EXACT action in progress and is expected to clear on its terminal event.
    pub fn user_banner(&self) -> Option<&'static str> {
        match self {
            Self::Cold | Self::Loading => Some("Loading model…"),
            Self::Resident | Self::Stabilizing => Some("Finishing model setup…"),
            // Locked steady states: silent, stable — no banner.
            Self::ResidentLocked | Self::PinnedResident => None,
            Self::Recovering => Some("Recovering GPU…"),
            Self::Emergency => Some("Reducing GPU use to stay stable…"),
            Self::Migrating => Some("Optimizing GPU placement…"),
            Self::ImageOverride => Some("Freeing GPU for image…"),
            Self::CloudFallback => Some("Using cloud…"),
        }
    }
}

/// The only events that may transition a model OUT of a locked state. Everything here is either a
/// correctness/safety reason or an explicit user workflow — never a performance optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakCondition {
    /// User explicitly requested image generation (needs the GPU).
    ImageGeneration,
    /// GPU out-of-memory.
    GpuOom,
    /// GPU driver reset / fell off the bus.
    DriverReset,
    /// Hardware failure.
    HardwareFailure,
    /// User changed the active model.
    ModelChange,
    /// User changed a setting that affects sizing (context, ngl cap, etc.).
    SettingsChange,
    /// Application restart.
    AppRestart,
    /// Explicit maintenance action.
    Maintenance,
    /// Sustained correctness-threatening VRAM pressure (measured free below emergency band for a
    /// dwell). NOT a performance trigger — a safety one.
    SustainedPressure,
    /// Cloud health changed while on CloudFallback (can return local).
    CloudHealthChange,
}

impl BreakCondition {
    /// The lock state a break condition drives the model into before it reloads back to locked.
    pub fn target_state(&self) -> LockState {
        match self {
            Self::ImageGeneration => LockState::ImageOverride,
            Self::GpuOom | Self::DriverReset | Self::HardwareFailure | Self::SustainedPressure => {
                LockState::Emergency
            }
            Self::ModelChange | Self::SettingsChange | Self::AppRestart | Self::Maintenance => {
                LockState::Loading
            }
            Self::CloudHealthChange => LockState::Loading,
        }
    }
}

/// Pure Resident Lock state machine. Holds only the current state; transitions are explicit so the
/// machine is fully unit-testable and free of timing/IO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidentLock {
    state: LockState,
}

impl Default for ResidentLock {
    fn default() -> Self {
        Self {
            state: LockState::Cold,
        }
    }
}

impl ResidentLock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> LockState {
        self.state
    }

    pub fn is_locked(&self) -> bool {
        self.state.is_locked()
    }

    /// Begin loading (from Cold or after a break that targets Loading).
    pub fn begin_load(&mut self) {
        self.state = LockState::Loading;
    }

    /// Load succeeded and the model is on GPU (not yet locked).
    pub fn on_loaded(&mut self) {
        self.state = LockState::Resident;
    }

    /// Enter the brief stabilize window after Resident.
    pub fn begin_stabilize(&mut self) {
        if self.state == LockState::Resident {
            self.state = LockState::Stabilizing;
        }
    }

    /// Lock the model in place. Allowed from Resident/Stabilizing (happy path) and from the
    /// terminal recovery/migration branches once they have reloaded onto the GPU.
    pub fn lock(&mut self) {
        match self.state {
            LockState::Resident
            | LockState::Stabilizing
            | LockState::Recovering
            | LockState::Migrating
            | LockState::ImageOverride
            | LockState::CloudFallback => {
                self.state = LockState::ResidentLocked;
            }
            _ => {}
        }
    }

    /// Pin (anti-thrash / explicit user pin). Only meaningful once locked.
    pub fn pin(&mut self) {
        if self.state.is_locked() {
            self.state = LockState::PinnedResident;
        }
    }

    /// Whether a performance optimization (Restart-class) may even be *considered*. False whenever
    /// the model is locked or in any transient/branch state — i.e. only the pre-lock Resident
    /// window (or a CloudFallback wanting promotion) is eligible. This is the structural guarantee
    /// that the governing law holds: a locked model can never be restarted for performance.
    pub fn perf_optimization_eligible(&self) -> bool {
        matches!(self.state, LockState::Resident | LockState::CloudFallback)
    }

    /// Apply a break condition. A pinned model resists everything except true correctness/safety
    /// emergencies (it may never be broken for an optimization — there is no optimization break).
    /// Returns the new state, or `None` if the break was refused (e.g. pinned vs a soft break).
    pub fn apply_break(&mut self, cond: BreakCondition) -> Option<LockState> {
        let is_emergency = matches!(
            cond,
            BreakCondition::GpuOom
                | BreakCondition::DriverReset
                | BreakCondition::HardwareFailure
                | BreakCondition::SustainedPressure
        );
        if self.state == LockState::PinnedResident && !is_emergency {
            // Pinned models only yield to correctness emergencies, not to user-workflow breaks.
            // (Image generation on a pinned model is handled by co-residency, not a break.)
            return None;
        }
        let target = cond.target_state();
        self.state = target;
        Some(target)
    }

    /// Return to the locked steady state after a break's reload completes.
    pub fn relock(&mut self) {
        self.state = LockState::ResidentLocked;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidencyError {
    /// A transition for this model is already in flight.
    Busy,
}

struct Entry {
    lifecycle: Arc<dyn ModelLifecycle>,
    state: ResidencyState,
    in_flight: bool,
}

#[derive(Default)]
struct Inner {
    models: HashMap<String, Entry>,
}

pub struct ResidencyManager {
    inner: Mutex<Inner>,
}

impl Default for ResidencyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ResidencyManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Register a model's lifecycle adapter (idempotent by id).
    pub async fn register(&self, lifecycle: Arc<dyn ModelLifecycle>) {
        let id = lifecycle.descriptor().id;
        let mut inner = self.inner.lock().await;
        inner.models.insert(
            id,
            Entry {
                lifecycle,
                state: ResidencyState::Unloaded,
                in_flight: false,
            },
        );
    }

    pub async fn state(&self, model: &str) -> Option<ResidencyState> {
        self.inner.lock().await.models.get(model).map(|e| e.state)
    }

    /// Drive `model` toward `target`. Serialized per model: returns `Busy` if a transition is
    /// already running for this model. The lifecycle ops run WITHOUT holding the map lock so other
    /// models can transition concurrently.
    pub async fn transition(&self, model: &str, target: Residency) -> Result<(), ResidencyError> {
        // Phase 1: claim the in-flight slot + clone the lifecycle handle.
        let (lifecycle, from) = {
            let mut inner = self.inner.lock().await;
            let entry = inner.models.get_mut(model).ok_or(ResidencyError::Busy)?; // unknown model treated as unavailable
            if entry.in_flight {
                return Err(ResidencyError::Busy);
            }
            entry.in_flight = true;
            entry.state = transient_for(target);
            (entry.lifecycle.clone(), entry.state)
        };
        let _ = from;

        // Phase 2: run the (possibly slow) transition without the lock.
        let result = execute(lifecycle.as_ref(), target).await;

        // Phase 3: commit final state + release in-flight.
        let mut inner = self.inner.lock().await;
        if let Some(entry) = inner.models.get_mut(model) {
            entry.in_flight = false;
            entry.state = if result.is_ok() {
                ResidencyState::from_residency(target)
            } else {
                // failed transition leaves model in a safe Unloaded-or-prior; mark Unloaded.
                ResidencyState::Unloaded
            };
        }
        Ok(())
    }
}

fn transient_for(target: Residency) -> ResidencyState {
    match target {
        Residency::VramHot => ResidencyState::Loading,
        Residency::RamWarm | Residency::DiskCold => ResidencyState::Cooling,
        Residency::Cloud => ResidencyState::Restoring,
        Residency::Unloaded => ResidencyState::Cooling,
    }
}

async fn execute(lc: &dyn ModelLifecycle, target: Residency) -> anyhow::Result<()> {
    match target {
        Residency::VramHot => {
            lc.load().await?;
            lc.warm().await
        }
        Residency::RamWarm | Residency::DiskCold => lc.cool().await,
        Residency::Cloud => lc.swap(Residency::Cloud).await,
        Residency::Unloaded => lc.unload().await,
    }
}

#[cfg(test)]
mod tests {
    use super::super::lifecycle::{ModelDescriptor, ModelHealth};
    use super::super::types::ConsumerId;
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct MockLifecycle {
        id: String,
        loads: Arc<AtomicU32>,
        gate: Arc<tokio::sync::Notify>,
        block: bool,
    }

    #[async_trait]
    impl ModelLifecycle for MockLifecycle {
        fn descriptor(&self) -> ModelDescriptor {
            ModelDescriptor {
                id: self.id.clone(),
                kind: ConsumerId::Llm,
                vram_est_mb: 4000,
                ram_est_mb: 2000,
            }
        }
        async fn load(&self) -> anyhow::Result<()> {
            if self.block {
                self.gate.notified().await;
            }
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn warm(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn cool(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn unload(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn swap(&self, _t: Residency) -> anyhow::Result<()> {
            Ok(())
        }
        fn health(&self) -> ModelHealth {
            ModelHealth::Healthy
        }
    }

    #[tokio::test]
    async fn transition_updates_state() {
        let mgr = ResidencyManager::new();
        let loads = Arc::new(AtomicU32::new(0));
        mgr.register(Arc::new(MockLifecycle {
            id: "m".into(),
            loads: loads.clone(),
            gate: Arc::new(tokio::sync::Notify::new()),
            block: false,
        }))
        .await;
        assert_eq!(mgr.state("m").await, Some(ResidencyState::Unloaded));
        mgr.transition("m", Residency::VramHot).await.unwrap();
        assert_eq!(mgr.state("m").await, Some(ResidencyState::VramHot));
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn second_concurrent_transition_is_busy() {
        let mgr = Arc::new(ResidencyManager::new());
        let gate = Arc::new(tokio::sync::Notify::new());
        mgr.register(Arc::new(MockLifecycle {
            id: "m".into(),
            loads: Arc::new(AtomicU32::new(0)),
            gate: gate.clone(),
            block: true,
        }))
        .await;

        // Start a blocking transition.
        let mgr1 = mgr.clone();
        let h = tokio::spawn(async move { mgr1.transition("m", Residency::VramHot).await });

        // Give the first transition time to claim the in-flight slot.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Second transition while first is in-flight → Busy.
        let busy = mgr.transition("m", Residency::VramHot).await;
        assert_eq!(busy, Err(ResidencyError::Busy));

        // Release the first transition.
        gate.notify_one();
        h.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn different_models_transition_concurrently() {
        let mgr = ResidencyManager::new();
        for id in ["a", "b"] {
            mgr.register(Arc::new(MockLifecycle {
                id: id.into(),
                loads: Arc::new(AtomicU32::new(0)),
                gate: Arc::new(tokio::sync::Notify::new()),
                block: false,
            }))
            .await;
        }
        mgr.transition("a", Residency::VramHot).await.unwrap();
        mgr.transition("b", Residency::RamWarm).await.unwrap();
        assert_eq!(mgr.state("a").await, Some(ResidencyState::VramHot));
        assert_eq!(mgr.state("b").await, Some(ResidencyState::RamWarm));
    }

    // ── G3: Resident Lock state machine tests ────────────────────────────────

    #[test]
    fn happy_path_reaches_locked() {
        let mut lock = ResidentLock::new();
        assert_eq!(lock.state(), LockState::Cold);
        lock.begin_load();
        assert_eq!(lock.state(), LockState::Loading);
        lock.on_loaded();
        assert_eq!(lock.state(), LockState::Resident);
        lock.begin_stabilize();
        assert_eq!(lock.state(), LockState::Stabilizing);
        lock.lock();
        assert_eq!(lock.state(), LockState::ResidentLocked);
        assert!(lock.is_locked());
    }

    #[test]
    fn locked_model_is_not_perf_optimization_eligible() {
        let mut lock = ResidentLock::new();
        lock.begin_load();
        lock.on_loaded();
        // pre-lock Resident IS eligible for the one-time promotion
        assert!(lock.perf_optimization_eligible());
        lock.lock();
        // once locked, structurally ineligible — the governing-law guarantee
        assert!(!lock.perf_optimization_eligible());
    }

    #[test]
    fn cloud_fallback_is_promotion_eligible() {
        // Force a cloud-fallback posture and check eligibility.
        let lock = ResidentLock {
            state: LockState::CloudFallback,
        };
        assert!(lock.perf_optimization_eligible());
    }

    #[test]
    fn emergency_break_drives_to_emergency_state() {
        let mut lock = ResidentLock::new();
        lock.begin_load();
        lock.on_loaded();
        lock.lock();
        let new = lock.apply_break(BreakCondition::GpuOom).unwrap();
        assert_eq!(new, LockState::Emergency);
        assert!(!lock.is_locked());
    }

    #[test]
    fn image_generation_break_targets_image_override() {
        let mut lock = ResidentLock {
            state: LockState::ResidentLocked,
        };
        let new = lock.apply_break(BreakCondition::ImageGeneration).unwrap();
        assert_eq!(new, LockState::ImageOverride);
        // after image gen completes, relock
        lock.relock();
        assert_eq!(lock.state(), LockState::ResidentLocked);
    }

    #[test]
    fn pinned_model_resists_soft_breaks_but_yields_to_emergency() {
        let mut lock = ResidentLock {
            state: LockState::ResidentLocked,
        };
        lock.pin();
        assert_eq!(lock.state(), LockState::PinnedResident);
        // soft break (image / settings) refused
        assert_eq!(lock.apply_break(BreakCondition::ImageGeneration), None);
        assert_eq!(lock.apply_break(BreakCondition::SettingsChange), None);
        assert_eq!(lock.state(), LockState::PinnedResident);
        // correctness emergency still breaks it
        assert_eq!(
            lock.apply_break(BreakCondition::DriverReset),
            Some(LockState::Emergency)
        );
    }

    #[test]
    fn relock_after_any_break_returns_to_locked() {
        for cond in [
            BreakCondition::ModelChange,
            BreakCondition::SettingsChange,
            BreakCondition::AppRestart,
            BreakCondition::Maintenance,
            BreakCondition::GpuOom,
        ] {
            let mut lock = ResidentLock {
                state: LockState::ResidentLocked,
            };
            lock.apply_break(cond);
            assert!(!lock.is_locked());
            lock.relock();
            assert_eq!(lock.state(), LockState::ResidentLocked);
        }
    }
}
