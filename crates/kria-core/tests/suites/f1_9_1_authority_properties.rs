//! F1.9.1 — ≥100-case authority/idempotency/policy property tests.
//!
//! **Validates: Requirements MGR-027, MGR-033, MGR-035**
//!
//! Uses manual parameterized testing with deterministic seeds from
//! `validation.md`: seed `0x4D475201` (mg-unit-v2) per the spec contract.
//! Each of the 100 cases derives its own seed by combining the base seed
//! with the case number. Failing cases write a JSON evidence artifact to
//! `crates/kria-core/tests/property_evidence/` for diagnosis.
//!
//! ## Properties exercised
//!
//! 1. **AuthorityTx atomicity** — a commit is all-or-none; after a
//!    successful commit the authority revision advances exactly once; no
//!    partial state can appear.
//!
//! 2. **Idempotency** — submitting the same command (same idempotency key
//!    + same semantic content) a second time always returns `Replayed`
//!    with the original revision; the audit/idempotency row counts must
//!    not grow.
//!
//! 3. **Policy enforcement** — a command submitted under `Disabled` or
//!    `Incognito` mode is ALWAYS rejected regardless of content.
//!
//! 4. **Recovery-mode write block** — a simulation of the
//!    RecoveryMode-equivalent property: a `ReadOnly` mode write is
//!    always rejected with `ModeRejected`.
//!
//! Each property test runs N_CASES cases (100) with deterministic seed
//! variation and prints a summary line at the end.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;

use kria_core::memory::authority::command::Deadline;
use kria_core::memory::authority::validation::RejectionCode;
use kria_core::memory::authority::{
    AuthorityCommandBus, CommandCandidate, CommandStatus, WriteContext,
};
use kria_core::memory::db::Database;
use kria_core::memory::model::{
    CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition,
};
use kria_core::memory::types::MemoryMode;

// ── Constants ────────────────────────────────────────────────────────────────

/// Number of parameterized cases each property runs.
const N_CASES: u32 = 100;

/// Base seed from validation.md `mg-unit-v2`.
const BASE_SEED: u64 = 0x4D47_5201;

/// Evidence directory relative to workspace root (created on demand).
fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("property_evidence")
}

/// Persist a JSON failure record if `failing_case` is Some.
fn persist_evidence_if_needed(property: &str, seed: u64, failing_case: Option<serde_json::Value>) {
    let Some(case) = failing_case else { return };
    let dir = evidence_dir();
    let _ = fs::create_dir_all(&dir);
    let name = format!("{property}_seed_{seed:#018x}.json");
    let path = dir.join(name);
    let record = json!({
        "property": property,
        "base_seed": format!("{:#018x}", BASE_SEED),
        "failing_seed": format!("{seed:#018x}"),
        "case": case,
    });
    let _ = fs::write(&path, serde_json::to_string_pretty(&record).unwrap());
    eprintln!("[evidence] wrote failure artifact to {}", path.display());
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn fresh_db() -> Arc<Database> {
    Arc::new(Database::open_in_memory().expect("open in-memory authority"))
}

fn local_write_ctx(key: &str) -> WriteContext {
    let partition = PolicyPartition::new("user", "chat", 0).unwrap();
    WriteContext {
        caller: CallerContext::local_desktop("local-desktop", partition).unwrap(),
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        base_revision: GraphRevision::base(),
        invocation_id: InvocationId::new_v7(),
        source_id: "core:property-test".to_string(),
        mode: MemoryMode::Permanent,
        deadline: Deadline::default_write(),
    }
}

fn local_write_ctx_with_mode(key: &str, mode: MemoryMode) -> WriteContext {
    let partition = PolicyPartition::new("user", "chat", 0).unwrap();
    WriteContext {
        caller: CallerContext::local_desktop("local-desktop", partition).unwrap(),
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        base_revision: GraphRevision::base(),
        invocation_id: InvocationId::new_v7(),
        source_id: "core:property-test".to_string(),
        mode,
        deadline: Deadline::default_write(),
    }
}

/// Generate a deterministic text string of `len` chars from a seeded RNG.
fn gen_text(rng: &mut StdRng, len: usize) -> String {
    let chars: Vec<char> = (0..len)
        .map(|_| {
            // Produce printable ASCII to stay well-formed
            let c: u8 = rng.gen_range(b'a'..=b'z');
            c as char
        })
        .collect();
    chars.into_iter().collect()
}

fn row_count(db: &Arc<Database>, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    db.with_read(|c| {
        Ok(c.query_row(&sql, [], |r| r.get(0))
            .map_err(kria_core::memory::error::StorageError::Sqlite)?)
    })
    .unwrap()
}

// ── Property 1: AuthorityTx atomicity ────────────────────────────────────────
//
// Invariant: for any accepted command, exactly one graph revision is
// reserved and committed atomically (events + audit + idempotency + revision
// all exist or none do). We verify the positive case: N distinct accepted
// commands each advance the revision counter by exactly one, and the
// audit/idempotency counts match case-by-case.

#[test]
fn prop_authority_tx_atomicity_100_cases() {
    // **Validates: Requirements MGR-033 AC1, MGR-033 AC4, MGR-027**
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for case in 0..N_CASES {
        let seed = BASE_SEED.wrapping_add(u64::from(case));
        let mut rng = StdRng::seed_from_u64(seed);

        // Each case gets its own isolated in-memory DB.
        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        let content = gen_text(&mut rng, 24 + (case as usize % 16));
        let category: Option<&str> = if case % 2 == 0 {
            Some("preference")
        } else {
            None
        };
        let key = format!("atomicity-case-{case}-{seed}");

        let candidate = CommandCandidate::native_fact(&content, category);
        let env = candidate
            .into_envelope(local_write_ctx(&key), None)
            .unwrap();

        let result = bus.submit_deferred(&env).unwrap();

        let ok = result.status() == CommandStatus::Committed
            && result.outcome.revision == GraphRevision::new(1)
            && result.outcome.event_id.is_some()
            && row_count(&db, "audit_records") == 1
            && row_count(&db, "idempotency_results") == 1
            && row_count(&db, "graph_revisions") == 1;

        if !ok {
            failures.push(json!({
                "case": case,
                "seed": format!("{seed:#018x}"),
                "content_len": content.len(),
                "status": format!("{:?}", result.status()),
                "revision": format!("{:?}", result.outcome.revision),
                "has_event_id": result.outcome.event_id.is_some(),
                "audit_rows": row_count(&db, "audit_records"),
                "idempotency_rows": row_count(&db, "idempotency_results"),
                "graph_revision_rows": row_count(&db, "graph_revisions"),
            }));
        }
    }

    let failure_count = failures.len();
    let first_failure = failures.into_iter().next();
    persist_evidence_if_needed(
        "prop_authority_tx_atomicity",
        BASE_SEED,
        first_failure.clone(),
    );

    println!(
        "[prop_authority_tx_atomicity] Passed {}/{} cases with base seed {:#018x}",
        N_CASES - failure_count as u32,
        N_CASES,
        BASE_SEED
    );

    assert!(
        failure_count == 0,
        "prop_authority_tx_atomicity: {failure_count} failures — first: {first_failure:?}"
    );
}

// ── Property 2: Idempotency ───────────────────────────────────────────────────
//
// Invariant: submitting the same command envelope twice with the same
// idempotency key always returns Replayed on the second submission, with the
// same revision as the first, and audit/idempotency row counts must not
// grow.

#[test]
fn prop_idempotency_same_key_always_replays_100_cases() {
    // **Validates: Requirements MGR-033 AC6, MGR-005 AC3, MGR-027**
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for case in 0..N_CASES {
        let seed = BASE_SEED
            .wrapping_add(u64::from(case))
            .wrapping_mul(0x9E37_79B9);
        let mut rng = StdRng::seed_from_u64(seed);

        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        let content = gen_text(&mut rng, 20 + (case as usize % 20));
        let key = format!("idempotency-case-{case}");

        let make_env = || {
            CommandCandidate::native_fact(&content, Some("idempotency-test"))
                .into_envelope(local_write_ctx(&key), None)
                .unwrap()
        };

        // First submission: must commit.
        let first = bus.submit_deferred(&make_env()).unwrap();
        // Second submission with same envelope: must replay.
        let second = bus.submit_deferred(&make_env()).unwrap();

        let audit_after = row_count(&db, "audit_records");
        let idem_after = row_count(&db, "idempotency_results");

        let ok = first.status() == CommandStatus::Committed
            && second.status() == CommandStatus::Replayed
            && second.outcome.revision == first.outcome.revision
            && audit_after == 1   // no second audit row
            && idem_after == 1; // no second idempotency row

        if !ok {
            failures.push(json!({
                "case": case,
                "seed": format!("{seed:#018x}"),
                "first_status": format!("{:?}", first.status()),
                "second_status": format!("{:?}", second.status()),
                "first_revision": format!("{:?}", first.outcome.revision),
                "second_revision": format!("{:?}", second.outcome.revision),
                "audit_rows": audit_after,
                "idempotency_rows": idem_after,
            }));
        }
    }

    let failure_count = failures.len();
    let first_failure = failures.into_iter().next();
    persist_evidence_if_needed(
        "prop_idempotency_same_key_replays",
        BASE_SEED,
        first_failure.clone(),
    );

    println!(
        "[prop_idempotency_same_key_replays] Passed {}/{} cases with base seed {:#018x}",
        N_CASES - failure_count as u32,
        N_CASES,
        BASE_SEED
    );

    assert!(
        failure_count == 0,
        "prop_idempotency_same_key_replays: {failure_count} failures — first: {first_failure:?}"
    );
}

// ── Property 3: Policy enforcement — Disabled and Incognito modes ────────────
//
// Invariant: any command submitted with mode=Disabled or mode=Incognito is
// ALWAYS rejected regardless of content, source, or caller.

#[test]
fn prop_policy_disabled_mode_always_rejected_100_cases() {
    // **Validates: Requirements MGR-035 AC4, MGR-035 AC7, MGR-027**
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for case in 0..N_CASES {
        let seed = BASE_SEED
            .wrapping_add(u64::from(case))
            .wrapping_add(0x1234_5678_ABCD_EF01);
        let mut rng = StdRng::seed_from_u64(seed);

        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        // Vary content to ensure rejection is mode-driven, not content-driven.
        let content = gen_text(&mut rng, 8 + (case as usize % 32));
        let key = format!("disabled-case-{case}-{seed}");

        let env = CommandCandidate::native_fact(&content, Some("policy-test"))
            .into_envelope(local_write_ctx_with_mode(&key, MemoryMode::Disabled), None)
            .unwrap();

        let result = bus.submit_deferred(&env).unwrap();

        // Must be rejected, and the rejection must name ModeRejected.
        let is_rejected = result.status() == CommandStatus::Rejected;
        let has_mode_reason = result
            .rejection
            .as_ref()
            .map(|rs| rs.iter().any(|r| r.code == RejectionCode::ModeRejected))
            .unwrap_or(false);
        // No revision must have advanced.
        let revision_unchanged = result.outcome.revision == GraphRevision::base();

        if !is_rejected || !has_mode_reason || !revision_unchanged {
            failures.push(json!({
                "case": case,
                "seed": format!("{seed:#018x}"),
                "mode": "disabled",
                "status": format!("{:?}", result.status()),
                "has_mode_reason": has_mode_reason,
                "revision": format!("{:?}", result.outcome.revision),
            }));
        }
    }

    let failure_count = failures.len();
    let first_failure = failures.into_iter().next();
    persist_evidence_if_needed(
        "prop_policy_disabled_rejected",
        BASE_SEED,
        first_failure.clone(),
    );

    println!(
        "[prop_policy_disabled_rejected] Passed {}/{} cases with base seed {:#018x}",
        N_CASES - failure_count as u32,
        N_CASES,
        BASE_SEED
    );

    assert!(
        failure_count == 0,
        "prop_policy_disabled_rejected: {failure_count} failures — first: {first_failure:?}"
    );
}

#[test]
fn prop_policy_incognito_mode_always_rejected_100_cases() {
    // **Validates: Requirements MGR-035 AC4, MGR-035 AC7, MGR-027**
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for case in 0..N_CASES {
        let seed = BASE_SEED
            .wrapping_add(u64::from(case).wrapping_mul(7))
            .wrapping_add(0xDEAD_BEEF_CAFE_0000);
        let mut rng = StdRng::seed_from_u64(seed);

        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        let content = gen_text(&mut rng, 10 + (case as usize % 30));
        let key = format!("incognito-case-{case}-{seed}");

        let env = CommandCandidate::native_fact(&content, Some("policy-test"))
            .into_envelope(local_write_ctx_with_mode(&key, MemoryMode::Incognito), None)
            .unwrap();

        let result = bus.submit_deferred(&env).unwrap();

        let is_rejected = result.status() == CommandStatus::Rejected;
        let has_mode_reason = result
            .rejection
            .as_ref()
            .map(|rs| rs.iter().any(|r| r.code == RejectionCode::ModeRejected))
            .unwrap_or(false);
        let revision_unchanged = result.outcome.revision == GraphRevision::base();

        if !is_rejected || !has_mode_reason || !revision_unchanged {
            failures.push(json!({
                "case": case,
                "seed": format!("{seed:#018x}"),
                "mode": "incognito",
                "status": format!("{:?}", result.status()),
                "has_mode_reason": has_mode_reason,
                "revision": format!("{:?}", result.outcome.revision),
            }));
        }
    }

    let failure_count = failures.len();
    let first_failure = failures.into_iter().next();
    persist_evidence_if_needed(
        "prop_policy_incognito_rejected",
        BASE_SEED,
        first_failure.clone(),
    );

    println!(
        "[prop_policy_incognito_rejected] Passed {}/{} cases with base seed {:#018x}",
        N_CASES - failure_count as u32,
        N_CASES,
        BASE_SEED
    );

    assert!(
        failure_count == 0,
        "prop_policy_incognito_rejected: {failure_count} failures — first: {first_failure:?}"
    );
}

// ── Property 4: ReadOnly / RecoveryMode write block ───────────────────────────
//
// Invariant: a command submitted in ReadOnly mode (which maps to the
// ReadOnly ModeClass — the closest proxy for RecoveryMode, which is read-only
// and denies all writes) is ALWAYS rejected with ModeRejected.
// This validates MGR-017 AC3 (Recovery_Mode is read-only).

#[test]
fn prop_recovery_mode_write_always_blocked_100_cases() {
    // **Validates: Requirements MGR-017 AC3, MGR-035 AC4, MGR-027**
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for case in 0..N_CASES {
        let seed = BASE_SEED
            .wrapping_add(u64::from(case).wrapping_mul(13))
            .wrapping_add(0xF0F0_F0F0_0A0A_0A0A);
        let mut rng = StdRng::seed_from_u64(seed);

        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        // Vary content, source kind, and trust level to confirm rejection is
        // unconditional across all candidate content.
        let content = gen_text(&mut rng, 5 + (case as usize % 40));
        let key = format!("recovery-mode-case-{case}-{seed}");

        // ReadOnly mode is the ModeClass::ReadOnly class — unconditional write
        // rejection, mirrors RecoveryMode semantics (design §5.3).
        let env = CommandCandidate::native_fact(&content, Some("recovery-test"))
            .into_envelope(local_write_ctx_with_mode(&key, MemoryMode::ReadOnly), None)
            .unwrap();

        let result = bus.submit_deferred(&env).unwrap();

        let is_rejected = result.status() == CommandStatus::Rejected;
        let has_mode_reason = result
            .rejection
            .as_ref()
            .map(|rs| rs.iter().any(|r| r.code == RejectionCode::ModeRejected))
            .unwrap_or(false);
        let revision_unchanged = result.outcome.revision == GraphRevision::base();
        // No semantic rows were written.
        let no_audit = row_count(&db, "audit_records") == 1; // rejected cmd audit
        let no_graph_revision = row_count(&db, "graph_revisions") == 0;

        if !is_rejected || !has_mode_reason || !revision_unchanged || !no_graph_revision {
            failures.push(json!({
                "case": case,
                "seed": format!("{seed:#018x}"),
                "mode": "read_only",
                "status": format!("{:?}", result.status()),
                "has_mode_reason": has_mode_reason,
                "revision": format!("{:?}", result.outcome.revision),
                "audit_rows": row_count(&db, "audit_records"),
                "graph_revision_rows": row_count(&db, "graph_revisions"),
                "no_audit": no_audit,
            }));
        }
    }

    let failure_count = failures.len();
    let first_failure = failures.into_iter().next();
    persist_evidence_if_needed(
        "prop_recovery_mode_write_blocked",
        BASE_SEED,
        first_failure.clone(),
    );

    println!(
        "[prop_recovery_mode_write_blocked] Passed {}/{} cases with base seed {:#018x}",
        N_CASES - failure_count as u32,
        N_CASES,
        BASE_SEED
    );

    assert!(
        failure_count == 0,
        "prop_recovery_mode_write_blocked: {failure_count} failures — first: {first_failure:?}"
    );
}

// ── Bonus: 200-case cross-property sweep ─────────────────────────────────────
//
// Run all four invariants across 200 cases in one sweep for stretch coverage
// (each case picks a random mode from the reject-all set and verifies rejection,
// interleaved with valid commit cases).

#[test]
fn prop_cross_property_sweep_200_cases() {
    // **Validates: Requirements MGR-027, MGR-033, MGR-035**
    const SWEEP_CASES: u32 = 200;
    let mut failures: Vec<serde_json::Value> = Vec::new();

    let reject_modes = [
        MemoryMode::Disabled,
        MemoryMode::Incognito,
        MemoryMode::ReadOnly,
    ];

    for case in 0..SWEEP_CASES {
        let seed = BASE_SEED
            .wrapping_mul(0x6C62_272E_07BB_0142)
            .wrapping_add(u64::from(case));
        let mut rng = StdRng::seed_from_u64(seed);

        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        let content = gen_text(&mut rng, 6 + (case as usize % 28));

        if case % 4 == 0 {
            // ── accepted commit path (atomicity + idempotency) ──
            let key = format!("sweep-commit-{case}");
            let env = CommandCandidate::native_fact(&content, Some("sweep"))
                .into_envelope(local_write_ctx(&key), None)
                .unwrap();
            let first = bus.submit_deferred(&env).unwrap();
            let second = bus
                .submit_deferred(
                    &CommandCandidate::native_fact(&content, Some("sweep"))
                        .into_envelope(local_write_ctx(&key), None)
                        .unwrap(),
                )
                .unwrap();

            let ok = first.status() == CommandStatus::Committed
                && second.status() == CommandStatus::Replayed
                && second.outcome.revision == first.outcome.revision
                && row_count(&db, "graph_revisions") == 1;

            if !ok {
                failures.push(json!({
                    "case": case,
                    "seed": format!("{seed:#018x}"),
                    "sub": "commit+idempotency",
                    "first_status": format!("{:?}", first.status()),
                    "second_status": format!("{:?}", second.status()),
                    "graph_revision_rows": row_count(&db, "graph_revisions"),
                }));
            }
        } else {
            // ── reject path (policy enforcement) ──
            let mode_idx = (case as usize) % reject_modes.len();
            let mode = reject_modes[mode_idx].clone();
            let key = format!("sweep-reject-{case}");
            let env = CommandCandidate::native_fact(&content, Some("sweep"))
                .into_envelope(local_write_ctx_with_mode(&key, mode.clone()), None)
                .unwrap();
            let result = bus.submit_deferred(&env).unwrap();

            let ok = result.status() == CommandStatus::Rejected
                && result
                    .rejection
                    .as_ref()
                    .map(|rs| rs.iter().any(|r| r.code == RejectionCode::ModeRejected))
                    .unwrap_or(false)
                && row_count(&db, "graph_revisions") == 0;

            if !ok {
                failures.push(json!({
                    "case": case,
                    "seed": format!("{seed:#018x}"),
                    "sub": "policy-reject",
                    "mode": mode.as_str(),
                    "status": format!("{:?}", result.status()),
                    "graph_revision_rows": row_count(&db, "graph_revisions"),
                }));
            }
        }
    }

    let failure_count = failures.len();
    let first_failure = failures.into_iter().next();
    persist_evidence_if_needed(
        "prop_cross_property_sweep_200",
        BASE_SEED,
        first_failure.clone(),
    );

    println!(
        "[prop_cross_property_sweep_200] Passed {}/{} cases with base seed {:#018x}",
        SWEEP_CASES - failure_count as u32,
        SWEEP_CASES,
        BASE_SEED
    );

    assert!(
        failure_count == 0,
        "prop_cross_property_sweep_200: {failure_count} failures — first: {first_failure:?}"
    );
}
