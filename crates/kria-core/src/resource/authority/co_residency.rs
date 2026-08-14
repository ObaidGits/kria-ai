//! Co-Residency GPU Lease Manager (HRA Phase B — the production residency authority).
//!
//! # Why this exists
//! The legacy `resource::gpu_lease::GpuLeaseManager` is **single-holder**: exactly one consumer can
//! hold the GPU at a time, so the LLM and image generation can never be resident together and the
//! LLM must be fully restarted (CPU↔GPU) around every image job — the "Optimizing GPU layers" cost.
//!
//! The HRA control plane already supports co-residency at the admission layer: the single
//! [`Scheduler`](super::scheduler::Scheduler) reserves VRAM per device through the single
//! [`DeviceTable`](super::device_table::DeviceTable), so multiple leases coexist on one GPU as long
//! as the VRAM budget (Soft/Hard/Emergency bands) holds, and a strictly-higher class preempts a
//! strictly-lower holder. This module is the cohesive *coordinator* that turns that admission core
//! into a usable production lease authority by adding the protocol the raw scheduler lacks:
//!
//! - **Iterative multi-victim preemption** — the scheduler reports one victim at a time; the
//!   coordinator loops, gracefully evicting victims (via the single
//!   [`ResidencyManager`](super::residency_manager::ResidencyManager)) until the request fits or no
//!   further preemption is permitted.
//! - **Cooperative revocation** — a preempted holder's lease is marked `revoked`; holders check
//!   [`CoResidencyLease::is_valid`] before GPU work and yield. No forced kill of in-flight work.
//! - **Foreground protection** — only a strictly-higher class can preempt; equal/higher holders are
//!   never evicted (delegated to the scheduler's `Busy` vs `PreemptionRequired` distinction).
//! - **Anti-thrash pinning** — a freshly granted background resident is pinned for a dwell window so
//!   it cannot be immediately re-evicted (prevents oscillation under churn).
//! - **Refcount dedup** — a model already hot-resident is shared (refcounted), never loaded twice
//!   (no duplicate residency, no double ownership).
//! - **Rollback** — if the residency load fails after admission, the tentative lease is released so
//!   the reservation never leaks.
//! - **Recovery** — TTL sweep reclaims leases whose holder vanished; epoch fencing + the persisted
//!   journal cover crash recovery.
//!
//! # Concurrency model (deadlock-free by construction)
//! The coordinator's own state is a `tokio::Mutex<Inner>`. The lock is **never** held across an
//! `.await`: the protocol takes the lock only for short bookkeeping, releases it, then performs the
//! synchronous (fast) `authority.request`/`release` and the asynchronous `ResidencyManager`
//! transitions outside the lock. The authority and residency manager each have their own internal
//! locking and are designed for this layering (the residency manager explicitly runs lifecycle ops
//! without its map lock). There is a strict lock acquisition order — coordinator → (released) →
//! authority → (released) → residency — so no lock cycle can form.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::ra::{LocalAuthority, RaOutcome, ResourceAuthority};
use super::residency_manager::ResidencyManager;
use super::scheduler::LeaseToken;
use super::types::{PriorityClass, Residency, ResourceRequest};

/// Tunable co-residency policy. Defaults are conservative and hardware-agnostic.
#[derive(Debug, Clone, Copy)]
pub struct CoResidencyPolicy {
    /// Maximum victims to preempt for a single admission before giving up (bounds the loop).
    pub max_preemptions: u32,
    /// Dwell window during which a freshly granted background resident cannot be re-evicted.
    pub pin_dwell: Duration,
    /// Lease TTL: a holder that does not refresh within this window is reclaimable by the sweep.
    pub lease_ttl: Duration,
}

impl Default for CoResidencyPolicy {
    fn default() -> Self {
        Self {
            max_preemptions: 8,
            pin_dwell: Duration::from_secs(20),
            lease_ttl: Duration::from_secs(180),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoResidencyError {
    /// Device contended by an equal-or-higher class; caller should back off / fall back.
    Busy,
    /// Shed under overload (low priority + full queue).
    Shed,
    /// Admission succeeded but the residency load failed; the lease was rolled back.
    ResidencyFailed,
    /// Could not make room without evicting a pinned (anti-thrash) background resident.
    Pinned,
    /// Preemption bound hit without fitting — safety stop (no infinite preemption).
    TooManyPreemptions,
}

/// What physically must happen to the model on grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidencyTarget {
    Hot,
    Warm,
}

impl ResidencyTarget {
    fn residency(self) -> Residency {
        match self {
            ResidencyTarget::Hot => Residency::VramHot,
            ResidencyTarget::Warm => Residency::RamWarm,
        }
    }
}

/// Internal record of a live co-resident holder.
struct Holder {
    model: String,
    class: PriorityClass,
    device: super::types::DeviceId,
    /// Reference count: how many outstanding leases share this residency (dedup).
    refs: u32,
    granted_at: Instant,
    last_refresh: Instant,
    pinned_until: Instant,
    /// Cooperative revocation flag shared with every `CoResidencyLease` for this holder.
    revoked: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
    /// Authority lease token → holder.
    holders: HashMap<u64, Holder>,
    /// Model id → its authority lease token (dedup index).
    by_model: HashMap<String, u64>,
}

/// The production co-residency authority. One per process (wraps the single authority + residency
/// executor). Cheap to clone via `Arc`.
pub struct CoResidencyManager {
    authority: Arc<LocalAuthority>,
    residency: Arc<ResidencyManager>,
    inner: Mutex<Inner>,
    policy: CoResidencyPolicy,
    preemptions: AtomicU64,
    rollbacks: AtomicU64,
    dedup_hits: AtomicU64,
}

impl CoResidencyManager {
    pub fn new(authority: Arc<LocalAuthority>, residency: Arc<ResidencyManager>) -> Arc<Self> {
        Self::with_policy(authority, residency, CoResidencyPolicy::default())
    }

    pub fn with_policy(
        authority: Arc<LocalAuthority>,
        residency: Arc<ResidencyManager>,
        policy: CoResidencyPolicy,
    ) -> Arc<Self> {
        Arc::new(Self {
            authority,
            residency,
            inner: Mutex::new(Inner::default()),
            policy,
            preemptions: AtomicU64::new(0),
            rollbacks: AtomicU64::new(0),
            dedup_hits: AtomicU64::new(0),
        })
    }

    pub fn metrics(&self) -> CoResidencyMetrics {
        CoResidencyMetrics {
            preemptions: self.preemptions.load(Ordering::Relaxed),
            rollbacks: self.rollbacks.load(Ordering::Relaxed),
            dedup_hits: self.dedup_hits.load(Ordering::Relaxed),
        }
    }

    /// Acquire a co-residency lease for `model_id`, running the preempt-retry admission protocol.
    ///
    /// Returns a RAII [`CoResidencyLease`]; dropping it releases the reference (and the underlying
    /// residency once the last reference is gone). The model is NOT loaded twice if already hot —
    /// the existing residency is shared (refcounted).
    pub async fn acquire(
        self: &Arc<Self>,
        req: &ResourceRequest,
        target: ResidencyTarget,
    ) -> Result<CoResidencyLease, CoResidencyError> {
        let model = req
            .need
            .model_id
            .clone()
            .unwrap_or_else(|| format!("{:?}", req.consumer));

        // Fast path: dedup. If the model is already resident and valid, share it (no reload).
        {
            let mut inner = self.inner.lock().await;
            if let Some(&tok) = inner.by_model.get(&model) {
                if let Some(h) = inner.holders.get_mut(&tok) {
                    if !h.revoked.load(Ordering::Acquire) {
                        h.refs += 1;
                        h.last_refresh = Instant::now();
                        let revoked = h.revoked.clone();
                        let device = h.device.clone();
                        self.dedup_hits.fetch_add(1, Ordering::Relaxed);
                        return Ok(CoResidencyLease::new(
                            tok,
                            model,
                            device,
                            revoked,
                            self.clone(),
                        ));
                    }
                }
            }
        }

        let emergency = req.class >= PriorityClass::InteractiveFg;
        let mut preempted = 0u32;

        loop {
            // Admission attempt (synchronous, fast; coordinator lock NOT held). GPU-only — the
            // coordinator runs the preemption protocol itself and surfaces Busy rather than letting
            // the planner silently degrade to CPU (the caller decides CPU fallback).
            let outcome = self.authority.request_on_gpu(req);
            match outcome {
                RaOutcome::Granted(lease) => {
                    // Execute the residency transition OUTSIDE any coordinator lock.
                    let ok = self.execute_residency(&model, target).await;
                    if !ok {
                        // Rollback: free the reservation so it never leaks.
                        self.authority.release(lease.token);
                        self.rollbacks.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            target: "hra",
                            consumer = ?req.consumer, model = %model, device = ?lease.device,
                            vram_mb = req.need.vram_mb, preempted,
                            "co-residency DECISION=rollback reason=residency_load_failed — reservation released"
                        );
                        return Err(CoResidencyError::ResidencyFailed);
                    }
                    tracing::info!(
                        target: "hra",
                        consumer = ?req.consumer, class = req.class.as_str(), model = %model,
                        device = ?lease.device, vram_mb = req.need.vram_mb, preempted,
                        decision = if preempted > 0 { "granted_after_evict" } else { "granted_coresident" },
                        "co-residency DECISION=accepted — model admitted to GPU residency"
                    );
                    return Ok(self
                        .record_grant(lease.token.0, model, req.class, lease.device)
                        .await);
                }
                RaOutcome::PreemptThenRetry { victim } => {
                    match self.preempt_victim(victim, emergency).await {
                        Ok(()) => {
                            preempted += 1;
                            self.preemptions.fetch_add(1, Ordering::Relaxed);
                            if preempted > self.policy.max_preemptions {
                                tracing::warn!(
                                    target: "hra",
                                    consumer = ?req.consumer, model = %model, preempted,
                                    "co-residency DECISION=denied reason=too_many_preemptions (safety bound)"
                                );
                                return Err(CoResidencyError::TooManyPreemptions);
                            }
                            continue;
                        }
                        Err(e) => {
                            tracing::info!(
                                target: "hra",
                                consumer = ?req.consumer, class = req.class.as_str(), model = %model,
                                reason = ?e,
                                "co-residency DECISION=denied — could not make room (victim pinned / protected)"
                            );
                            return Err(e);
                        }
                    }
                }
                RaOutcome::Busy => {
                    tracing::info!(
                        target: "hra",
                        consumer = ?req.consumer, class = req.class.as_str(), model = %model,
                        vram_mb = req.need.vram_mb,
                        "co-residency DECISION=busy reason=gpu_held_by_equal_or_higher_class — caller falls back"
                    );
                    return Err(CoResidencyError::Busy);
                }
                RaOutcome::Shed => {
                    tracing::warn!(
                        target: "hra",
                        consumer = ?req.consumer, class = req.class.as_str(), model = %model,
                        "co-residency DECISION=shed reason=overload_queue_full"
                    );
                    return Err(CoResidencyError::Shed);
                }
            }
        }
    }

    /// Drive the model to the requested residency tier and confirm it actually got there. Returns
    /// false on load failure (the ResidencyManager swallows the error into an Unloaded state, so we
    /// verify the resulting state rather than trusting the Ok).
    async fn execute_residency(&self, model: &str, target: ResidencyTarget) -> bool {
        // A model unknown to the residency manager (not registered) is treated as a pure
        // reservation (the consumer manages its own process) — admission already succeeded.
        if self.residency.state(model).await.is_none() {
            return true;
        }
        if self
            .residency
            .transition(model, target.residency())
            .await
            .is_err()
        {
            return false; // Busy (another transition in flight) — caller retries
        }
        match self.residency.state(model).await {
            Some(st) => st.is_resident_at_least(target.residency()),
            None => true,
        }
    }

    /// Gracefully evict a victim: refuse if it is pinned (anti-thrash) unless this is an emergency
    /// (foreground) admission. On eviction, mark the lease revoked (cooperative), cool the model to
    /// RAM, and release the authority reservation.
    async fn preempt_victim(
        &self,
        victim: LeaseToken,
        emergency: bool,
    ) -> Result<(), CoResidencyError> {
        let (model, revoked) = {
            let inner = self.inner.lock().await;
            match inner.holders.get(&victim.0) {
                Some(h) => {
                    if h.pinned_until > Instant::now() && !emergency {
                        return Err(CoResidencyError::Pinned);
                    }
                    (h.model.clone(), h.revoked.clone())
                }
                // Unknown victim (e.g. a raw reservation with no coordinator holder) — just release
                // it in the authority so admission can proceed.
                None => {
                    self.authority.release(victim);
                    return Ok(());
                }
            }
        };

        // Mark revoked BEFORE eviction so the holder stops issuing new GPU work immediately.
        revoked.store(true, Ordering::Release);
        tracing::info!(
            target: "hra",
            victim_token = victim.0, model = %model, emergency,
            "co-residency DECISION=preempted — evicting lower-priority background resident to RAM"
        );

        // Graceful eviction to RAM (outside the coordinator lock).
        let _ = self.residency.transition(&model, Residency::RamWarm).await;

        // Free the reservation + drop the holder bookkeeping.
        self.authority.release(victim);
        let mut inner = self.inner.lock().await;
        if let Some(h) = inner.holders.remove(&victim.0) {
            inner.by_model.remove(&h.model);
        }
        Ok(())
    }

    async fn record_grant(
        self: &Arc<Self>,
        token: u64,
        model: String,
        class: PriorityClass,
        device: super::types::DeviceId,
    ) -> CoResidencyLease {
        let now = Instant::now();
        // Background residents are pinned for the dwell window to prevent immediate re-eviction.
        let pinned_until = if class <= PriorityClass::InteractiveBg {
            now + self.policy.pin_dwell
        } else {
            now
        };
        let revoked = Arc::new(AtomicBool::new(false));
        let mut inner = self.inner.lock().await;
        inner.by_model.insert(model.clone(), token);
        inner.holders.insert(
            token,
            Holder {
                model: model.clone(),
                class,
                device: device.clone(),
                refs: 1,
                granted_at: now,
                last_refresh: now,
                pinned_until,
                revoked: revoked.clone(),
            },
        );
        CoResidencyLease::new(token, model, device, revoked, self.clone())
    }

    /// Refresh a lease's TTL (call periodically while the work is ongoing).
    pub async fn refresh(&self, token: u64) {
        let mut inner = self.inner.lock().await;
        if let Some(h) = inner.holders.get_mut(&token) {
            h.last_refresh = Instant::now();
        }
    }

    /// Release one reference of a lease. When the last reference is dropped the authority
    /// reservation is freed. The model is left RAM-warm (cheap to re-promote) rather than fully
    /// unloaded — the idle monitor / pressure engine decides final unload.
    async fn release(&self, token: u64) {
        let release_authority = {
            let mut inner = self.inner.lock().await;
            match inner.holders.get_mut(&token) {
                Some(h) => {
                    h.refs = h.refs.saturating_sub(1);
                    if h.refs == 0 {
                        if let Some(h) = inner.holders.remove(&token) {
                            inner.by_model.remove(&h.model);
                        }
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        };
        if release_authority {
            self.authority.release(LeaseToken(token));
        }
    }

    /// Reclaim leases whose holder hasn't refreshed within the TTL (recovery sweep). Returns the
    /// number reclaimed. Safe to call periodically from a background task.
    pub async fn reclaim_expired(&self) -> usize {
        let now = Instant::now();
        let ttl = self.policy.lease_ttl;
        let mut expired: Vec<u64> = Vec::new();
        {
            let inner = self.inner.lock().await;
            for (tok, h) in inner.holders.iter() {
                if now.saturating_duration_since(h.last_refresh) >= ttl {
                    expired.push(*tok);
                }
            }
        }
        let mut count = 0;
        for tok in expired {
            let model = {
                let mut inner = self.inner.lock().await;
                inner.holders.remove(&tok).map(|h| {
                    h.revoked.store(true, Ordering::Release);
                    inner.by_model.remove(&h.model);
                    h.model
                })
            };
            if let Some(model) = model {
                let _ = self.residency.transition(&model, Residency::RamWarm).await;
                self.authority.release(LeaseToken(tok));
                tracing::info!(
                    target: "hra",
                    token = tok, model = %model,
                    "co-residency DECISION=recovered reason=lease_ttl_expired — stale lease reclaimed, model cooled"
                );
                count += 1;
            }
        }
        count
    }

    /// Current number of live co-resident holders (diagnostics).
    pub async fn resident_count(&self) -> usize {
        self.inner.lock().await.holders.len()
    }

    /// Snapshot of live residents for diagnostics/UI (model, class, age_ms, refs, revoked).
    pub async fn residents_snapshot(&self) -> Vec<ResidentSnapshot> {
        let now = Instant::now();
        let inner = self.inner.lock().await;
        inner
            .holders
            .values()
            .map(|h| ResidentSnapshot {
                model: h.model.clone(),
                class: h.class,
                device: format!("{:?}", h.device),
                age_ms: now.saturating_duration_since(h.granted_at).as_millis() as u64,
                refs: h.refs,
                pinned: h.pinned_until > now,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoResidencyMetrics {
    pub preemptions: u64,
    pub rollbacks: u64,
    pub dedup_hits: u64,
}

#[derive(Debug, Clone)]
pub struct ResidentSnapshot {
    pub model: String,
    pub class: PriorityClass,
    pub device: String,
    pub age_ms: u64,
    pub refs: u32,
    pub pinned: bool,
}

/// RAII handle for a co-residency reservation. Dropping it releases one reference. Holders MUST
/// check [`is_valid`](Self::is_valid) before each GPU operation and yield if it returns false
/// (cooperative preemption — a higher-priority request revoked this residency).
pub struct CoResidencyLease {
    token: u64,
    model: String,
    device: super::types::DeviceId,
    revoked: Arc<AtomicBool>,
    manager: Arc<CoResidencyManager>,
    released: AtomicBool,
}

impl CoResidencyLease {
    fn new(
        token: u64,
        model: String,
        device: super::types::DeviceId,
        revoked: Arc<AtomicBool>,
        manager: Arc<CoResidencyManager>,
    ) -> Self {
        Self {
            token,
            model,
            device,
            revoked,
            manager,
            released: AtomicBool::new(false),
        }
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn device(&self) -> &super::types::DeviceId {
        &self.device
    }

    /// True while this residency is still valid. Becomes false if a higher-priority request
    /// preempted it; the holder should stop GPU work at the next checkpoint and re-acquire.
    pub fn is_valid(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
    }

    /// Explicit async release (preferred over relying on Drop in async contexts).
    pub async fn release(self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.manager.release(self.token).await;
        }
    }
}

impl Drop for CoResidencyLease {
    fn drop(&mut self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        // Best-effort release on drop. Prefer the async `release` in async contexts; this Drop path
        // schedules the release on the current runtime if one is available.
        let manager = self.manager.clone();
        let token = self.token;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                manager.release(token).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::authority::lifecycle::{
        ModelDescriptor, ModelHealth, ModelLifecycle,
    };
    use crate::resource::authority::planner::PolicyProfile;
    use crate::resource::authority::types::{Constraints, ConsumerId, ResourceNeed, TurnId};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    /// Mock model lifecycle. Can be told to fail load (to exercise rollback) and counts load/cool.
    struct MockModel {
        id: String,
        kind: ConsumerId,
        loads: Arc<AtomicU32>,
        cools: Arc<AtomicU32>,
        fail_load: Arc<AtomicBool>,
    }

    impl MockModel {
        fn new(
            id: &str,
            kind: ConsumerId,
        ) -> (Arc<Self>, Arc<AtomicU32>, Arc<AtomicU32>, Arc<AtomicBool>) {
            let loads = Arc::new(AtomicU32::new(0));
            let cools = Arc::new(AtomicU32::new(0));
            let fail = Arc::new(AtomicBool::new(false));
            (
                Arc::new(Self {
                    id: id.into(),
                    kind,
                    loads: loads.clone(),
                    cools: cools.clone(),
                    fail_load: fail.clone(),
                }),
                loads,
                cools,
                fail,
            )
        }
    }

    #[async_trait]
    impl ModelLifecycle for MockModel {
        fn descriptor(&self) -> ModelDescriptor {
            ModelDescriptor {
                id: self.id.clone(),
                kind: self.kind,
                vram_est_mb: 4000,
                ram_est_mb: 2000,
            }
        }
        async fn load(&self) -> anyhow::Result<()> {
            if self.fail_load.load(Ordering::Acquire) {
                anyhow::bail!("simulated load failure");
            }
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn warm(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn cool(&self) -> anyhow::Result<()> {
            self.cools.fetch_add(1, Ordering::SeqCst);
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

    fn authority(gpu_vram: u64) -> Arc<LocalAuthority> {
        Arc::new(LocalAuthority::bootstrap(
            &[(0, gpu_vram)],
            512,
            32768,
            &[],
            PolicyProfile::Balanced,
        ))
    }

    fn req(consumer: ConsumerId, class: PriorityClass, vram: u64, model: &str) -> ResourceRequest {
        ResourceRequest {
            consumer,
            class,
            need: ResourceNeed {
                vram_mb: vram,
                ram_mb: 2048,
                cpu_threads: 4,
                exclusivity: false,
                model_id: Some(model.into()),
                est_ms: 1000,
            },
            constraints: Constraints::default(),
            turn_id: TurnId(format!("turn-{model}")),
        }
    }

    async fn mgr_with_models(
        gpu_vram: u64,
        models: &[(&str, ConsumerId)],
    ) -> Arc<CoResidencyManager> {
        let auth = authority(gpu_vram);
        let res = Arc::new(ResidencyManager::new());
        for (id, kind) in models {
            let (m, _, _, _) = MockModel::new(id, *kind);
            res.register(m).await;
        }
        CoResidencyManager::new(auth, res)
    }

    // ── Functional ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn two_models_co_reside_on_one_gpu() {
        // 12 GB GPU; LLM 4 GB + Image 4 GB fit together → both hot, no preemption.
        let mgr = mgr_with_models(
            12288,
            &[("llm", ConsumerId::Llm), ("img", ConsumerId::Image)],
        )
        .await;
        let l1 = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 4000, "llm"),
                ResidencyTarget::Hot,
            )
            .await
            .expect("llm grant");
        let l2 = mgr
            .acquire(
                &req(ConsumerId::Image, PriorityClass::Batch, 4000, "img"),
                ResidencyTarget::Hot,
            )
            .await
            .expect("image grant");
        assert!(l1.is_valid() && l2.is_valid());
        assert_eq!(mgr.resident_count().await, 2, "both co-resident");
        assert_eq!(mgr.metrics().preemptions, 0, "co-residency, not preemption");
    }

    #[tokio::test]
    async fn dedup_shares_residency_without_reloading() {
        let auth = authority(12288);
        let res = Arc::new(ResidencyManager::new());
        let (m, loads, _, _) = MockModel::new("llm", ConsumerId::Llm);
        res.register(m).await;
        let mgr = CoResidencyManager::new(auth, res);

        let a = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 4000, "llm"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        let b = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 4000, "llm"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        // Same model acquired twice → loaded once, shared (refcount 2).
        assert_eq!(loads.load(Ordering::SeqCst), 1, "no duplicate residency");
        assert_eq!(mgr.metrics().dedup_hits, 1);
        assert_eq!(mgr.resident_count().await, 1);
        a.release().await;
        // Still resident under the second reference.
        assert_eq!(mgr.resident_count().await, 1);
        b.release().await;
        assert_eq!(mgr.resident_count().await, 0, "freed after last ref");
    }

    // ── Preemption + foreground protection ──────────────────────────────────

    #[tokio::test]
    async fn foreground_preempts_background_to_fit() {
        // 8 GB GPU. Background image 6 GB resident; foreground LLM 6 GB needs room → preempt image.
        let mgr = mgr_with_models(
            8192,
            &[("llm", ConsumerId::Llm), ("img", ConsumerId::Image)],
        )
        .await;
        let img = mgr
            .acquire(
                &req(ConsumerId::Image, PriorityClass::Batch, 6000, "img"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        assert!(img.is_valid());
        // FG LLM must preempt the background image. NOTE: image is pinned for the dwell window, but
        // emergency (InteractiveFg) admission overrides the pin.
        let llm = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 6000, "llm"),
                ResidencyTarget::Hot,
            )
            .await
            .expect("fg should preempt bg and grant");
        assert!(llm.is_valid());
        assert!(
            !img.is_valid(),
            "preempted background lease must be revoked (cooperative)"
        );
        assert!(mgr.metrics().preemptions >= 1);
    }

    #[tokio::test]
    async fn background_never_preempts_foreground() {
        // 8 GB GPU. Foreground LLM 6 GB resident; background image 6 GB cannot preempt it → Busy.
        let mgr = mgr_with_models(
            8192,
            &[("llm", ConsumerId::Llm), ("img", ConsumerId::Image)],
        )
        .await;
        let llm = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 6000, "llm"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        let outcome = mgr
            .acquire(
                &req(ConsumerId::Image, PriorityClass::Batch, 6000, "img"),
                ResidencyTarget::Hot,
            )
            .await;
        assert_eq!(
            outcome.err(),
            Some(CoResidencyError::Busy),
            "bg must not preempt fg"
        );
        assert!(llm.is_valid(), "foreground residency untouched");
    }

    #[tokio::test]
    async fn equal_class_does_not_preempt() {
        let mgr = mgr_with_models(8192, &[("a", ConsumerId::Llm), ("b", ConsumerId::Image)]).await;
        let _a = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::Batch, 6000, "a"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        let b = mgr
            .acquire(
                &req(ConsumerId::Image, PriorityClass::Batch, 6000, "b"),
                ResidencyTarget::Hot,
            )
            .await;
        assert_eq!(
            b.err(),
            Some(CoResidencyError::Busy),
            "equal class cannot preempt"
        );
    }

    // ── Anti-thrash pinning ─────────────────────────────────────────────────

    #[tokio::test]
    async fn pinned_background_not_evicted_by_non_emergency() {
        // Interactive-bg (not emergency) request must not evict a freshly pinned background resident.
        let mgr =
            mgr_with_models(8192, &[("a", ConsumerId::Image), ("b", ConsumerId::Vision)]).await;
        let _a = mgr
            .acquire(
                &req(ConsumerId::Image, PriorityClass::Batch, 6000, "a"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        // InteractiveBg outranks Batch so the scheduler would preempt, but the victim is pinned and
        // the requester is not foreground-emergency → Pinned (caller falls back).
        let b = mgr
            .acquire(
                &req(ConsumerId::Vision, PriorityClass::InteractiveBg, 6000, "b"),
                ResidencyTarget::Hot,
            )
            .await;
        assert_eq!(b.err(), Some(CoResidencyError::Pinned));
    }

    // ── Rollback ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn failed_load_rolls_back_reservation() {
        let auth = authority(8192);
        let res = Arc::new(ResidencyManager::new());
        let (m, _, _, fail) = MockModel::new("llm", ConsumerId::Llm);
        res.register(m).await;
        fail.store(true, Ordering::Release); // force load failure
        let mgr = CoResidencyManager::new(auth.clone(), res);

        let outcome = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 6000, "llm"),
                ResidencyTarget::Hot,
            )
            .await;
        assert_eq!(outcome.err(), Some(CoResidencyError::ResidencyFailed));
        assert_eq!(mgr.metrics().rollbacks, 1);
        assert_eq!(
            mgr.resident_count().await,
            0,
            "no holder recorded on failure"
        );
        // Reservation released → a fresh request can now fit the full GPU.
        assert!(
            auth.with_table_for_compare(|t| t
                .get(&crate::resource::authority::types::DeviceId::Gpu(0))
                .unwrap()
                .reserved_vram_mb)
                == 0
        );
    }

    // ── Recovery sweep ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn expired_lease_is_reclaimed() {
        let auth = authority(8192);
        let res = Arc::new(ResidencyManager::new());
        let (m, _, _, _) = MockModel::new("llm", ConsumerId::Llm);
        res.register(m).await;
        let mgr = CoResidencyManager::with_policy(
            auth,
            res,
            CoResidencyPolicy {
                lease_ttl: Duration::from_millis(1),
                ..Default::default()
            },
        );
        let lease = mgr
            .acquire(
                &req(ConsumerId::Llm, PriorityClass::InteractiveFg, 4000, "llm"),
                ResidencyTarget::Hot,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let reclaimed = mgr.reclaim_expired().await;
        assert_eq!(reclaimed, 1);
        assert!(!lease.is_valid(), "reclaimed lease is revoked");
        assert_eq!(mgr.resident_count().await, 0);
    }

    // ── Concurrency / stress ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_acquire_release_no_overcommit_no_panic() {
        // Many concurrent consumers contend for a small GPU. Invariant: reserved VRAM never exceeds
        // the device's hard-admissible capacity, and the manager never deadlocks/panics.
        let auth = authority(12288);
        let res = Arc::new(ResidencyManager::new());
        for i in 0..6 {
            let (m, _, _, _) = MockModel::new(&format!("m{i}"), ConsumerId::Image);
            res.register(m).await;
        }
        let mgr = CoResidencyManager::new(auth.clone(), res);

        let mut handles = Vec::new();
        for i in 0..6 {
            let mgr = mgr.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..50 {
                    let model = format!("m{}", i % 6);
                    let class = if i % 3 == 0 {
                        PriorityClass::InteractiveFg
                    } else {
                        PriorityClass::Batch
                    };
                    if let Ok(l) = mgr
                        .acquire(
                            &req(ConsumerId::Image, class, 3000, &model),
                            ResidencyTarget::Hot,
                        )
                        .await
                    {
                        tokio::task::yield_now().await;
                        l.release().await;
                    }
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // After all work, reservations drain to zero (no leak).
        tokio::time::sleep(Duration::from_millis(50)).await;
        let reserved = auth.with_table_for_compare(|t| {
            t.get(&crate::resource::authority::types::DeviceId::Gpu(0))
                .unwrap()
                .reserved_vram_mb
        });
        assert_eq!(reserved, 0, "all reservations released — no leak");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn chaos_randomized_acquire_release_holds_invariants() {
        // Randomized churn: mixed classes, sizes, holds, drops, and a periodic reclaim sweep. The
        // device hard limit must never be over-committed and the manager must never deadlock.
        let auth = authority(16384);
        let res = Arc::new(ResidencyManager::new());
        for i in 0..8 {
            let (m, _, _, _) = MockModel::new(&format!("c{i}"), ConsumerId::Image);
            res.register(m).await;
        }
        let mgr = CoResidencyManager::with_policy(
            auth.clone(),
            res,
            CoResidencyPolicy {
                pin_dwell: Duration::from_millis(2),
                ..Default::default()
            },
        );

        let device = crate::resource::authority::types::DeviceId::Gpu(0);
        let total = 16384u64;

        let mut handles = Vec::new();
        for t in 0..8u64 {
            let mgr = mgr.clone();
            let auth = auth.clone();
            let device = device.clone();
            handles.push(tokio::spawn(async move {
                let mut rng = t.wrapping_mul(2654435761) ^ 0x9E37;
                let mut held: Vec<CoResidencyLease> = Vec::new();
                for _ in 0..120 {
                    rng = rng
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let r = (rng >> 33) as u32;
                    let vram = 1000 + (r % 5) as u64 * 1000;
                    let class = match r % 4 {
                        0 => PriorityClass::InteractiveFg,
                        1 => PriorityClass::RealtimeVoice,
                        2 => PriorityClass::InteractiveBg,
                        _ => PriorityClass::Batch,
                    };
                    let model = format!("c{}", r % 8);
                    if r % 3 == 0 && !held.is_empty() {
                        held.pop(); // drop a lease (Drop releases)
                    } else if let Ok(l) = mgr
                        .acquire(
                            &req(ConsumerId::Image, class, vram, &model),
                            ResidencyTarget::Hot,
                        )
                        .await
                    {
                        held.push(l);
                    }
                    // Invariant check: never over-committed beyond physical total.
                    let reserved = auth
                        .with_table_for_compare(|tbl| tbl.get(&device).unwrap().reserved_vram_mb);
                    assert!(reserved <= total, "over-commit: {reserved} > {total}");
                    if r % 7 == 0 {
                        let _ = mgr.reclaim_expired().await;
                    }
                    tokio::task::yield_now().await;
                }
                drop(held);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Final drain.
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = mgr.reclaim_expired().await;
    }

    #[tokio::test]
    async fn unregistered_model_is_pure_reservation() {
        // A consumer not registered with the residency manager (manages its own process) still gets
        // a valid lease (admission-only), and it is released cleanly.
        let auth = authority(8192);
        let res = Arc::new(ResidencyManager::new());
        let mgr = CoResidencyManager::new(auth, res);
        let l = mgr
            .acquire(
                &req(ConsumerId::Ext, PriorityClass::Batch, 2000, "ext-thing"),
                ResidencyTarget::Hot,
            )
            .await
            .expect("admission-only grant");
        assert!(l.is_valid());
        l.release().await;
        assert_eq!(mgr.resident_count().await, 0);
    }
}
