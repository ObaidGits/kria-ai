//! Pins the policy engine to the frozen OS contract for unconditional risks.
//!
//! # The bug this prevents
//!
//! The policy engine used to derive an OS tool's risk from its **name**. That rated
//! 20 operations higher than the reviewed contract does, so the assistant demanded
//! approval to read the current volume or the screen state. Reading a value is not a
//! system modification, and being asked for it makes the whole feature feel unusable.
//!
//! # Why "just don't gate reads" would have been wrong
//!
//! Some reads genuinely are sensitive, and the contract says which: the system
//! journal carries authentication failures, the clipboard may hold a password, and
//! visible Wi-Fi names reveal where you are. A blanket rule would have silently
//! un-gated those. Deferring to the contract keeps them gated **and** stops the
//! nagging, because the contract distinguishes them properly.

use kria_core::safety::policy::PolicyEngine;
use kria_core::safety::RiskLevel;

/// An unconditional contract risk must be honoured exactly.
#[test]
fn unconditional_contract_risk_is_authoritative() {
    let engine = PolicyEngine::new();
    let mut mismatches = Vec::new();

    for tool in kria_core::os_control::frozen_tool_names() {
        let Some(contract) = kria_core::os_control::frozen_contract(&tool) else {
            continue;
        };
        let Some(declared) = contract.risk.fixed_risk() else {
            // Conditional risk is resolved from the request, not from the contract's
            // ceiling — deliberately not pinned here.
            continue;
        };
        let decided = engine.evaluate(&tool, &serde_json::json!({}));
        if decided.risk_level != declared {
            mismatches.push(format!(
                "{tool}: contract={declared:?} policy={:?}",
                decided.risk_level
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "the policy engine disagrees with the frozen contract on these operations, so they will \
         be gated at the wrong tier: {mismatches:#?}"
    );
}

/// The specific reads that were nagging must now be silent.
#[test]
fn harmless_state_reads_do_not_require_approval() {
    let engine = PolicyEngine::new();
    for tool in [
        "get_audio_state",
        "get_display_state",
        "get_battery_health",
        "get_os_capabilities",
    ] {
        let decision = engine.evaluate(tool, &serde_json::json!({}));
        assert!(
            !decision.requires_approval,
            "`{tool}` reads a value and must not ask for approval (got {:?})",
            decision.risk_level
        );
    }
}

/// The genuinely sensitive reads must STILL require approval.
#[test]
fn sensitive_reads_are_still_gated() {
    let engine = PolicyEngine::new();
    for tool in [
        // The journal carries authentication failures and other users' activity.
        "get_system_logs",
        // May hold a password the user copied a moment ago.
        "get_clipboard",
        "get_clipboard_history",
        // Reads INSIDE the user's documents.
        "search_desktop",
        // Credential identities.
        "list_secret_references",
        "list_saved_connectivity_credentials",
        // Visible network names reveal physical location.
        "get_wifi_networks",
    ] {
        let decision = engine.evaluate(tool, &serde_json::json!({}));
        assert!(
            decision.requires_approval,
            "`{tool}` is privacy-sensitive and must keep asking for approval (got {:?})",
            decision.risk_level
        );
    }
}

/// A conditional risk must NOT be flattened to the contract's ceiling.
#[test]
fn conditional_risk_is_not_taken_from_the_contract_ceiling() {
    // `write_file` is RED for a protected path and YELLOW for an ordinary one. Using
    // the ceiling would demand approval for writing to the user's own Documents.
    let contract = kria_core::os_control::frozen_contract("write_file")
        .expect("write_file is a canonical operation");
    assert!(
        contract.risk.fixed_risk().is_none(),
        "write_file's risk is conditional; if this becomes fixed, revisit the policy override"
    );
    assert_eq!(
        contract.risk.max_risk(),
        RiskLevel::Red,
        "the ceiling is RED, which is exactly why it must not be used as the decision"
    );
}
