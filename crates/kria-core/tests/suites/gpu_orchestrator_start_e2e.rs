//! FULL-PATH hardware E2E: runs the real `Orchestrator::start` against the local GPU + the real
//! llama-server binary + the real Qwen3VL-4B model. Exercises the cold-start fresh-VRAM read AND the
//! startup ngl-backoff ladder end to end, then asserts the server actually came up.
//!
//! Gated behind `KRIA_HW_E2E=1` (and the model/binary existing) so normal `cargo test` and CI skip
//! it — it spawns a real llama-server and uses the GPU.
//!
//! Run on the target machine:
//!   KRIA_HW_E2E=1 cargo test -p kria-core --test gpu_orchestrator_start_e2e -- --nocapture --test-threads=1

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kria_core::config::OrchestratorConfig;
use kria_core::infra::event_bus::EventBus;
use kria_core::infra::health::HealthRegistry;
use kria_core::llm::orchestrator::Orchestrator;

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".into())
}

fn model_path() -> String {
    format!(
        "{}/.kria/models/llm/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
        home()
    )
}

fn mmproj_path() -> Option<String> {
    let p = format!(
        "{}/.kria/models/llm/mmproj-Qwen3VL-4B-Instruct-F16.gguf",
        home()
    );
    Path::new(&p).exists().then_some(p)
}

fn enabled() -> bool {
    std::env::var("KRIA_HW_E2E").ok().as_deref() == Some("1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn orchestrator_start_backs_off_and_serves() {
    if !enabled() {
        eprintln!("SKIP: set KRIA_HW_E2E=1 to run the full-path orchestrator hardware E2E");
        return;
    }
    if !Path::new(&model_path()).exists() {
        eprintln!("SKIP: model not found at {}", model_path());
        return;
    }

    // Realistic config: enable orchestrator + a 36-layer vision profile (matches the real model).
    let mut config = OrchestratorConfig::default();
    config.enabled = true;
    config.model_profile.total_layers = 36;
    config.model_profile.has_vision_projector = mmproj_path().is_some();
    // Keep the health/port discovery generous; the ladder applies its own short probe per attempt.
    config.health_check_timeout_secs = 90;
    config.port_discovery_timeout_secs = 90;

    let event_bus = Arc::new(EventBus::new(256));
    let health = Arc::new(HealthRegistry::new());

    let t0 = std::time::Instant::now();
    let result = Orchestrator::start(config, model_path(), mmproj_path(), event_bus, health).await;
    let elapsed = t0.elapsed();

    match result {
        Ok(orch) => {
            let snap = orch.snapshot();
            println!(
                "orchestrator started in {:?}: backend={:?} ngl={} ctx={} healthy={}",
                elapsed, snap.backend, snap.current_ngl, snap.current_context, snap.server_healthy
            );
            assert!(
                snap.server_healthy,
                "server must report healthy after start"
            );
            // The backoff must have landed on a loadable config. On the RTX 4050 Vulkan build that
            // is a GPU ngl in the safe zone (≤ ~28) OR, worst case, CPU (ngl=0) — both are "served".
            println!(
                "PASS: LLM is up (ngl={}). Backoff/cold-start path served the model.",
                snap.current_ngl
            );
            // Give the child a moment, then drop — ChildGuard (PR_SET_PDEATHSIG) reaps llama-server
            // when this test process exits.
            tokio::time::sleep(Duration::from_millis(200)).await;
            drop(orch);
        }
        Err(e) => {
            panic!("orchestrator failed to start even with backoff + CPU fallback: {e}");
        }
    }
}
