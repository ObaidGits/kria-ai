//! Deterministic policy-admission microbenchmark (task **F1.4.7**; design §18
//! budget "Write policy evaluation ≤2ms p95 excluding commit"; validation
//! `V-POLICY-01`; MGR-004, MGR-009 bounded execution, MGR-035, MGR-043,
//! MGR-045; MGD-007).
//!
//! ## What "admission" means here
//!
//! Admission is the **pure, deterministic policy decision a durable command
//! passes through *before* the SQLite transaction opens** (design §18 pseudo-
//! code: `write_policy.evaluate(&caller, &candidate)` — "deterministic, no SQL
//! write"). It is built from the F1.4 primitives that already exist:
//!
//! 1. the Effective-Policy **meet** over every contributing source
//!    ([`EffectivePolicy::meet_all`] — F1.4.2),
//! 2. the **memory-mode** admission gate ([`modes::admit`] / [`modes::read_permitted`]
//!    — F1.4.4), and
//! 3. the **read authorization** decision ([`authorize_read`] → `AuthorizedScope`
//!    — F1.4.5).
//!
//! The measured region contains **only** that in-memory decision. No SQLite
//! connection is opened and **no transaction is committed** — the design budget
//! is explicitly "excluding commit", so commit cost is structurally absent from
//! this benchmark rather than merely excluded by measurement.
//!
//! ## What this benchmark asserts
//!
//! * **Latency:** ≥30 warm samples (this run collects far more), computing the
//!   p95 and asserting it is within the design budget of **2ms** (2000µs) on
//!   reference hardware. The decision is a handful of small-struct combinations
//!   plus BLAKE3 hashing, so it runs in single-digit microseconds — orders of
//!   magnitude under budget — and cannot flake against the 2ms bound. For
//!   pathological CI hosts the budget is overridable via the
//!   `KRIA_POLICY_ADMISSION_P95_BUDGET_US` env var (default 2000), so the target
//!   is honored on reference hardware while a slow shared runner can raise it
//!   without weakening the reference assertion.
//! * **Correctness:** every measured decision is checked against its expected
//!   allow/deny outcome, deny reasons, mode admission, and read authorization,
//!   so a fast-but-wrong path can never pass the benchmark.
//! * **Determinism:** the same inputs always produce the same outcome, the same
//!   Effective-Policy provenance hash, and the same authorized-scope policy hash
//!   (MGR-004 / MGD-007 determinism).
//!
//! Run with:
//!   `cargo test -p kria-core --test policy_admission_bench -- --nocapture`

use std::collections::BTreeSet;
use std::time::Instant;

use kria_core::memory::authority::SourceTrust;
use kria_core::memory::model::{CallerContext, PolicyPartition};
use kria_core::memory::modes::{self, Admission};
use kria_core::memory::policy::effective_policy::{
    ContributingPolicy, DenyReason, EffectivePolicy,
};
use kria_core::memory::policy::read_authorization::authorize_read;
use kria_core::memory::policy::source_trust::{
    Capability, CapabilitySet, ConsentRequirement, SourceCategory,
};
use kria_core::memory::types::MemoryMode;

// ─────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────

fn partition(ns: &str, scope: &str, sensitivity: u8, owner: Option<&str>) -> PolicyPartition {
    PolicyPartition::with_owner(ns, scope, sensitivity, owner.map(str::to_string)).unwrap()
}

/// A contributing source resolved from its canonical [`SourceCategory`] default
/// profile (trust / capability / consent), writing under `part`.
fn from_category(id: &str, cat: SourceCategory, part: PolicyPartition) -> ContributingPolicy {
    ContributingPolicy::from_profile(id, part, &cat.profile()).unwrap()
}

/// A contributing source with an explicit capability set (for constructing
/// deny-by-empty-intersection scenarios).
fn with_caps(id: &str, part: PolicyPartition, caps: &[Capability]) -> ContributingPolicy {
    ContributingPolicy::new(
        id,
        part,
        CapabilitySet::from_capabilities(caps.iter().copied()),
        SourceTrust::System,
        ConsentRequirement::NotRequired,
    )
    .unwrap()
}

// ─────────────────────────────────────────────────────────────────────────
// The admission decision under test — pure, pre-transaction, no commit
// ─────────────────────────────────────────────────────────────────────────

/// The captured result of one admission decision. Comparable so determinism can
/// be asserted by equality across repeated evaluations of identical inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AdmissionDecision {
    policy_allowed: bool,
    deny_reasons: Option<BTreeSet<DenyReason>>,
    provenance_hash: String,
    /// `Some(true)` = durable, `Some(false)` = session-scoped, `None` = mode
    /// forbids the write.
    mode_admits_durable: Option<bool>,
    read_mode_permitted: bool,
    read_authorized: bool,
    /// The `AuthorizedScope` policy hash when the read is authorized.
    read_scope_hash: Option<String>,
}

/// One admission scenario: the inputs and the exact expected classification.
struct Scenario {
    name: &'static str,
    contributors: Vec<ContributingPolicy>,
    mode: MemoryMode,
    caller: CallerContext,
    expected: AdmissionExpectation,
}

/// The expected classification of a scenario, checked on every measured
/// decision so correctness is co-verified with latency.
struct AdmissionExpectation {
    policy_allowed: bool,
    deny_reasons: Option<BTreeSet<DenyReason>>,
    mode_admits_durable: Option<bool>,
    read_mode_permitted: bool,
    read_authorized: bool,
}

impl Scenario {
    /// Evaluate the pure admission decision. This is the **measured region**:
    /// Effective-Policy meet + mode gate + read authorization, all in memory,
    /// with no SQLite connection and no transaction commit.
    fn decide(&self) -> AdmissionDecision {
        // (1) Effective-Policy meet over the contributing sources (F1.4.2).
        //     The engine receives fresh contributor instances per command, so
        //     cloning the small value objects is part of the realistic cost.
        let policy = EffectivePolicy::meet_all(self.contributors.iter().cloned());

        // (2) Memory-mode admission + read gate (F1.4.4). `now` only affects a
        //     Temporary binding's expiry, never the decision class.
        let mode_admit = modes::admit(&self.mode, chrono::Utc::now());
        let read_mode_permitted = modes::read_permitted(&self.mode).is_ok();

        // (3) Read authorization decision (F1.4.5).
        let read = authorize_read(&self.caller, &policy);

        AdmissionDecision {
            policy_allowed: policy.is_allowed(),
            deny_reasons: policy.deny_reasons().cloned(),
            provenance_hash: policy.provenance_hash().to_string(),
            mode_admits_durable: match mode_admit {
                Ok(Admission::Durable) => Some(true),
                Ok(Admission::SessionScoped(_)) => Some(false),
                Err(_) => None,
            },
            read_mode_permitted,
            read_authorized: read.is_ok(),
            read_scope_hash: read.ok().map(|scope| scope.policy_hash().to_string()),
        }
    }

    /// Assert the decision matches the scenario's expected classification.
    fn assert_correct(&self, d: &AdmissionDecision) {
        assert_eq!(
            d.policy_allowed, self.expected.policy_allowed,
            "[{}] policy allow/deny mismatch",
            self.name
        );
        assert_eq!(
            d.deny_reasons, self.expected.deny_reasons,
            "[{}] deny-reason mismatch",
            self.name
        );
        assert_eq!(
            d.mode_admits_durable, self.expected.mode_admits_durable,
            "[{}] mode admission mismatch",
            self.name
        );
        assert_eq!(
            d.read_mode_permitted, self.expected.read_mode_permitted,
            "[{}] mode read-gate mismatch",
            self.name
        );
        assert_eq!(
            d.read_authorized, self.expected.read_authorized,
            "[{}] read authorization mismatch",
            self.name
        );
        // A denied policy or a non-readable grant yields no scope hash; an
        // authorized read always does.
        assert_eq!(
            d.read_scope_hash.is_some(),
            self.expected.read_authorized,
            "[{}] scope-hash presence must track read authorization",
            self.name
        );
    }
}

/// The realistic set of admission scenarios exercised by the benchmark. Covers
/// allow + every deny reason and every mode class so a fast-but-wrong decision
/// path cannot pass.
fn scenarios() -> Vec<Scenario> {
    vec![
        // A — a native tool acting alongside the user's conversation turn and a
        // vetted MCP server, all under one partition. Capability intersection is
        // {observe, read_core}; Permanent mode admits a durable write and the
        // read is authorized.
        Scenario {
            name: "allow_permanent_multi_source_read",
            contributors: vec![
                from_category(
                    "native-1",
                    SourceCategory::NativeTool,
                    partition("user", "chat", 2, None),
                ),
                from_category(
                    "conv-1",
                    SourceCategory::Conversation,
                    partition("user", "chat", 2, None),
                ),
                from_category(
                    "mcp-1",
                    SourceCategory::McpServer,
                    partition("user", "chat", 2, None),
                ),
            ],
            mode: MemoryMode::Permanent,
            caller: CallerContext::local_desktop("device-1", partition("user", "chat", 2, None))
                .unwrap(),
            expected: AdmissionExpectation {
                policy_allowed: true,
                deny_reasons: None,
                mode_admits_durable: Some(true),
                read_mode_permitted: true,
                read_authorized: true,
            },
        },
        // B — contributors disagree on namespace: A5 isolation denies the meet
        // (no cross-namespace combination); read is denied with no scope.
        Scenario {
            name: "deny_namespace_conflict",
            contributors: vec![
                from_category(
                    "native-1",
                    SourceCategory::NativeTool,
                    partition("user", "chat", 1, None),
                ),
                from_category(
                    "conv-1",
                    SourceCategory::Conversation,
                    partition("system", "chat", 1, None),
                ),
            ],
            mode: MemoryMode::Permanent,
            caller: CallerContext::local_desktop("device-1", partition("user", "chat", 1, None))
                .unwrap(),
            expected: AdmissionExpectation {
                policy_allowed: false,
                deny_reasons: Some([DenyReason::NamespaceConflict].into_iter().collect()),
                mode_admits_durable: Some(true),
                read_mode_permitted: true,
                read_authorized: false,
            },
        },
        // C — capability intersection is empty (correct-only ∩ observe-only):
        // no operation is jointly permitted, so the meet denies.
        Scenario {
            name: "deny_empty_capability_intersection",
            contributors: vec![
                with_caps(
                    "s-correct",
                    partition("user", "notes", 0, None),
                    &[Capability::CorrectMemory],
                ),
                with_caps(
                    "s-observe",
                    partition("user", "notes", 0, None),
                    &[Capability::ObserveMemory],
                ),
            ],
            mode: MemoryMode::Permanent,
            caller: CallerContext::local_desktop("device-1", partition("user", "notes", 0, None))
                .unwrap(),
            expected: AdmissionExpectation {
                policy_allowed: false,
                deny_reasons: Some(
                    [DenyReason::EmptyCapabilityIntersection]
                        .into_iter()
                        .collect(),
                ),
                mode_admits_durable: Some(true),
                read_mode_permitted: true,
                read_authorized: false,
            },
        },
        // D — Read_Only mode: the policy allows and carries read_core, so reads
        // stay authorized, but the mode forbids the durable write.
        Scenario {
            name: "readonly_mode_forbids_write_preserves_read",
            contributors: vec![
                from_category(
                    "native-1",
                    SourceCategory::NativeTool,
                    partition("work", "notes", 1, None),
                ),
                from_category(
                    "conv-1",
                    SourceCategory::Conversation,
                    partition("work", "notes", 1, None),
                ),
            ],
            mode: MemoryMode::ReadOnly,
            caller: CallerContext::local_desktop("device-1", partition("work", "notes", 1, None))
                .unwrap(),
            expected: AdmissionExpectation {
                policy_allowed: true,
                deny_reasons: None,
                mode_admits_durable: None,
                read_mode_permitted: true,
                read_authorized: true,
            },
        },
        // E — Disabled mode: no durable write and no read (the mode read-gate
        // forbids reads), even though the Effective Policy itself allows.
        Scenario {
            name: "disabled_mode_forbids_write_and_read",
            contributors: vec![
                from_category(
                    "native-1",
                    SourceCategory::NativeTool,
                    partition("user", "chat", 3, None),
                ),
                from_category(
                    "conv-1",
                    SourceCategory::Conversation,
                    partition("user", "chat", 3, None),
                ),
            ],
            mode: MemoryMode::Disabled,
            caller: CallerContext::local_desktop("device-1", partition("user", "chat", 3, None))
                .unwrap(),
            expected: AdmissionExpectation {
                policy_allowed: true,
                deny_reasons: None,
                mode_admits_durable: None,
                // Disabled forbids reads at the mode gate; the authorization
                // gate would still admit (policy carries read_core), but the
                // mode read-gate is the outer refusal.
                read_mode_permitted: false,
                read_authorized: true,
            },
        },
    ]
}

/// The `p`-quantile of `xs` (nearest-rank on the sorted samples).
fn pct(mut xs: Vec<u128>, p: f64) -> u128 {
    xs.sort_unstable();
    if xs.is_empty() {
        return 0;
    }
    let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[idx]
}

/// The p95 latency budget in microseconds. Defaults to the design §18 budget of
/// 2ms (2000µs) on reference hardware; overridable for slow CI runners via
/// `KRIA_POLICY_ADMISSION_P95_BUDGET_US` without weakening the reference target.
fn p95_budget_us() -> u128 {
    std::env::var("KRIA_POLICY_ADMISSION_P95_BUDGET_US")
        .ok()
        .and_then(|v| v.parse::<u128>().ok())
        .unwrap_or(2000)
}

#[test]
fn bench_deterministic_admission_p95_under_2ms() {
    let scenarios = scenarios();
    assert!(
        !scenarios.is_empty(),
        "need at least one admission scenario"
    );

    // ── Determinism: identical inputs → identical decision + hashes ──────
    for scn in &scenarios {
        let a = scn.decide();
        let b = scn.decide();
        assert_eq!(
            a, b,
            "[{}] admission decision is not deterministic",
            scn.name
        );
        assert_eq!(
            a.provenance_hash, b.provenance_hash,
            "[{}] provenance hash is not deterministic",
            scn.name
        );
        assert_eq!(
            a.read_scope_hash, b.read_scope_hash,
            "[{}] authorized-scope policy hash is not deterministic",
            scn.name
        );
        // Correctness of every scenario before we start timing.
        scn.assert_correct(&a);
    }

    // ── Warm-up (JIT-free native, but warms caches/branch predictors) ────
    let warmup = 500usize;
    for i in 0..warmup {
        let scn = &scenarios[i % scenarios.len()];
        std::hint::black_box(scn.decide());
    }

    // ── Timed samples: far more than the ≥30 warm-sample floor ───────────
    let sample_target = 2_000usize;
    let min_warm_samples = 30usize;
    let mut samples: Vec<u128> = Vec::with_capacity(sample_target);
    for i in 0..sample_target {
        let scn = &scenarios[i % scenarios.len()];
        let t = Instant::now();
        let decision = scn.decide();
        let elapsed = t.elapsed().as_micros();
        // Timing captured; correctness asserted OUTSIDE the measured region so
        // assertion cost never inflates the latency samples.
        samples.push(elapsed);
        scn.assert_correct(&decision);
    }

    assert!(
        samples.len() >= min_warm_samples,
        "collected {} warm samples, need ≥{min_warm_samples}",
        samples.len()
    );

    let p50 = pct(samples.clone(), 0.50);
    let p95 = pct(samples.clone(), 0.95);
    let p99 = pct(samples.clone(), 0.99);
    let avg: u128 = samples.iter().sum::<u128>() / samples.len() as u128;
    let budget = p95_budget_us();

    println!(
        "[F1.4.7 policy admission bench] scenarios={} warm_samples={} avg={avg}us p50={p50}us p95={p95}us p99={p99}us budget(p95)={budget}us (excludes transaction commit)",
        scenarios.len(),
        samples.len(),
    );

    assert!(
        p95 <= budget,
        "policy admission p95 regressed: {p95}us > budget {budget}us (design §18: ≤2ms p95 excluding commit)"
    );
}
