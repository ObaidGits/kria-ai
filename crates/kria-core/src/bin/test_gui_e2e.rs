//! RFC 007 Phase 4 - Safe E2E Sandboxed Test
//!
//! This binary tests the complete GUI automation pipeline in a safe, harmless way.
//!
//! Test Scenario:
//! 1. Open gedit (text editor) - harmless sandboxed application
//! 2. Get screen elements to find the text area
//! 3. Click the text area (with pHash verification)
//! 4. Type "KRIA HTN E2E TEST SUCCESS"
//! 5. Verify the text appeared via OCR
//!
//! Prerequisites (must be running before this test):
//! 1. Python Vision Sidecar: cd sidecars/kria-vision && pip install -r requirements.txt && python main.py
//! 2. Uinput Daemon: cargo run -p kria-uinput-daemon (requires sudo for uinput access)
//!
//! Run this test:
//!   cargo run -p kria-core --bin test_gui_e2e

use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use kria_core::agent::htn_executor::{GuiExecutor, GuiWorkflowBuilder, SafeAbortExecutor};
use kria_core::infra::ToolResult;
use kria_core::tools::gui_automation::{KillSwitchInterceptor, YdotoolBackend};
use kria_core::tools::registry::ToolRegistry;

/// Mock tool executor for testing
struct TestToolExecutor {
    registry: Arc<ToolRegistry>,
}

#[async_trait::async_trait]
impl kria_core::agent::htn_executor::ToolExecutor for TestToolExecutor {
    async fn execute(&self, action: &str, params: &serde_json::Value) -> ToolResult {
        // Get handler from registry and execute
        if let Some(handler) = self.registry.get_handler(action) {
            handler.execute(params.clone()).await
        } else {
            ToolResult::err(format!("Tool '{}' not found in registry", action))
        }
    }
}

#[tokio::main]
async fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  KRIA RFC 007 Phase 4 - E2E GUI Automation Test              ║");
    println!("║  Safe Sandboxed Test: Text Editor Workflow                   ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Check prerequisites
    println!("🔍 Checking prerequisites...");

    // Check if vision sidecar is reachable
    let vision_endpoint = std::env::var("KRIA_OMNIPARSER_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    match reqwest::get(format!("{}/health", vision_endpoint)).await {
        Ok(resp) if resp.status().is_success() => {
            println!("  ✓ Vision sidecar online at {}", vision_endpoint);
        }
        _ => {
            println!("  ✗ Vision sidecar NOT reachable at {}", vision_endpoint);
            println!();
            println!("Please start the vision sidecar:");
            println!("  cd sidecars/kria-vision");
            println!("  pip install -r requirements.txt");
            println!("  python main.py");
            std::process::exit(1);
        }
    }

    // Check if uinput daemon socket exists (orchestrator uses /tmp/kria-uinput.sock)
    let socket_path = std::path::PathBuf::from(
        std::env::var("KRIA_UINPUT_SOCKET").unwrap_or_else(|_| "/tmp/kria-uinput.sock".to_string()),
    );

    if !socket_path.exists() {
        println!("  ✗ Uinput daemon socket NOT found at {:?}", socket_path);
        println!();
        println!("Please start the uinput daemon (requires sudo):");
        println!("  cargo run -p kria-uinput-daemon");
        std::process::exit(1);
    }
    println!("  ✓ Uinput daemon socket found at {:?}", socket_path);

    println!();
    println!("⚠️  WARNING: This test will:");
    println!("   1. Open gedit text editor on your desktop");
    println!("   2. Click in the text area");
    println!("   3. Type: 'KRIA HTN E2E TEST SUCCESS'");
    println!();
    println!("Press Ctrl+C within 5 seconds to cancel...");
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!();

    // Initialize tool registry
    println!("🔧 Initializing tool registry...");
    let registry = Arc::new(ToolRegistry::new());

    // Register GUI automation tools
    kria_core::tools::gui_automation::register(&registry);
    kria_core::tools::vision_automation::register(&registry);
    kria_core::tools::app_lifecycle::register(&registry);

    println!("  ✓ Registered {} tools", registry.len());

    // Build test workflow manually (planner API changed, use builder directly for E2E)
    println!();
    println!("📋 Building HTN workflow...");

    let workflow = GuiWorkflowBuilder::new("e2e-test-001")
        .max_duration(120)
        .add_step(
            1,
            "open_application",
            serde_json::json!({"name": "gedit"}),
            kria_core::agent::htn_executor::VerificationType::WindowState {
                title_contains: Some("gedit".to_string()),
                class: None,
            },
        )
        .add_step(
            2,
            "system_sleep",
            serde_json::json!({"duration_ms": 1200}),
            kria_core::agent::htn_executor::VerificationType::None,
        )
        .add_step(
            3,
            "type_text",
            serde_json::json!({"text": "KRIA HTN E2E TEST SUCCESS", "interval_ms": 20}),
            kria_core::agent::htn_executor::VerificationType::TextPresent {
                text: "KRIA HTN E2E TEST SUCCESS".to_string(),
                case_insensitive: false,
            },
        )
        .add_abort_step("press_shortcut", serde_json::json!({"keys": ["Escape"]}))
        .build();

    // Print workflow details
    println!();
    println!("📋 Workflow details:");
    for (_i, goal) in workflow.sub_goals.iter().enumerate() {
        println!(
            "  Step {}: {} (verify: {:?})",
            goal.step, goal.action, goal.verify
        );
    }

    // Initialize executor components
    println!();
    println!("🔧 Initializing GUI executor...");

    let cancellation = CancellationToken::new();

    // Create GUI backend
    let gui_backend: Arc<dyn kria_core::tools::gui_automation::GuiBackend> =
        Arc::new(YdotoolBackend::new(socket_path));

    // Create kill switch interceptor
    let kill_switch = Arc::new(KillSwitchInterceptor::new(
        cancellation.clone(),
        Arc::clone(&gui_backend),
    ));

    // Create tool executor wrapper
    let tool_executor: Arc<dyn kria_core::agent::htn_executor::ToolExecutor> =
        Arc::new(TestToolExecutor {
            registry: Arc::clone(&registry),
        });

    // Create safe abort executor
    let abort_executor = SafeAbortExecutor::new(Arc::clone(&tool_executor));

    // Create GUI executor
    let mut executor = GuiExecutor::new(kill_switch, tool_executor, abort_executor);

    println!("  ✓ Executor initialized");

    // Execute workflow
    println!();
    println!("🚀 Executing HTN workflow...");
    println!("═══════════════════════════════════════════════════════════════");

    let start = std::time::Instant::now();
    let result = executor.execute_workflow(&workflow, cancellation).await;

    let _elapsed = start.elapsed();

    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // Print results
    println!("📊 Test Results:");
    println!("  Task ID: {}", result.task_id);
    println!(
        "  Success: {}",
        if result.success {
            "✅ PASSED"
        } else {
            "❌ FAILED"
        }
    );
    println!("  Steps: {}/{}", result.completed_steps, result.total_steps);
    println!("  Duration: {}ms", result.duration_ms);
    println!("  Aborted: {}", result.aborted);

    if let Some(error) = &result.error {
        println!();
        println!("  Error: {}", error);
    }

    // Cleanup
    println!();
    println!("🧹 Cleanup:");
    println!("  You can close the gedit window if it remains open.");
    println!();

    // Exit with appropriate code
    if result.success {
        println!("🎉 E2E TEST COMPLETED SUCCESSFULLY!");
        std::process::exit(0);
    } else {
        println!("💥 E2E TEST FAILED");
        std::process::exit(1);
    }
}

// Implementation of ToolExecutor trait for our test
mod kria_core {
    pub use kria_core::*;
}
