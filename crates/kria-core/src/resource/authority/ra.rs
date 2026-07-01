//! Resource Authority assembly (HRA Task 10) + transport-agnostic trait (Task 39) + bypass
//! kill-switch (Task 35).
//!
//! `LocalAuthority` ties together the DeviceTable, Planner, Scheduler, Journal, and Pressure
//! Engine into the single control-plane grantor. It is exposed behind the `ResourceAuthority`
//! trait so a future remote/gRPC authority can be dropped in without changing consumers
//! (R23.3 distributed-readiness). All decisions are deterministic — no LLM (R13).

use std::collections::HashSet;
use std::sync::Mutex;

use super::device_table::DeviceTable;
use super::journal::{DecisionKind, Journal};
use super::journal_store::JournalStore;
use super::planner::{self, PolicyProfile};
use super::scheduler::{AdmitError, Lease, LeaseToken, Scheduler};
use super::types::{ConsumerId, DeviceId, Epoch, Plan, ResourceRequest, Residency, Capacity};

/// Result of a resource request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaOutcome {
    /// A lease was granted on the planned (or fallback) device.
    Granted(Lease),
    /// A higher-priority request needs the runtime to preempt this victim, then retry.
    PreemptThenRetry { victim: LeaseToken },
    /// Device contended by an equal-or-higher class; caller should wait/retry.
    Busy,
    /// Request shed due to overload.
    Shed,
}

/// Transport-agnostic authority contract (R23.3). One local impl today; a remote impl later.
pub trait ResourceAuthority: Send + Sync {
    fn request(&self, req: &ResourceRequest) -> RaOutcome;
    fn release(&self, token: LeaseToken);
    fn current_epoch(&self) -> Epoch;
    /// Whether a consumer is in RA-bypass mode (static direct plan, no authority) — Task 35.
    fn is_bypassed(&self, consumer: ConsumerId) -> bool;
}

pub struct LocalAuthority {
    table: Mutex<DeviceTable>,
    scheduler: Mutex<Scheduler>,
    journal: Mutex<Journal>,
    profile: Mutex<PolicyProfile>,
    /// Consumers routed around the authority (bypass kill-switch, Task 35).
    bypass: Mutex<HashSet<ConsumerId>>,
    /// Optional durable backing for the journal (HRA Phase D1). When set, the journal is persisted
    /// (atomic + fsync) after every mutation so the Reconciler can replay leases on boot. `None`
    /// keeps the authority fully in-memory (tests / shadow-only contexts) with zero IO.
    store: Option<JournalStore>,
}

impl LocalAuthority {
    /// Build with an initial device table. Bumps the epoch on construction (a fresh authority is a
    /// fresh epoch — pre-existing leases from a prior instance are fenced, R21.1).
    pub fn new(table: DeviceTable, profile: PolicyProfile) -> Self {
        let mut journal = Journal::new();
        let epoch = journal.bump_epoch(super::types::TurnId("boot".into()), 0);
        Self {
            table: Mutex::new(table),
            scheduler: Mutex::new(Scheduler::new(epoch)),
            journal: Mutex::new(journal),
            profile: Mutex::new(profile),
            bypass: Mutex::new(HashSet::new()),
            store: None,
        }
    }

    /// Build with a durable journal store (HRA Phase D1). Loads + replays any prior journal from
    /// disk (recovering crash state for the Reconciler), then bumps the epoch on top so leases from
    /// the previous instance are fenced. The post-boot journal is flushed immediately so the new
    /// epoch is durable. Falls back to a clean journal if the store is unreadable.
    pub fn new_persisted(table: DeviceTable, profile: PolicyProfile, store: JournalStore) -> Self {
        let (mut journal, truncated) = store.load();
        if truncated > 0 {
            tracing::warn!(
                target: "hra",
                truncated,
                path = %store.path().display(),
                "HRA journal recovery truncated a corrupt tail (last-good wins)"
            );
        }
        let epoch = journal.bump_epoch(super::types::TurnId("boot".into()), 0);
        if let Err(e) = store.save(&journal) {
            tracing::warn!(target: "hra", error = %e, "HRA journal initial flush failed (continuing in-memory)");
        }
        Self {
            table: Mutex::new(table),
            scheduler: Mutex::new(Scheduler::new(epoch)),
            journal: Mutex::new(journal),
            profile: Mutex::new(profile),
            bypass: Mutex::new(HashSet::new()),
            store: Some(store),
        }
    }

    /// Persist the current journal if a durable store is configured (best-effort; a flush failure
    /// is logged but never breaks admission). Caller holds the journal lock.
    fn flush_locked(&self, journal: &Journal) {
        if let Some(store) = &self.store {
            if let Err(e) = store.save(journal) {
                tracing::warn!(target: "hra", error = %e, "HRA journal flush failed");
            }
        }
    }

    /// Recover prior-instance lease records for the Reconciler: the device + vram of every lease
    /// that was granted and not subsequently released in the persisted journal. Empty when no
    /// store / clean boot. Used to detect orphan GPU residency after a crash (Phase D1/D2).
    pub fn recovered_open_leases(&self) -> Vec<(u64, DeviceId, u64)> {
        let j = self.journal.lock().unwrap();
        let mut open: std::collections::HashMap<u64, (DeviceId, u64)> = std::collections::HashMap::new();
        for rec in j.records() {
            match &rec.payload.kind {
                DecisionKind::LeaseGranted { token, device, vram_mb } => {
                    open.insert(*token, (device.clone(), *vram_mb));
                }
                DecisionKind::LeaseReleased { token } => {
                    open.remove(token);
                }
                _ => {}
            }
        }
        open.into_iter().map(|(t, (d, v))| (t, d, v)).collect()
    }

    pub fn set_profile(&self, profile: PolicyProfile) {
        *self.profile.lock().unwrap() = profile;
    }

    pub fn set_bypass(&self, consumer: ConsumerId, on: bool) {
        let mut b = self.bypass.lock().unwrap();
        if on {
            b.insert(consumer);
        } else {
            b.remove(&consumer);
        }
    }

    /// Static direct plan used in bypass mode: full GPU0 if present, else CPU. No authority state.
    pub fn static_plan(&self, req: &ResourceRequest) -> Plan {
        let table = self.table.lock().unwrap();
        let gpu = table
            .usable_gpus()
            .into_iter()
            .find(|d| d.can_admit_vram(req.need.vram_mb))
            .map(|d| d.id.clone());
        match gpu {
            Some(dev) => Plan {
                device: dev,
                residency: Residency::VramHot,
                budget: Capacity::vram(req.need.vram_mb),
                fallback_chain: vec![],
                rationale: super::types::RationaleCode::FitsLocal,
            },
            None => Plan {
                device: DeviceId::Cpu,
                residency: Residency::RamWarm,
                budget: Capacity {
                    ram_mb: req.need.ram_mb,
                    ..Default::default()
                },
                fallback_chain: vec![],
                rationale: super::types::RationaleCode::FailOpenCpu,
            },
        }
    }

    /// Refresh a device's live free figures (runtime feeds telemetry here).
    pub fn refresh_free(&self, id: &DeviceId, free_vram_mb: u64, free_ram_mb: u64) {
        self.table
            .lock()
            .unwrap()
            .refresh_free(id, free_vram_mb, free_ram_mb);
    }

    /// Apply a full host snapshot (multi-device) from the TelemetryCollector (Task 3 integration).
    pub fn apply_snapshot(&self, snap: &super::collector::HostSnapshot, gpu_safety_mb: u64) {
        let mut t = self.table.lock().unwrap();
        snap.apply_to(&mut t, gpu_safety_mb);
    }

    /// Bootstrap an authority from device specs: `gpus` = (index, total_vram_mb), plus CPU RAM and
    /// optional cloud pool names. Convenience entry point for runtime wiring (Task 10/Task 3).
    pub fn bootstrap(
        gpus: &[(u32, u64)],
        gpu_safety_mb: u64,
        cpu_ram_mb: u64,
        cloud_pools: &[&str],
        profile: PolicyProfile,
    ) -> Self {
        let mut table = DeviceTable::new();
        for (idx, vram) in gpus {
            table.upsert(super::device_table::DeviceRecord::gpu(*idx, *vram, gpu_safety_mb));
        }
        table.upsert(super::device_table::DeviceRecord::cpu(cpu_ram_mb));
        for pool in cloud_pools {
            table.upsert(super::device_table::DeviceRecord::cloud(*pool));
        }
        Self::new(table, profile)
    }

    /// Like [`bootstrap`](Self::bootstrap) but durable: the journal is loaded/replayed from `store`
    /// on boot and persisted after every mutation (HRA Phase D1).
    pub fn bootstrap_persisted(
        gpus: &[(u32, u64)],
        gpu_safety_mb: u64,
        cpu_ram_mb: u64,
        cloud_pools: &[&str],
        profile: PolicyProfile,
        store: JournalStore,
    ) -> Self {
        let mut table = DeviceTable::new();
        for (idx, vram) in gpus {
            table.upsert(super::device_table::DeviceRecord::gpu(*idx, *vram, gpu_safety_mb));
        }
        table.upsert(super::device_table::DeviceRecord::cpu(cpu_ram_mb));
        for pool in cloud_pools {
            table.upsert(super::device_table::DeviceRecord::cloud(*pool));
        }
        Self::new_persisted(table, profile, store)
    }

    /// Read-only journal length (diagnostics/tests).
    pub fn journal_len(&self) -> usize {
        self.journal.lock().unwrap().records().len()
    }

    /// Recent decisions from the journal (newest last), rendered for the Explainability UI. Each
    /// entry carries a human-readable rationale so the dashboard can answer "why" without the user
    /// reading raw codes (HRA Phase 4 / R9.2).
    pub fn recent_decisions(&self, n: usize) -> Vec<serde_json::Value> {
        let j = self.journal.lock().unwrap();
        let recs = j.records();
        let start = recs.len().saturating_sub(n);
        recs[start..]
            .iter()
            .map(|r| {
                let (kind, detail, why) = match &r.payload.kind {
                    DecisionKind::EpochBump { to } => (
                        "epoch_bump",
                        format!("epoch → {to}"),
                        "Authority (re)started — prior leases fenced for split-brain safety.".to_string(),
                    ),
                    DecisionKind::Planned { device, rationale } => (
                        "planned",
                        format!("{device:?}"),
                        rationale.human().to_string(),
                    ),
                    DecisionKind::LeaseGranted { token, device, vram_mb } => (
                        "granted",
                        format!("#{token} {device:?} {vram_mb} MB"),
                        "Admitted — fit within the device VRAM budget.".to_string(),
                    ),
                    DecisionKind::LeaseReleased { token } => (
                        "released",
                        format!("#{token}"),
                        "Lease released — reservation returned to the device.".to_string(),
                    ),
                    DecisionKind::Preempted { victim_token, reason } => (
                        "preempted",
                        format!("#{victim_token}"),
                        format!("Evicted a lower-priority resident to make room: {reason}"),
                    ),
                    DecisionKind::Evicted { model } => (
                        "evicted",
                        model.clone(),
                        "Model cooled to RAM to free VRAM.".to_string(),
                    ),
                    DecisionKind::Failover { pool } => (
                        "failover",
                        pool.clone(),
                        "Routed to cloud — local capacity insufficient.".to_string(),
                    ),
                    DecisionKind::SimulateReject { rationale } => (
                        "simulate_reject",
                        String::new(),
                        rationale.human().to_string(),
                    ),
                };
                serde_json::json!({
                    "seq": r.payload.seq,
                    "turn_id": r.payload.turn_id.0,
                    "kind": kind,
                    "detail": detail,
                    "why": why,
                })
            })
            .collect()
    }

    /// Run a closure against the live device table (read-only) — used by the shadow comparator.
    pub fn with_table_for_compare<R>(&self, f: impl FnOnce(&DeviceTable) -> R) -> R {
        let t = self.table.lock().unwrap();
        f(&t)
    }

    /// GPU-residency admission with NO CPU/cloud fallback (HRA Phase B — co-residency).
    ///
    /// The default [`request`](ResourceAuthority::request) walks the planner's fallback chain (and
    /// the planner itself may pick CPU as the *primary* once the GPU is full), so a contended GPU
    /// silently lands the work on CPU and the preemption signal is lost. The Co-Residency
    /// Coordinator needs the raw GPU verdict, so this targets the GPU(s) directly: it tries each
    /// usable GPU (most-free first) and returns `Granted` (fit, possibly alongside other residents),
    /// `PreemptThenRetry` (a strictly-lower holder must be evicted first on some GPU), or `Busy`
    /// (no GPU, or all GPUs held by equal/higher class — do not preempt). The caller decides on CPU
    /// degradation only after GPU options are exhausted.
    pub fn request_on_gpu(&self, req: &ResourceRequest) -> RaOutcome {
        if self.is_bypassed(req.consumer) {
            return self.admit_plan(req, self.static_plan(req));
        }
        // Candidate GPUs, most effective-free first (best fit / least disruptive).
        let gpus: Vec<DeviceId> = {
            let table = self.table.lock().unwrap();
            table.usable_gpus().into_iter().map(|d| d.id.clone()).collect()
        };
        if gpus.is_empty() {
            return RaOutcome::Busy; // no GPU — caller falls back to CPU/cloud
        }
        let mut saw_preempt: Option<RaOutcome> = None;
        for device in gpus {
            let plan = Plan {
                device,
                residency: Residency::VramHot,
                budget: Capacity::vram(req.need.vram_mb),
                fallback_chain: vec![],
                rationale: super::types::RationaleCode::CoResident,
            };
            match self.admit_plan(req, plan) {
                RaOutcome::Granted(l) => return RaOutcome::Granted(l),
                pre @ RaOutcome::PreemptThenRetry { .. } => {
                    // Remember the first preemptable GPU but keep scanning for a clean fit.
                    if saw_preempt.is_none() {
                        saw_preempt = Some(pre);
                    }
                }
                RaOutcome::Shed => return RaOutcome::Shed,
                RaOutcome::Busy => {}
            }
        }
        saw_preempt.unwrap_or(RaOutcome::Busy)
    }

    /// Shared admission path: journal the plan, admit through the single scheduler, journal+flush
    /// the grant, and map the scheduler result to an [`RaOutcome`]. Used by both `request` (with
    /// fallback) and `request_no_fallback`.
    fn admit_plan(&self, req: &ResourceRequest, plan: Plan) -> RaOutcome {
        // Bypass: hand back a static plan as an immediate grant (no scheduler/journal).
        if self.is_bypassed(req.consumer) {
            let static_plan = self.static_plan(req);
            let mut table = self.table.lock().unwrap();
            let mut sched = self.scheduler.lock().unwrap();
            return match sched.admit(&mut table, req.class, req.turn_id.clone(), &static_plan) {
                Ok(lease) => RaOutcome::Granted(lease),
                Err(_) => RaOutcome::Busy,
            };
        }

        // Journal the plan decision (explainability correlation).
        {
            let mut j = self.journal.lock().unwrap();
            j.append(
                req.turn_id.clone(),
                DecisionKind::Planned {
                    device: plan.device.clone(),
                    rationale: plan.rationale,
                },
                0,
            );
        }

        let mut table = self.table.lock().unwrap();
        let mut sched = self.scheduler.lock().unwrap();
        match sched.admit(&mut table, req.class, req.turn_id.clone(), &plan) {
            Ok(lease) => {
                let mut j = self.journal.lock().unwrap();
                j.append(
                    req.turn_id.clone(),
                    DecisionKind::LeaseGranted {
                        token: lease.token.0,
                        device: lease.device.clone(),
                        vram_mb: lease.budget.vram_mb,
                    },
                    0,
                );
                self.flush_locked(&j);
                RaOutcome::Granted(lease)
            }
            Err(AdmitError::PreemptionRequired { victim }) => {
                RaOutcome::PreemptThenRetry { victim }
            }
            Err(AdmitError::Busy { .. }) => RaOutcome::Busy,
            Err(AdmitError::Shed) => RaOutcome::Shed,
            Err(AdmitError::NoCapacity) => RaOutcome::Busy,
        }
    }
}

impl ResourceAuthority for LocalAuthority {
    fn request(&self, req: &ResourceRequest) -> RaOutcome {
        let plan = {
            let table = self.table.lock().unwrap();
            let profile = *self.profile.lock().unwrap();
            planner::plan(req, &table, profile)
        };
        self.admit_plan(req, plan)
    }

    fn release(&self, token: LeaseToken) {
        let mut table = self.table.lock().unwrap();
        let mut sched = self.scheduler.lock().unwrap();
        sched.release(&mut table, token);
        let mut j = self.journal.lock().unwrap();
        j.append(
            super::types::TurnId("release".into()),
            DecisionKind::LeaseReleased { token: token.0 },
            0,
        );
        self.flush_locked(&j);
    }

    fn current_epoch(&self) -> Epoch {
        self.journal.lock().unwrap().current_epoch()
    }

    fn is_bypassed(&self, consumer: ConsumerId) -> bool {
        self.bypass.lock().unwrap().contains(&consumer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::device_table::{DeviceRecord, DeviceTable};
    use super::super::types::{
        Constraints, ConsumerId, PriorityClass, ResourceNeed, TurnId,
    };
    use super::*;

    fn table() -> DeviceTable {
        let mut t = DeviceTable::new();
        t.upsert(DeviceRecord::gpu(0, 12288, 512));
        t.upsert(DeviceRecord::cpu(32768));
        t.upsert(DeviceRecord::cloud("openai"));
        t
    }

    fn req(consumer: ConsumerId, vram_mb: u64) -> ResourceRequest {
        ResourceRequest {
            consumer,
            class: PriorityClass::InteractiveFg,
            need: ResourceNeed {
                vram_mb,
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
    fn fresh_authority_starts_at_epoch_1() {
        let ra = LocalAuthority::new(table(), PolicyProfile::Balanced);
        assert_eq!(ra.current_epoch(), Epoch(1));
    }

    #[test]
    fn request_grants_and_journals() {
        let ra = LocalAuthority::new(table(), PolicyProfile::Balanced);
        let before = ra.journal_len();
        let outcome = ra.request(&req(ConsumerId::Llm, 4000));
        match outcome {
            RaOutcome::Granted(l) => {
                assert_eq!(l.device, DeviceId::Gpu(0));
                assert_eq!(l.epoch, Epoch(1));
            }
            other => panic!("expected grant, got {other:?}"),
        }
        // plan + grant journaled.
        assert!(ra.journal_len() >= before + 2);
    }

    #[test]
    fn release_frees_capacity_for_next_request() {
        let ra = LocalAuthority::new(table(), PolicyProfile::Balanced);
        // Hog most of the GPU.
        let l1 = match ra.request(&req(ConsumerId::Llm, 11000)) {
            RaOutcome::Granted(l) => l,
            o => panic!("{o:?}"),
        };
        ra.release(l1.token);
        // Now a fresh large request fits again on GPU.
        match ra.request(&req(ConsumerId::Image, 9000)) {
            RaOutcome::Granted(l) => assert_eq!(l.device, DeviceId::Gpu(0)),
            o => panic!("expected grant after release, got {o:?}"),
        }
    }

    #[test]
    fn bypass_returns_static_plan_grant() {
        let ra = LocalAuthority::new(table(), PolicyProfile::Balanced);
        ra.set_bypass(ConsumerId::Llm, true);
        assert!(ra.is_bypassed(ConsumerId::Llm));
        match ra.request(&req(ConsumerId::Llm, 4000)) {
            RaOutcome::Granted(l) => assert_eq!(l.device, DeviceId::Gpu(0)),
            o => panic!("expected bypass grant, got {o:?}"),
        }
    }

    #[test]
    fn persisted_authority_recovers_open_leases_across_restart() {
        use super::super::journal_store::JournalStore;
        let mut path = std::env::temp_dir();
        path.push(format!("kria_ra_persist_{}.journal", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // Instance 1: grant a lease (do NOT release) then drop the authority.
        {
            let store = JournalStore::new(&path);
            let ra = LocalAuthority::new_persisted(table(), PolicyProfile::Balanced, store);
            match ra.request(&req(ConsumerId::Llm, 4000)) {
                RaOutcome::Granted(_) => {}
                o => panic!("expected grant, got {o:?}"),
            }
        }

        // Instance 2: a fresh authority backed by the same store must recover the still-open lease
        // (granted, never released) so the Reconciler can reclaim it. Epoch advances (fencing).
        {
            let store = JournalStore::new(&path);
            let ra = LocalAuthority::new_persisted(table(), PolicyProfile::Balanced, store);
            let open = ra.recovered_open_leases();
            assert_eq!(open.len(), 1, "expected one recovered open lease");
            assert_eq!(open[0].1, DeviceId::Gpu(0));
            assert_eq!(ra.current_epoch(), Epoch(2), "epoch must advance on restart (fencing)");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn released_lease_is_not_recovered_as_open() {
        use super::super::journal_store::JournalStore;
        let mut path = std::env::temp_dir();
        path.push(format!("kria_ra_persist_rel_{}.journal", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let store = JournalStore::new(&path);
            let ra = LocalAuthority::new_persisted(table(), PolicyProfile::Balanced, store);
            if let RaOutcome::Granted(l) = ra.request(&req(ConsumerId::Llm, 4000)) {
                ra.release(l.token);
            }
        }
        {
            let store = JournalStore::new(&path);
            let ra = LocalAuthority::new_persisted(table(), PolicyProfile::Balanced, store);
            assert!(ra.recovered_open_leases().is_empty());
        }
        let _ = std::fs::remove_file(&path);
    }
}
