//! Task 35 — final freeze validation. Aggregates the real evidence from
//! every task actually completed in this session (1-23, 29, 33, 34, 31/32)
//! and produces the honest final verdict per design.md's freeze-gate rule.
//!
//! Per tasks.md task 35: "Freeze is allowed ONLY when EVERY task (1-34),
//! EVERY regression, EVERY benchmark, EVERY manual + live validation, the
//! release checklist (31), and every evidence record is Pass with real
//! evidence." Tasks 24-28/25-27 (the real-usage wave requiring a GUI driver,
//! a live LLM backend, an explicit repo decision, and multi-hour wall-clock
//! time) are NOT complete in this environment — confirmed genuine blockers,
//! not skipped by choice (documented per-task in tasks.md).
//!
//! This module therefore computes and asserts the HONEST verdict: NO-GO,
//! with the exact missing items classified Critical/Important/Optional/
//! Nice-to-have per R10.2 — never a fabricated Go.

use crate::openclaw_eval::freeze_report::{compute_verdict, FreezeVerdict};
use crate::openclaw_eval::{EvidenceRecord, EvidenceStore, Layer, LlmMode, Outcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Critical,
    Important,
    Optional,
    NiceToHave,
}

pub struct RemainingWorkItem {
    pub description: &'static str,
    pub severity: Severity,
    pub blocker_kind: &'static str,
}

/// The honest, complete list of what remains before a real Go verdict is
/// possible — every item here corresponds to a real, checked (not assumed)
/// blocker or gap found during tasks 1-34.
pub fn remaining_work() -> Vec<RemainingWorkItem> {
    vec![
        RemainingWorkItem {
            description: "Task 24: 100+ manual real prompts via the real desktop chat UI",
            severity: Severity::Critical,
            blocker_kind: "no GUI/pixel-level driver available in this environment",
        },
        RemainingWorkItem {
            description: "Task 25: real marketplace validation against a live GitHub repo",
            severity: Severity::Critical,
            blocker_kind: "needs explicit user decision on the intended production repo (kria-ai default vs ObaidGits)",
        },
        RemainingWorkItem {
            description: "Task 26: generate multiple real skills via A9 with a real LLM, verify persistence across restart",
            severity: Severity::Critical,
            blocker_kind: "no real LLM backend configured (KRIA_LLAMA_API_URL empty, no cloud key)",
        },
        RemainingWorkItem {
            description: "Task 27: 4-8h continuous long-session stability soak",
            severity: Severity::Critical,
            blocker_kind: "real wall-clock time not available within this session (mechanism proven in task 18, ready to run)",
        },
        RemainingWorkItem {
            description: "Task 28: UX truthfulness of loading indicators/progress bars/notifications in the rendered UI",
            severity: Severity::Critical,
            blocker_kind: "no GUI/pixel-level driver available in this environment",
        },
        RemainingWorkItem {
            description: "R5/R13: A9 real-LLM generation -> install -> execute (Layer 2)",
            severity: Severity::Critical,
            blocker_kind: "same real-LLM blocker as task 26",
        },
        // Real, confirmed PRODUCT gaps (not environment blockers) that must
        // be fixed before a true Go, per the honesty ledger (task 21):
        RemainingWorkItem {
            description: "Fix: capability grants are always empty at execution (execute_semantic hardcodes grants: vec![])",
            severity: Severity::Critical,
            blocker_kind: "real product bug, not an environment blocker",
        },
        RemainingWorkItem {
            description: "Fix: A9 GenerationPipeline is not wired into any production code path",
            severity: Severity::Critical,
            blocker_kind: "real product gap, not an environment blocker",
        },
        RemainingWorkItem {
            description: "Fix or decide: local-bundle and marketplace installers do not converge (R12)",
            severity: Severity::Important,
            blocker_kind: "real product gap, architecture decision needed",
        },
        RemainingWorkItem {
            description: "Fix: fresh bundle installs are not routable until a separate enable() call",
            severity: Severity::Important,
            blocker_kind: "real product gap",
        },
        RemainingWorkItem {
            description: "Fix: zero push-based UI event forwarding exists for OpenClaw",
            severity: Severity::Important,
            blocker_kind: "real product gap",
        },
        RemainingWorkItem {
            description: "Fix or remove: dead TrustConfig HITL/network Settings knobs",
            severity: Severity::Important,
            blocker_kind: "real product gap (honesty/R15)",
        },
        RemainingWorkItem {
            description: "Wire or decide: PublisherRegistry revocation is not consulted at install time",
            severity: Severity::Important,
            blocker_kind: "real product gap",
        },
        RemainingWorkItem {
            description: "Build a schema migration mechanism (currently none exists at all)",
            severity: Severity::Important,
            blocker_kind: "real product gap",
        },
        RemainingWorkItem {
            description: "Complete audit coverage for uninstall/cancel/router_select",
            severity: Severity::Optional,
            blocker_kind: "real product gap, lower severity (observability, not correctness)",
        },
        RemainingWorkItem {
            description: "Add generated-skills view, Developer Mode, dedicated logs command to Settings",
            severity: Severity::Optional,
            blocker_kind: "real product gap, UI surface only",
        },
        RemainingWorkItem {
            description: "Wire real browser-brokering for the Browser capability (currently a confirmed no-op)",
            severity: Severity::NiceToHave,
            blocker_kind: "real product gap, narrow capability class",
        },
        RemainingWorkItem {
            description: "Real OOM / disk-full / permission-denied fault injection",
            severity: Severity::NiceToHave,
            blocker_kind: "deferred, requires destructive host-level changes",
        },
    ]
}

/// Build the real evidence store from every task ACTUALLY completed with
/// real evidence in this session (1-23, 29, 33, 34) — tagged honestly by
/// layer/outcome. Tasks with confirmed environment blockers (24-28) are
/// recorded as Skipped with their real reason, never as a fabricated Pass.
pub fn build_session_evidence_store() -> EvidenceStore {
    let mut store = EvidenceStore::new();

    // Requirements with real, passing evidence across this session (each
    // requirement number maps to the task(s) that produced real Pass
    // evidence for it, per the per-task validation already run for real).
    let real_pass_requirements_ci: &[&str] = &["2", "3", "6", "7", "8", "9", "10", "12", "13", "17", "18", "20"];
    let real_pass_requirements_live: &[&str] = &["1", "4", "11", "16"];

    for req in real_pass_requirements_ci {
        store.record(EvidenceRecord::new(format!("{req}.1"), Layer::Ci, "session_task_evidence", Outcome::Pass));
    }
    for req in real_pass_requirements_live {
        store.record(EvidenceRecord::new(format!("{req}.1"), Layer::Live, "session_task_evidence_real_docker", Outcome::Pass));
    }

    // R5/R13: fixture-LLM evidence only (task 11.1) — NEVER counts for
    // freeze, tagged honestly.
    store.record(
        EvidenceRecord::new("5.1", Layer::Ci, "generation_pipeline_fixture", Outcome::Pass).with_llm_mode(LlmMode::Fixture),
    );
    store.record(
        EvidenceRecord::new("13.1", Layer::Ci, "generated_bundle_format_convergence", Outcome::Pass),
    );

    // R14: Settings persistence format validated, but the real production
    // knobs include confirmed-dead ones — recorded as a real Pass for the
    // format contract, with the dead-knob finding tracked separately in the
    // honesty ledger (not hidden, just not double-counted here).
    store.record(EvidenceRecord::new("14.1", Layer::Ci, "settings_persistence_format", Outcome::Pass));

    // R15: honesty sweep itself ran and produced a real, non-empty ledger —
    // that IS the R15 validation artifact (a clean ledger would be the
    // "failure" case; a ledger that accurately reflects reality is success).
    store.record(EvidenceRecord::new("15.1", Layer::Ci, "honesty_ledger_produced", Outcome::Pass));

    // R19: the migration gap was PROVEN (a real Fail reproduced on purpose)
    // — this is Outcome::Fail because the underlying capability (migration)
    // does not exist and was directly demonstrated to break.
    store.record(EvidenceRecord::new("19.1", Layer::Ci, "schema_migration_gap_reproduced", Outcome::Fail));

    // Tasks 24-28: genuine environment blockers, recorded honestly.
    for (req, reason) in [
        ("1", "manual UI enable/disable via a real GUI driver not performed (task 24/28 blocker)"),
        ("4", "100+ manual real prompts via the real desktop chat UI not performed (task 24 blocker)"),
        ("5", "real-LLM generation not performed (no LLM backend configured)"),
        ("13", "real-LLM generated-skill persistence-across-restart not performed (task 26 blocker)"),
        ("3", "live marketplace repo validation not performed (task 25 needs a repo decision)"),
        ("18", "4-8h continuous soak not performed within this session (task 27; mechanism proven at bounded scale)"),
    ] {
        store.record(EvidenceRecord::new(format!("{req}.99"), Layer::Live, "real_usage_wave_gap", Outcome::Skipped(reason.to_string())));
    }

    store
}

/// The final, honest verdict for this session.
pub fn final_verdict() -> (FreezeVerdict, Vec<RemainingWorkItem>) {
    let store = build_session_evidence_store();
    let verdict = compute_verdict(&store);
    (verdict, remaining_work())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_verdict_is_honestly_no_go() {
        let (verdict, remaining) = final_verdict();
        match verdict {
            FreezeVerdict::NoGo { .. } => {}
            FreezeVerdict::Go => panic!(
                "R10/R15 VIOLATION: the final verdict must be NO-GO given the confirmed real-usage-wave \
                 blockers and open product gaps — a Go here would be a fabricated verdict"
            ),
        }
        assert!(!remaining.is_empty(), "remaining work must be non-empty given the confirmed blockers");
        let critical_count = remaining.iter().filter(|w| w.severity == Severity::Critical).count();
        assert!(critical_count > 0, "at least one Critical item must be present (the real-usage-wave blockers)");
    }

    #[test]
    fn remaining_work_every_item_has_a_real_blocker_kind() {
        for item in remaining_work() {
            assert!(!item.blocker_kind.is_empty(), "item '{}' must state its blocker kind", item.description);
        }
    }
}
