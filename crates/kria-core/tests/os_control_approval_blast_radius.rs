//! Reports how many canonical OS tools the policy engine gates behind approval.
//!
//! # Why this matters
//!
//! Approval-gated mutations currently fail with `grant_invalid: binding_mismatch`
//! (the grant minted after approval carries the decision-recorded parameters, which
//! do not hash-match the live request). Mutations that need **no** approval work.
//!
//! So the count below is the exact blast radius of that one bug — and it also shows
//! how many READS are being gated, which is a usability problem in its own right.

use kria_core::safety::policy::PolicyEngine;

/// Print the split so it appears in test output with `--nocapture`.
#[test]
fn report_approval_gating_across_the_whole_os_surface() {
    let engine = PolicyEngine::new();
    let tools = kria_core::os_control::frozen_tool_names();

    let mut gated_reads = Vec::new();
    let mut gated_mutations = Vec::new();
    let mut ungated = Vec::new();

    for tool in &tools {
        let decision = engine.evaluate(tool, &serde_json::json!({}));
        // A read, by naming convention across this surface.
        let is_read = tool.starts_with("get_")
            || tool.starts_with("list_")
            || tool.starts_with("search_")
            || tool.starts_with("scan_")
            || tool.starts_with("diagnose_")
            || tool.starts_with("plan_");
        if decision.requires_approval {
            if is_read {
                gated_reads.push(tool.clone());
            } else {
                gated_mutations.push(tool.clone());
            }
        } else {
            ungated.push(tool.clone());
        }
    }

    println!("\n=== OS tool approval gating ===");
    println!("total canonical tools      : {}", tools.len());
    println!("needs approval (mutations) : {}", gated_mutations.len());
    println!("needs approval (READS)     : {}", gated_reads.len());
    println!("no approval needed         : {}", ungated.len());
    println!("\nREADS wrongly gated (first 20):");
    for tool in gated_reads.iter().take(20) {
        println!("  {tool}");
    }
    println!("\nGated mutations (first 20):");
    for tool in gated_mutations.iter().take(20) {
        println!("  {tool}");
    }

    println!("\n=== the six areas tested live ===");
    for tool in [
        "set_volume", "get_audio_state", "set_brightness", "get_display_state",
        "set_night_light", "search_files", "get_wifi_networks",
        "get_bluetooth_state", "set_bluetooth_enabled",
    ] {
        let d = engine.evaluate(tool, &serde_json::json!({}));
        println!(
            "  {tool:26} approval={} risk={:?}",
            if d.requires_approval { "YES" } else { "no " },
            d.risk_level
        );
    }

    assert_eq!(
        tools.len(),
        149,
        "the frozen manifest must still hold 149 tools"
    );
}
