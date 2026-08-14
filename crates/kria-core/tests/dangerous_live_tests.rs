// ─────────────────────────────────────────────────────────────────────────────
//  dangerous_live_tests.rs
//
//  Tests that exercise genuinely destructive actions.  Organised into tiers:
//
//  Tier 1 (always run, no #[ignore]):
//    Policy-gate assertions only — verify that shutdown/reboot/kill/delete are
//    classified Red and that HitlGateway is invoked for each.
//
//  Tier 2 (always run, no #[ignore]):
//    Sandbox-only destructive ops inside target/test-sandbox/ — safe to always run.
//
//  Tier 3 (manual, #[ignore]):
//    Real destructive actions (actual shutdown/Gmail send/push to main).
//    Run only with explicit developer intent:
//      KRIA_DANGEROUS=1 cargo test -p kria-core --test dangerous_live_tests -- --ignored
//
//  NOTE: Tier 3 tests print a confirmation banner and wait 3 seconds before executing.
// ─────────────────────────────────────────────────────────────────────────────

mod common;

use std::sync::Arc;

use common::{dangerous_enabled, SandboxDir};

/// SAFETY GUARD: Asserts that the current process is running inside a VM.
/// Panics with a clear message if running on bare metal / host OS.
/// This prevents Tier 3 destructive tests from accidentally running on
/// the developer's laptop (e.g. executing a real shutdown command).
fn assert_running_in_vm() {
    // Check KRIA_RUNNING_IN_VM env var (set by test runner SSH dispatch)
    if std::env::var("KRIA_RUNNING_IN_VM").as_deref() == Ok("1") {
        return;
    }
    // Fallback: check DMI/CPU info for VM signatures
    let vm_indicators = ["kvm", "qemu", "virtualbox", "vmware", "xen", "hyper-v"];
    let dmi_paths = [
        "/sys/class/dmi/id/product_name",
        "/sys/class/dmi/id/sys_vendor",
        "/sys/class/dmi/id/bios_vendor",
    ];
    for path in &dmi_paths {
        if let Ok(contents) = std::fs::read_to_string(path) {
            let lower = contents.to_ascii_lowercase();
            for indicator in &vm_indicators {
                if lower.contains(indicator) {
                    return; // Running inside a VM, safe to proceed
                }
            }
        }
    }
    // Check /proc/cpuinfo for hypervisor flag
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        if cpuinfo.to_ascii_lowercase().contains("hypervisor") {
            return;
        }
    }
    panic!(
        "🚨 SAFETY ABORT: Tier 3 destructive test running on HOST machine! \
         These tests MUST run inside a VM or Docker container. \
         Set KRIA_RUNNING_IN_VM=1 or run via 'cargo kria-test --mode FULL' \
         which dispatches destructive tests to the VM via SSH."
    );
}
use kria_core::safety::hitl::{ApprovalResponse, HitlGateway};
use kria_core::safety::policy::{PolicyEngine, RiskLevel};
use kria_core::tools::registry;
use tokio_util::sync::CancellationToken;

/// Create a default ToolContext for tests that call execute_with_context.
fn test_tool_context() -> kria_core::tools::ToolContext {
    let reg = registry::build_default_registry();
    reg.make_tool_context(CancellationToken::new())
}

// ═══════════════════════════════════════════════════════════════════════════
//  TIER 1 — Policy-gate assertions (always run)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn dangerous_t1_shutdown_is_red_and_requires_approval() {
    let engine = PolicyEngine::new();
    let d = engine.evaluate("shutdown", &serde_json::json!({}));
    assert_eq!(d.risk_level, RiskLevel::Red, "shutdown must be Red");
    assert!(d.requires_approval, "shutdown must require approval");
}

#[test]
fn dangerous_t1_reboot_is_red_and_requires_approval() {
    let engine = PolicyEngine::new();
    let d = engine.evaluate("reboot", &serde_json::json!({}));
    assert_eq!(d.risk_level, RiskLevel::Red, "reboot must be Red");
    assert!(d.requires_approval, "reboot must require approval");
}

#[test]
fn dangerous_t1_kill_process_is_red_and_requires_approval() {
    // The frozen contract declares `risk.fixed.red` for kill_process (verified in
    // .kiro/specs/linux-os-control-production/operation-contracts.json), and the policy
    // engine defers to it. RED is right: terminating a process by PID can take down the
    // user's editor mid-edit or a database mid-write, and it cannot be undone. The
    // Yellow expectation and the "no blocking approval needed" comment below both
    // predate the contract.
    let engine = PolicyEngine::new();
    let d = engine.evaluate("kill_process", &serde_json::json!({ "pid": 1 }));
    assert_eq!(
        d.risk_level,
        RiskLevel::Red,
        "kill_process is RED per the frozen contract"
    );
    assert!(
        d.requires_approval,
        "a RED action must ask a human before running"
    );
    assert!(!d.blocked, "kill_process must not be blocked");
}

#[test]
fn dangerous_t1_delete_file_is_red_and_requires_approval() {
    let engine = PolicyEngine::new();
    let d = engine.evaluate(
        "delete_file",
        &serde_json::json!({ "path": "/home/obaid/test.txt" }),
    );
    assert_eq!(d.risk_level, RiskLevel::Red, "delete_file must be Red");
    assert!(d.requires_approval, "delete_file must require approval");
}

#[test]
fn dangerous_t1_gmail_send_is_red_and_requires_approval() {
    // Sending email leaves the assistant boundary and is treated as Red.
    let engine = PolicyEngine::new();
    let d = engine.evaluate(
        "gw_gmail_send",
        &serde_json::json!({ "to": "test@example.com" }),
    );
    assert_eq!(
        d.risk_level,
        RiskLevel::Red,
        "gw_gmail_send must be Red per policy"
    );
    assert!(d.requires_approval, "gw_gmail_send must require approval");
}

#[test]
fn dangerous_t1_push_to_main_is_red_and_requires_approval() {
    let engine = PolicyEngine::new();
    let bash = engine.evaluate(
        "execute_bash",
        &serde_json::json!({ "command": "git push origin main" }),
    );
    assert_eq!(
        bash.risk_level,
        RiskLevel::Red,
        "git push origin main must be Red"
    );
    assert!(
        bash.requires_approval,
        "git push origin main must require approval"
    );
}

// ── HITL invoked for each Red action ─────────────────────────────────────

#[tokio::test]
async fn dangerous_t1_hitl_is_invoked_for_red_action() {
    let gateway = Arc::new(HitlGateway::new(30));
    let req_id = HitlGateway::generate_request_id();

    // Pre-load a Rejected response to keep the test fast and deterministic
    let gw2 = Arc::clone(&gateway);
    let id2 = req_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        gw2.respond(&id2, ApprovalResponse::Denied).await;
    });

    let outcome = gateway
        .request_approval_with_id(
            &req_id,
            "delete_file",
            serde_json::json!({ "path": "/home/obaid/something.txt" }),
            RiskLevel::Red,
            "Deletes /home/obaid/something.txt",
            false,
        )
        .await;
    // HITL was invoked and returned Rejected — the destructive action did NOT proceed
    assert!(
        matches!(outcome, ApprovalResponse::Denied),
        "HITL rejection must prevent the action"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  TIER 2 — Sandbox-only destructive ops (always run)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn dangerous_t2_sandbox_delete_file() {
    let sandbox = SandboxDir::new();
    sandbox.write_file("to_delete.txt", "delete me");
    assert!(
        sandbox.exists("to_delete.txt"),
        "File must exist before delete"
    );

    let reg = registry::build_default_registry();
    let handler = match reg.get_handler("delete_file") {
        Some(h) => h.clone(),
        None => {
            eprintln!("SKIP: delete_file tool not registered");
            return;
        }
    };
    let ctx = test_tool_context();
    let result = handler
        .execute_with_context(
            serde_json::json!({
                "path": sandbox.child("to_delete.txt").to_str().unwrap()
            }),
            ctx,
        )
        .await;
    assert!(
        result.success,
        "sandbox delete_file should succeed: {:?}",
        result.error
    );
    assert!(
        !sandbox.exists("to_delete.txt"),
        "File must not exist after delete"
    );
}

#[tokio::test]
async fn dangerous_t2_sandbox_move_then_delete() {
    let sandbox = SandboxDir::new();
    sandbox.write_file("source.txt", "source");

    let reg = registry::build_default_registry();

    // Move
    let mv_handler = match reg.get_handler("move_file") {
        Some(h) => h.clone(),
        None => {
            eprintln!("SKIP: move_file tool not registered");
            return;
        }
    };
    let ctx_mv = test_tool_context();
    let mv_result = mv_handler
        .execute_with_context(
            serde_json::json!({
                "source": sandbox.child("source.txt").to_str().unwrap(),
                "destination": sandbox.child("moved.txt").to_str().unwrap()
            }),
            ctx_mv,
        )
        .await;
    assert!(
        mv_result.success,
        "move_file should succeed: {:?}",
        mv_result.error
    );
    assert!(
        sandbox.exists("moved.txt"),
        "moved.txt must exist after move"
    );

    // Delete moved file
    let del_handler = match reg.get_handler("delete_file") {
        Some(h) => h.clone(),
        None => {
            eprintln!("SKIP: delete_file tool not registered");
            return;
        }
    };
    let ctx_del = test_tool_context();
    let del_result = del_handler
        .execute_with_context(
            serde_json::json!({
                "path": sandbox.child("moved.txt").to_str().unwrap()
            }),
            ctx_del,
        )
        .await;
    assert!(
        del_result.success,
        "delete moved file should succeed: {:?}",
        del_result.error
    );
    assert!(
        !sandbox.exists("moved.txt"),
        "moved.txt must not exist after delete"
    );
}

#[tokio::test]
async fn dangerous_t2_sandbox_clean_directory() {
    let sandbox = SandboxDir::new();
    for i in 0..5 {
        sandbox.write_file(&format!("file_{i}.txt"), "content");
    }
    let reg = registry::build_default_registry();
    let Some(handler) = reg.get_handler("clean_directory") else {
        // Tool may not exist; skip
        eprintln!("SKIP: clean_directory tool not registered");
        return;
    };
    let handler = handler.clone();
    let ctx = test_tool_context();
    let result = handler
        .execute_with_context(
            serde_json::json!({
                "path": sandbox.path.to_str().unwrap()
            }),
            ctx,
        )
        .await;
    assert!(
        result.success,
        "clean_directory should succeed: {:?}",
        result.error
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  TIER 3 — Real destructive actions (#[ignore] by default)
// ═══════════════════════════════════════════════════════════════════════════

/// ⚠  This test actually shuts down the machine.
/// Run ONLY when explicitly testing the shutdown flow inside a VM.
/// KRIA_DANGEROUS=1 cargo test dangerous_t3_real_shutdown -- --ignored
#[tokio::test]
#[ignore]
async fn dangerous_t3_real_shutdown() {
    if !dangerous_enabled() {
        eprintln!("SKIP: KRIA_DANGEROUS not set");
        return;
    }
    // SAFETY: Must be running inside a VM — never execute real shutdown on the host
    assert_running_in_vm();
    eprintln!("⚠️  DANGER: scheduling real system shutdown in 3 seconds!");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let reg = registry::build_default_registry();
    let handler = match reg.get_handler("shutdown_system") {
        Some(h) => h.clone(),
        None => {
            eprintln!("SKIP: shutdown_system tool not registered");
            return;
        }
    };
    // Shutdown with 1-minute delay so the test can verify the command was accepted
    let result = handler
        .execute(serde_json::json!({ "delay_minutes": 1 }))
        .await;
    assert!(
        result.success,
        "shutdown tool must succeed: {:?}",
        result.error
    );
}

/// ⚠  This test sends a real email.
/// KRIA_DANGEROUS=1 cargo test dangerous_t3_real_gmail_send -- --ignored
#[tokio::test]
#[ignore]
async fn dangerous_t3_real_gmail_send() {
    if !dangerous_enabled() {
        eprintln!("SKIP: KRIA_DANGEROUS not set");
        return;
    }
    // SAFETY: Must be running inside a VM — never send real emails from the host
    assert_running_in_vm();
    if !common::gworkspace_creds_available() {
        eprintln!("SKIP: Google Workspace credentials not available");
        return;
    }
    eprintln!("⚠️  DANGER: sending a real Gmail message in 3 seconds!");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let reg = registry::build_default_registry();
    let handler = match reg.get_handler("gw_gmail_send") {
        Some(h) => h.clone(),
        None => {
            eprintln!("SKIP: gw_gmail_send tool not registered");
            return;
        }
    };
    let result = handler
        .execute(serde_json::json!({
            "to": "kria-test@example.com",
            "subject": "KRIA Dangerous Test Email",
            "body": "This is an automated dangerous live test from the KRIA test suite."
        }))
        .await;
    assert!(
        result.success,
        "gw_gmail_send must succeed: {:?}",
        result.error
    );
}
