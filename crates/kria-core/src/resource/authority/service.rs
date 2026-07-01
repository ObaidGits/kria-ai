//! HraService — the single runtime integration object (HRA assembly: Tasks 3/10/36/37 + bypass).
//!
//! Bundles the control plane (`LocalAuthority`), the residency executor (`ResidencyManager`), the
//! telemetry apply path (`HostSnapshot`), SLA evaluation, low-cardinality metrics, the shadow
//! comparator, and the daemon supervisors behind one façade the desktop/server call. It is the
//! intended entry point for consumer cutover: a consumer calls `request`/`release` here instead of
//! constructing its own lease manager.
//!
//! Designed to run in SHADOW first (record decisions + compare to legacy without gating), then flip
//! per-consumer via the bypass switch once the shadow comparator is clean.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::collector::HostSnapshot;
use super::co_residency::{CoResidencyManager, CoResidencyMetrics};
use super::metrics::Counters;
use super::planner::PolicyProfile;
use super::predict::{Forecast, Forecaster, ResourceKind};
use super::ra::{LocalAuthority, RaOutcome, ResourceAuthority};
use super::residency_manager::ResidencyManager;
use super::scheduler::LeaseToken;
use super::shadow::{self, ShadowReport};
use super::sla::{SlaState, SlaTable};
use super::types::{ConsumerId, ResourceRequest};

/// HRA verdict on GPU placement for a consumer (see `HraService::advise_gpu_admission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAdmissionAdvice {
    /// Whether the consumer should be placed on GPU now.
    pub allow_gpu: bool,
    /// True when HRA is in shadow mode (advisory only — caller logs but does not honor).
    pub shadow: bool,
    /// Human-readable explanation (logged + shown in diagnostics).
    pub reason: String,
}

/// Process-wide HRA handle so legacy components (e.g. the GPU watchdog) can consult the authority
/// without a constructor dependency. Set once at startup via `set_global_hra`.
static GLOBAL_HRA: OnceLock<Arc<HraService>> = OnceLock::new();

/// Register the process-wide HRA service (idempotent — first set wins).
pub fn set_global_hra(svc: Arc<HraService>) {
    let _ = GLOBAL_HRA.set(svc);
}

/// Get the process-wide HRA service, if registered.
pub fn global_hra() -> Option<Arc<HraService>> {
    GLOBAL_HRA.get().cloned()
}

pub struct HraService {
    authority: Arc<LocalAuthority>,
    residency: Arc<ResidencyManager>,
    /// The Co-Residency GPU Lease Manager (HRA Phase B): the production residency authority that
    /// lets multiple models share a GPU under the VRAM budget with priority preemption. Built over
    /// the same single `authority` + `residency` (no duplicate scheduler/executor).
    co_residency: Arc<CoResidencyManager>,
    sla: SlaTable,
    gpu_safety_mb: u64,
    profile: PolicyProfile,
    metrics: Mutex<Counters>,
    shadow: Mutex<ShadowReport>,
    /// Live VRAM-exhaustion forecaster, fed by `apply_snapshot` (HRA Phase 4 — Forecasting view).
    vram_forecaster: Mutex<Forecaster>,
    last_forecast: Mutex<Option<Forecast>>,
    /// When true, `request` only records a shadow comparison and metrics; it does NOT gate (the
    /// legacy path still runs). Flipped per-consumer (via bypass) / globally after shadow
    /// validation. Interior-mutable so it can be toggled at runtime through an `Arc`.
    shadow_only: AtomicBool,
}

impl HraService {
    /// Build from device specs. `gpus` = (index, total_vram_mb).
    pub fn new(
        gpus: &[(u32, u64)],
        gpu_safety_mb: u64,
        cpu_ram_mb: u64,
        cloud_pools: &[&str],
        profile: PolicyProfile,
    ) -> Arc<Self> {
        let authority = Arc::new(LocalAuthority::bootstrap(
            gpus,
            gpu_safety_mb,
            cpu_ram_mb,
            cloud_pools,
            profile,
        ));
        let residency = Arc::new(ResidencyManager::new());
        let co_residency = CoResidencyManager::new(authority.clone(), residency.clone());
        Arc::new(Self {
            authority,
            residency,
            co_residency,
            sla: SlaTable::defaults(),
            gpu_safety_mb,
            profile,
            metrics: Mutex::new(Counters::default()),
            shadow: Mutex::new(ShadowReport::default()),
            vram_forecaster: Mutex::new(Forecaster::new(ResourceKind::Vram)),
            last_forecast: Mutex::new(None),
            shadow_only: AtomicBool::new(true),
        })
    }

    /// Build with a durable journal store (HRA Phase D1). The authority loads/replays the journal
    /// on boot and persists after every mutation, so the Reconciler can reclaim orphan residency
    /// after a crash. `journal_path` is created if missing.
    pub fn new_persisted(
        gpus: &[(u32, u64)],
        gpu_safety_mb: u64,
        cpu_ram_mb: u64,
        cloud_pools: &[&str],
        profile: PolicyProfile,
        journal_path: std::path::PathBuf,
    ) -> Arc<Self> {
        let store = super::journal_store::JournalStore::new(journal_path);
        let authority = Arc::new(LocalAuthority::bootstrap_persisted(
            gpus,
            gpu_safety_mb,
            cpu_ram_mb,
            cloud_pools,
            profile,
            store,
        ));
        let residency = Arc::new(ResidencyManager::new());
        let co_residency = CoResidencyManager::new(authority.clone(), residency.clone());
        Arc::new(Self {
            authority,
            residency,
            co_residency,
            sla: SlaTable::defaults(),
            gpu_safety_mb,
            profile,
            metrics: Mutex::new(Counters::default()),
            shadow: Mutex::new(ShadowReport::default()),
            vram_forecaster: Mutex::new(Forecaster::new(ResourceKind::Vram)),
            last_forecast: Mutex::new(None),
            shadow_only: AtomicBool::new(true),
        })
    }

    pub fn authority(&self) -> Arc<LocalAuthority> {
        self.authority.clone()
    }

    pub fn residency(&self) -> Arc<ResidencyManager> {
        self.residency.clone()
    }

    /// The Co-Residency GPU Lease Manager (HRA Phase B). Consumers acquire GPU residency through
    /// this (when enforcing) instead of the legacy single-holder lease.
    pub fn co_residency(&self) -> Arc<CoResidencyManager> {
        self.co_residency.clone()
    }

    pub fn co_residency_metrics(&self) -> CoResidencyMetrics {
        self.co_residency.metrics()
    }

    /// Unified GPU admission gateway (HRA Phase 1 — consumer cutover entry point).
    ///
    /// This is the SINGLE call every GPU consumer (LLM, image, voice, vision, embeddings, tools…)
    /// uses to ask HRA for permission before touching the GPU. Its behavior is governed by the
    /// enforce flag so the cutover is rollback-safe:
    /// - **Shadow (default)**: returns [`AdmissionGuard::Shadow`] immediately, touching no authority
    ///   state — the consumer proceeds on its existing (legacy) path. Byte-for-behavior unchanged.
    /// - **Enforcing** (`KRIA_HRA_ENFORCE=1`): routes through the Co-Residency manager, returning a
    ///   real [`CoResidencyLease`] (or an error the consumer maps to CPU/cloud fallback). HRA now
    ///   owns the residency decision.
    ///
    /// Consumers hold the returned guard for the duration of their GPU work and check
    /// [`AdmissionGuard::is_valid`] at checkpoints (cooperative preemption).
    pub async fn admit_gpu(
        &self,
        req: &ResourceRequest,
        target: super::co_residency::ResidencyTarget,
    ) -> Result<AdmissionGuard, super::co_residency::CoResidencyError> {
        if self.is_shadow_only() {
            return Ok(AdmissionGuard::Shadow);
        }
        let lease = self.co_residency.acquire(req, target).await?;
        Ok(AdmissionGuard::Granted(lease))
    }

    pub fn set_shadow_only(&self, shadow_only: bool) {
        self.shadow_only.store(shadow_only, Ordering::Release);
    }

    pub fn is_shadow_only(&self) -> bool {
        self.shadow_only.load(Ordering::Acquire)
    }

    /// Apply a fresh telemetry snapshot to the device table (collector → RA).
    pub fn apply_snapshot(&self, snap: &HostSnapshot) {
        self.authority.apply_snapshot(snap, self.gpu_safety_mb);
        // Feed the live VRAM-exhaustion forecaster (Phase 4). Threshold 0 MB free = exhaustion.
        if let Some(g) = snap.gpus.first() {
            let mut f = self.vram_forecaster.lock().unwrap();
            // Snapshots arrive on the hub cadence (~5s); use it as dt.
            let fc = f.observe(g.free_vram_mb as f64, 5.0, 0.0);
            *self.last_forecast.lock().unwrap() = Some(fc);
        }
    }

    /// Latest VRAM forecast as JSON for the dashboard Forecasting view.
    pub fn forecast_json(&self) -> serde_json::Value {
        match *self.last_forecast.lock().unwrap() {
            Some(fc) => serde_json::json!({
                "resource": "vram",
                "time_to_exhaustion_s": fc.time_to_threshold_s,
                "confidence": fc.confidence,
            }),
            None => serde_json::json!({ "resource": "vram", "time_to_exhaustion_s": null, "confidence": 0.0 }),
        }
    }

    /// Per-consumer bypass kill-switch (Task 35).
    pub fn set_bypass(&self, consumer: ConsumerId, on: bool) {
        self.authority.set_bypass(consumer, on);
    }

    /// Request resources. Always records a shadow comparison + metrics. In shadow-only mode the
    /// returned outcome is advisory (the caller keeps using the legacy path); once flipped, the
    /// caller honors the outcome.
    pub fn request(&self, req: &ResourceRequest) -> RaOutcome {
        // Shadow comparison against the legacy static plan (cutover gate signal).
        {
            // Borrow the table read-only via a transient compare on a cloned snapshot is not
            // available; compare uses the live table inside the authority.
            self.authority.with_table_for_compare(|table| {
                let d = shadow::compare(req, table, self.profile);
                self.shadow.lock().unwrap().record(d);
            });
        }

        let outcome = self.authority.request(req);

        // Metrics (low-cardinality).
        {
            let mut m = self.metrics.lock().unwrap();
            match &outcome {
                RaOutcome::Granted(_) => m.admissions_granted += 1,
                RaOutcome::Busy => m.admissions_busy += 1,
                RaOutcome::Shed => m.admissions_shed += 1,
                RaOutcome::PreemptThenRetry { .. } => m.preemptions += 1,
            }
        }
        outcome
    }

    pub fn release(&self, token: LeaseToken) {
        self.authority.release(token);
    }

    /// Evaluate an operation latency against its SLA (Task 47).
    pub fn sla_eval(&self, op: &str, measured_ms: u32) -> SlaState {
        self.sla.evaluate(op, measured_ms)
    }

    pub fn metrics(&self) -> Counters {
        self.metrics.lock().unwrap().clone()
    }

    /// HRA verdict on whether the LLM should be placed on GPU right now, based on the live
    /// DeviceTable (fed by the telemetry snapshot loop). In SHADOW mode the caller only logs this;
    /// when enforcing, the caller honors `allow_gpu` (vetoing an unsafe GPU scale-up). This is the
    /// real "HRA commands the legacy executor" hook.
    pub fn advise_gpu_admission(&self, needed_vram_mb: u64) -> GpuAdmissionAdvice {
        let shadow = self.is_shadow_only();
        self.authority.with_table_for_compare(|table| {
            let gpus = table.usable_gpus();
            if let Some(d) = gpus.iter().find(|d| d.can_admit_vram(needed_vram_mb)) {
                GpuAdmissionAdvice {
                    allow_gpu: true,
                    shadow,
                    reason: format!(
                        "fits {:?}: free {} MB ≥ need {} MB (after safety/hard band)",
                        d.id,
                        d.effective_free_vram_mb(),
                        needed_vram_mb
                    ),
                }
            } else {
                let best_free = gpus.iter().map(|d| d.effective_free_vram_mb()).max().unwrap_or(0);
                GpuAdmissionAdvice {
                    allow_gpu: false,
                    shadow,
                    reason: format!(
                        "no usable GPU can admit {} MB (best effective free {} MB); recommend CPU",
                        needed_vram_mb, best_free
                    ),
                }
            }
        })
    }

    /// Like [`advise_gpu_admission`](Self::advise_gpu_admission) but takes a guaranteed-fresh
    /// telemetry reading first (HRA Phase A2). The 15s snapshot loop is fine for the dashboard, but
    /// an admission/scale-up veto must not decide on stale VRAM. When the global telemetry hub is
    /// present this samples the device synchronously-fresh, applies it to the DeviceTable, then
    /// renders the verdict on current data. Falls back to the last-applied snapshot if no hub.
    pub async fn advise_gpu_admission_fresh(&self, needed_vram_mb: u64) -> GpuAdmissionAdvice {
        if let Some(hub) = crate::resource::telemetry_hub::global_telemetry_hub() {
            let snap = hub.sample_now().await;
            self.apply_snapshot(&snap);
        }
        self.advise_gpu_admission(needed_vram_mb)
    }

    /// Async diagnostics including the live co-resident snapshot (which requires an async lock).
    /// Use this from async contexts (event loop, command); `diagnostics_json` is the sync subset.
    pub async fn diagnostics_json_async(&self) -> serde_json::Value {
        let mut base = self.diagnostics_json();
        let residents = self.co_residency.residents_snapshot().await;
        let residents_json: Vec<serde_json::Value> = residents
            .iter()
            .map(|r| {
                serde_json::json!({
                    "model": r.model,
                    "class": r.class.as_str(),
                    "device": r.device,
                    "age_ms": r.age_ms,
                    "refs": r.refs,
                    "pinned": r.pinned,
                })
            })
            .collect();
        if let Some(obj) = base.as_object_mut() {
            obj.insert("residents".to_string(), serde_json::Value::Array(residents_json));
        }
        base
    }

    pub fn shadow_gate_passes(&self) -> bool {
        self.shadow.lock().unwrap().gate_passes()
    }

    /// JSON status for the frontend Resource Dashboard / Diagnostics.
    pub fn status_json(&self) -> serde_json::Value {
        let m = self.metrics();
        serde_json::json!({
            "epoch": self.authority.current_epoch().0,
            "shadow_only": self.is_shadow_only(),
            "enforcing": !self.is_shadow_only(),
            "shadow_gate_passes": self.shadow_gate_passes(),
            "metrics": {
                "granted": m.admissions_granted,
                "busy": m.admissions_busy,
                "shed": m.admissions_shed,
                "preemptions": m.preemptions,
                "swaps": m.swaps,
                "oom_events": m.oom_events,
                "foreground_invariant_ok": m.foreground_invariant_ok(),
            },
        })
    }

    /// Full diagnostics bundle for the frontend Recovery/Explainability/Diagnostics views and the
    /// backend export (HRA Phase E2/E4). Code-grounded: every field comes from live state, so the
    /// dashboard can drop its "awaiting data" placeholders once this is consumed.
    pub fn diagnostics_json(&self) -> serde_json::Value {
        // Devices (authoritative table view).
        let devices = self.authority.with_table_for_compare(|table| {
            table
                .usable_gpus()
                .into_iter()
                .map(|d| {
                    serde_json::json!({
                        "id": format!("{:?}", d.id),
                        "total_vram_mb": d.total.vram_mb,
                        "free_vram_mb": d.free_vram_mb,
                        "reserved_vram_mb": d.reserved_vram_mb,
                        "effective_free_vram_mb": d.effective_free_vram_mb(),
                        "soft_limit_mb": d.budget.soft_mb,
                        "hard_limit_mb": d.budget.hard_mb,
                        "emergency_limit_mb": d.budget.emergency_mb,
                        "health": format!("{:?}", d.health),
                        "breaker": format!("{:?}", d.breaker),
                    })
                })
                .collect::<Vec<_>>()
        });

        // Telemetry freshness (single hub).
        let telemetry = match crate::resource::telemetry_hub::global_telemetry_hub() {
            Some(hub) => {
                let snap = hub.latest();
                let cores = &snap.cpu.per_core_pct;
                let cpu_avg = if cores.is_empty() {
                    0u32
                } else {
                    (cores.iter().map(|c| *c as u32).sum::<u32>()) / cores.len() as u32
                };
                serde_json::json!({
                    "seq": snap.seq,
                    "gpu_count": snap.gpus.len(),
                    "ram_free_mb": snap.ram.free_mb,
                    "ram_total_mb": snap.ram.total_mb,
                    "cpu_cores": cores.len(),
                    "cpu_avg_pct": cpu_avg,
                    "cpu_per_core_pct": cores,
                    "source": "unified_hub",
                })
            }
            None => serde_json::json!({ "source": "none" }),
        };

        // Recovered open leases (crash recovery, Phase D1).
        let recovered = self.authority.recovered_open_leases();
        let recovered_json = recovered
            .iter()
            .map(|(token, dev, vram)| {
                serde_json::json!({ "token": token, "device": format!("{:?}", dev), "vram_mb": vram })
            })
            .collect::<Vec<_>>();

        // SLA snapshot for the standard operations (thresholds only — live measurements are emitted
        // per-op via sla_eval at call sites).
        let sla = serde_json::json!({
            "voice.wake": format!("{:?}", self.sla.evaluate("voice.wake", 0)),
            "configured": true,
        });

        // Co-residency manager metrics (Phase B).
        let m = self.co_residency.metrics();
        let co_residency = serde_json::json!({
            "preemptions": m.preemptions,
            "rollbacks": m.rollbacks,
            "dedup_hits": m.dedup_hits,
        });

        // Explainability: recent decisions from the journal with human rationale (Phase 4).
        let decisions = self.authority.recent_decisions(20);

        serde_json::json!({
            "status": self.status_json(),
            "devices": devices,
            "telemetry": telemetry,
            "recovered_open_leases": recovered_json,
            "sla": sla,
            "co_residency": co_residency,
            "decisions": decisions,
            "forecast": self.forecast_json(),
            "profile": format!("{:?}", self.profile),
        })
    }
}

/// Result of [`HraService::admit_gpu`]. In shadow mode it is an inert pass (the consumer keeps its
/// legacy path); under enforce it carries the real co-residency lease.
pub enum AdmissionGuard {
    /// HRA is not enforcing — the consumer proceeds on its existing path. No-op.
    Shadow,
    /// HRA granted and owns this GPU residency. Drop to release.
    Granted(super::co_residency::CoResidencyLease),
}

impl AdmissionGuard {
    /// Whether HRA is actively enforcing this admission (vs an inert shadow pass).
    pub fn is_enforced(&self) -> bool {
        matches!(self, AdmissionGuard::Granted(_))
    }

    /// Whether the residency is still valid. Always true in shadow (legacy owns it); in enforce it
    /// reflects the lease's cooperative-revocation state — a consumer must yield GPU work when false.
    pub fn is_valid(&self) -> bool {
        match self {
            AdmissionGuard::Shadow => true,
            AdmissionGuard::Granted(l) => l.is_valid(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{Constraints, ConsumerId, PriorityClass, ResourceNeed, TurnId};
    use super::*;

    fn svc() -> Arc<HraService> {
        HraService::new(&[(0, 12288)], 512, 32768, &["openai"], PolicyProfile::Balanced)
    }

    fn req(vram: u64) -> ResourceRequest {
        ResourceRequest {
            consumer: ConsumerId::Llm,
            class: PriorityClass::InteractiveFg,
            need: ResourceNeed {
                vram_mb: vram,
                ram_mb: 2048,
                cpu_threads: 4,
                exclusivity: false,
                model_id: Some("m".into()),
                est_ms: 1000,
            },
            constraints: Constraints::default(),
            turn_id: TurnId("t".into()),
        }
    }

    #[test]
    fn request_records_metrics_and_shadow() {
        let s = svc();
        let _ = s.request(&req(4000));
        let m = s.metrics();
        assert_eq!(m.admissions_granted, 1);
        assert!(s.shadow_gate_passes());
    }

    #[test]
    fn sla_eval_works() {
        let s = svc();
        assert_eq!(s.sla_eval("voice.wake", 100), SlaState::Ok);
        assert_eq!(s.sla_eval("voice.wake", 700), SlaState::Critical);
    }

    #[test]
    fn status_json_has_fields() {
        let s = svc();
        let _ = s.request(&req(4000));
        let j = s.status_json();
        assert_eq!(j["epoch"], 1);
        assert_eq!(j["metrics"]["granted"], 1);
        assert_eq!(j["shadow_gate_passes"], true);
    }

    #[test]
    fn starts_in_shadow_only() {
        assert!(svc().is_shadow_only());
    }

    #[tokio::test]
    async fn admit_gpu_is_inert_passthrough_in_shadow() {
        let s = svc(); // shadow by default
        let g = s.admit_gpu(&req(4000), super::super::co_residency::ResidencyTarget::Hot).await.unwrap();
        assert!(!g.is_enforced(), "shadow admission must be inert");
        assert!(g.is_valid());
        // No co-resident state was created in shadow.
        assert_eq!(s.co_residency().resident_count().await, 0);
    }

    #[tokio::test]
    async fn admit_gpu_grants_through_co_residency_when_enforcing() {
        let s = svc();
        s.set_shadow_only(false);
        let g = s.admit_gpu(&req(4000), super::super::co_residency::ResidencyTarget::Hot).await.unwrap();
        assert!(g.is_enforced(), "enforcing admission routes through co-residency");
        assert!(g.is_valid());
        assert_eq!(s.co_residency().resident_count().await, 1);
    }

    #[test]
    fn advise_gpu_admission_reflects_capacity() {
        let s = svc(); // one 12 GB GPU (bootstrap sets free = total)
        let small = s.advise_gpu_admission(2000);
        assert!(small.allow_gpu, "{}", small.reason);
        let huge = s.advise_gpu_admission(50_000);
        assert!(!huge.allow_gpu);
        assert!(huge.reason.contains("CPU") || huge.reason.contains("no usable GPU"));
        assert!(small.shadow);
    }
}
