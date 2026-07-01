//! Hardware & Resource Authority (HRA) — control-plane foundations.
//!
//! This module hosts the deterministic, additive HRA building blocks specified in
//! `.kiro/specs/hardware-resource-authority/`. These are pure/testable units wired incrementally
//! into the runtime by later tasks; none of them redesign existing components — they extend them.
//!
//! Implemented so far (HRA implementation phase):
//! - `types`               — shared control-plane vocabulary (Task 2)
//! - `capability`          — Capability Vector (Task 24 / R18)
//! - `budget`              — multi-band memory budget (Task 45 / R27)
//! - `simulator`           — pre-commit resource simulator (Task 43 / R25)
//! - `capability_registry` — deterministic model selection registry (Task 46 / R28)
//! - `device_table`        — authoritative device table + reservations (Task 4 / R1.3)
//! - `planner`             — deterministic placement cost model (Task 5 / R13.1)
//! - `journal`             — checksummed/versioned decision journal (Task 8 / R21.2)
//! - `scheduler`           — admission, leases, priority, preemption, shedding (Task 6 / R6,R21)
//! - `pressure`            — EMA/dwell/hysteresis pressure engine (Task 7 / R5)
//! - `reconciler`          — crash-recovery + epoch fencing + kill-scope (Task 9 / R12,R21,R23)
//! - `lifecycle`           — uniform ModelLifecycle contract (Task 11 / R4.1)
//! - `residency_manager`   — single residency executor (Task 42 / R24)
//! - `ra`                  — Resource Authority assembly + bypass (Tasks 10,35,39)
//! - `session`             — Session Intent Profiles + Session Ownership (Tasks 31,44)
//! - `predict`             — Workload Prediction + Forecasting + Autonomous Optimization (30,32,34)
//! - `thermal`             — Thermal & Power Policy Engine (Task 33)
//! - `sla`                 — SLA framework (Task 47)
//! - `foreground_guard`    — disruptive-op chokepoint (Task 25)
//! - `anomaly`             — root-cause anomaly detectors (Task 18)
//! - `benchmark`           — resource benchmark + regression gate (Task 48)
//! - `collector`           — host telemetry snapshot model + DeviceTable apply (Task 3)
//! - `cloud_health`        — cloud circuit breaker (Task 29)
//! - `metrics`             — low-cardinality SLO counters/histogram (Task 36)
//! - `shadow`              — shadow comparator + cutover gate (Task 37)
//! - `security`            — kill-scope capability gate + privacy egress (Task 38)
//! - `daemon_supervisor`   — restart/backoff/circuit-breaker FSM (Task 19)
//! - `service`             — HraService runtime assembly façade (Tasks 3/10/36/37 + bypass)

pub mod activity;
pub mod anomaly;
pub mod benchmark;
pub mod benefit;
pub mod budget;
pub mod capability;
pub mod capability_registry;
pub mod cloud_health;
pub mod co_residency;
pub mod collector;
pub mod daemon_supervisor;
pub mod device_table;
pub mod foreground_guard;
pub mod journal;
pub mod journal_store;
pub mod lifecycle;
pub mod metrics;
pub mod planner;
pub mod policy;
pub mod predict;
pub mod pressure;
pub mod ra;
pub mod reconciler;
pub mod residency_manager;
pub mod scheduler;
pub mod security;
pub mod service;
pub mod session;
pub mod shadow;
pub mod simulator;
pub mod sla;
pub mod thermal;
pub mod types;

pub use anomaly::{Anomaly, AnomalyKind};
pub use activity::{ActivityModel, ActivitySignals, ActivityState, ActivityThresholds};
pub use benefit::{evaluate as evaluate_benefit, Benefit, BenefitEval, BenefitInputs, BenefitReason, BenefitThresholds};
pub use benchmark::{detect_regressions, gate_passes, BenchResult, Regression, RegressionTolerance};
pub use budget::{BandPolicy, Budget};
pub use capability::CapabilityVector;
pub use capability_registry::{
    CapabilityRegistry, LatencyClass, ModelCapability, QualityTier, SelectQuery,
};
pub use cloud_health::{Breaker, CloudHealth};
pub use co_residency::{
    CoResidencyError, CoResidencyLease, CoResidencyManager, CoResidencyMetrics, CoResidencyPolicy,
    ResidencyTarget, ResidentSnapshot,
};
pub use collector::{CpuLive, DeviceLive, HostSnapshot, RamLive};
pub use daemon_supervisor::{DaemonState, DaemonSupervisor, SupervisorPolicy};
pub use device_table::{BreakerState, DeviceHealth, DeviceRecord, DeviceTable};
pub use foreground_guard::{ActionImpact, ForegroundGuard, GuardContext, GuardDecision};
pub use journal::{Decision, DecisionKind, Journal, JournalRecord};
pub use journal_store::JournalStore;
pub use lifecycle::{ModelDescriptor, ModelHealth, ModelLifecycle, ResidencyState};
pub use metrics::{Counters, LatencyHistogram};
pub use planner::{plan, PolicyProfile, PolicyWeights};
pub use policy::{
    decide as policy_decide, decide_image_admission, derive_mode, Action, Confidence,
    Decision as PolicyDecision, HealthPosture, ImageAdmission, LockPosture, PolicyInputs, PolicyLog,
    PolicyReason, RuntimeMode,
};
pub use predict::{
    prewarm_allowed, prewarm_hint, AutonomousOptimizer, Forecast, Forecaster, PrewarmHint,
    ResourceKind, WorkloadSignal,
};
pub use pressure::{PressureEngine, PressureLevel, Remedy};
pub use ra::{LocalAuthority, RaOutcome, ResourceAuthority};
pub use reconciler::{reconcile, ObservedProcess, ReconcilePlan};
pub use residency_manager::{BreakCondition, LockState, ResidencyError, ResidencyManager, ResidentLock};
pub use scheduler::{AdmitError, Lease, LeaseToken, QueueCaps, Scheduler};
pub use security::{egress_allowed, CapabilityToken, KillAuthorizer};
pub use service::{global_hra, set_global_hra, AdmissionGuard, GpuAdmissionAdvice, HraService};
pub use session::{SessionIntent, SessionOwnership, SessionProfile};
pub use shadow::{compare as shadow_compare, Divergence, ShadowReport};
pub use simulator::{simulate, Disruption, Estimate, RiskLevel, SimAction, SimDeviceState};
pub use sla::{Sla, SlaState, SlaTable};
pub use thermal::{decide as thermal_decide, PowerDecision, PowerSource, ThermalState};
pub use types::{
    Capacity, Constraints, ConsumerId, DeviceId, DeviceKind, Epoch, Plan, PowerReq, PriorityClass,
    PrivacyReq, RationaleCode, Residency, ResourceNeed, ResourceRequest, TurnId,
};
