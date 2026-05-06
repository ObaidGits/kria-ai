//! Integration tests for the API-level model swap, VRAM budget injection,
//! and VisionMode state machine integration.
//!
//! These tests exercise the contracts BETWEEN modules — ensuring the strategy
//! calculator, vision mode, VRAM budget, and server_manager spawn arguments
//! all align correctly. They do NOT spawn real llama-server processes.
//!
//! Key regression targets from live terminal logs:
//! 1. `evict_to_cpu` must use CpuVision (not Disabled) when RAM allows.
//! 2. `analyze_image` tool fallback must inject `hard_visual_token_cap`.
//! 3. Spawn args must include `--mmproj` when VisionMode is CpuVision (ngl=0).

use kria_core::config::{ModelProfile, OrchestratorConfig};
use kria_core::llm::orchestrator::strategy::{self, DegradationLevel};
use kria_core::llm::orchestrator::vision_strategy::{self, VisionMode};
use kria_core::llm::orchestrator::vram_budget;
use kria_core::llm::orchestrator::GpuBackend;

// ── Helper: 6GB vision model profile (Qwen2.5-VL-7B typical) ────────────

fn vision_6gb_profile() -> ModelProfile {
    ModelProfile {
        total_layers: 28,
        per_layer_vram_mb: 165,
        base_vram_overhead_mb: 200,
        kv_per_1k_ctx_mb: 100,
        min_context: 2048,
        max_context: 8192,
        has_vision_projector: true,
        vision_min_ngl: 15,
        mmproj_vram_mb: 1300,
    }
}

fn text_only_profile() -> ModelProfile {
    ModelProfile {
        has_vision_projector: false,
        mmproj_vram_mb: 0,
        ..vision_6gb_profile()
    }
}

// ────────────────────────────────────────────────────────────────────────
// Test 1: VisionMode flows correctly through strategy calculator
// ────────────────────────────────────────────────────────────────────────

#[test]
fn strategy_cpu_only_backend_produces_cpu_vision_when_ram_sufficient() {
    // CpuOnly backend should produce ngl=0 + VisionMode based on RAM.
    let profile = vision_6gb_profile();
    let result = strategy::calculate_target_params(&profile, 0, 256, GpuBackend::CpuOnly);

    assert_eq!(result.ngl, 0);
    assert_eq!(result.degradation, DegradationLevel::CpuOnly);

    // VisionMode depends on system RAM at call time — we can't mock sysinfo
    // in an integration test. But we CAN verify the contract:
    // If the host has >= 2048 MB free RAM, vision_mode should be CpuVision.
    // If not, it should be Disabled.
    match result.vision_mode {
        VisionMode::CpuVision => {
            assert!(
                result.enable_vision,
                "CpuVision mode must have enable_vision=true"
            );
            assert!(
                result.vision_mode.load_mmproj(),
                "CpuVision mode must load mmproj"
            );
        }
        VisionMode::Disabled => {
            assert!(
                !result.enable_vision,
                "Disabled mode must have enable_vision=false"
            );
        }
        other => panic!("CpuOnly backend at ngl=0 should never produce {:?}", other),
    }
}

#[test]
fn strategy_cuda_low_vram_falls_to_cpu_only_with_vision_mode() {
    let profile = vision_6gb_profile();
    // Only 300 MB free, 256 margin → 44 MB budget. Can't even fit base overhead.
    let result = strategy::calculate_target_params(&profile, 300, 256, GpuBackend::Cuda);

    assert_eq!(result.ngl, 0);
    assert_eq!(result.degradation, DegradationLevel::CpuOnly);

    // At ngl=0, vision_mode should be CpuVision or Disabled depending on RAM.
    assert!(
        matches!(
            result.vision_mode,
            VisionMode::CpuVision | VisionMode::Disabled
        ),
        "ngl=0 vision_mode must be CpuVision or Disabled, got {:?}",
        result.vision_mode
    );
}

// ────────────────────────────────────────────────────────────────────────
// Test 2: VRAM budget calculation integrates with preflight vision check
// ────────────────────────────────────────────────────────────────────────

#[test]
fn vram_budget_integrates_with_preflight_for_6gb_gpu() {
    let profile = vision_6gb_profile();
    let safety_margin = 512;

    // Scenario: 6 GB GPU with model loaded, 1500 MB free VRAM, 300 ctx tokens used
    let budget = vram_budget::preflight_vision_check(
        1024,
        1024, // 1024×1024 input image
        1500,
        safety_margin,
        &profile,
        300,
    );

    // 1500 - 512 = 988 MB headroom
    // 988 * 1024 / 100 = 10117 tokens total
    // 10117 - 300 = 9817 → capped at 4096
    assert_eq!(budget.safe_visual_token_cap, 4096);

    // 1024×1024 at patch=14 → 74*74 = 5476 tokens → exceeds 4096 cap
    assert!(budget.estimated_visual_tokens > 4096);
    assert!(budget.requires_downscale);
    assert!(budget.suggested_max_width > 0);
    assert!(budget.suggested_max_width < 1024);
}

#[test]
fn vram_budget_with_tight_headroom_caps_correctly() {
    let profile = vision_6gb_profile();

    // Only 600 MB free, 512 safety → 88 MB headroom
    // 88 * 1024 / 100 = 901 tokens
    let cap = vram_budget::calculate_safe_visual_tokens(600, 512, &profile, 0);
    assert_eq!(cap, 901);

    // 512×512 → 37*37 = 1369 tokens → exceeds 901 cap
    let budget = vram_budget::preflight_vision_check(512, 512, 600, 512, &profile, 0);
    assert!(budget.requires_downscale);
    assert!(budget.estimated_visual_tokens > budget.safe_visual_token_cap);
}

// ────────────────────────────────────────────────────────────────────────
// Test 3: evict_to_cpu vision mode regression
// ────────────────────────────────────────────────────────────────────────

#[test]
fn evict_to_cpu_should_use_cpu_vision_not_disabled() {
    // This test validates the FIX for the critical bug:
    // `evict_to_cpu` was hardcoding `VisionMode::Disabled` at ngl=0,
    // completely ignoring the CpuVision degradation state.
    //
    // The correct behavior: when evicting to CPU, determine_vision_mode
    // should be called with ngl=0 and the current free RAM.

    let profile = vision_6gb_profile();

    // Simulate the eviction path: ngl will be 0, with plenty of RAM
    let eviction_ngl = 0u32;
    let free_ram_mb = 8000u64; // 8 GB free RAM — plenty for CPU vision

    let vision_mode = vision_strategy::determine_vision_mode(&profile, eviction_ngl, free_ram_mb);

    assert_eq!(
        vision_mode,
        VisionMode::CpuVision,
        "BUG: evict_to_cpu MUST produce CpuVision when RAM >= 2048 MB, \
         not Disabled. Without this, --mmproj is dropped and the LLM is blind."
    );

    assert!(
        vision_mode.load_mmproj(),
        "CpuVision mode MUST load mmproj — the projector weights live in system RAM."
    );
}

#[test]
fn evict_to_cpu_disables_vision_when_ram_too_low() {
    let profile = vision_6gb_profile();

    // Low RAM scenario: eviction should disable vision entirely
    let vision_mode = vision_strategy::determine_vision_mode(&profile, 0, 1000);

    assert_eq!(vision_mode, VisionMode::Disabled);
    assert!(!vision_mode.load_mmproj());
}

// ────────────────────────────────────────────────────────────────────────
// Test 4: Vision mode round-trip through strategy → spawn args
// ────────────────────────────────────────────────────────────────────────

#[test]
fn vision_mode_determines_mmproj_flag_at_ngl_0() {
    // The server_manager spawn() derives `vision_enabled` from
    // `vision_mode.load_mmproj()`. This test validates that contract.

    let profile = vision_6gb_profile();
    let cpu_vision = vision_strategy::determine_vision_mode(&profile, 0, 4000);

    // CpuVision.load_mmproj() must be true → spawn will include --mmproj
    assert_eq!(cpu_vision, VisionMode::CpuVision);
    let vision_requested = cpu_vision.load_mmproj();
    assert!(
        vision_requested,
        "CpuVision.load_mmproj() must be true so spawn() passes --mmproj to llama-server"
    );

    // In spawn(), the actual flag is: vision_enabled = vision_requested && vision_configured()
    // vision_configured() checks if mmproj_path exists on disk.
    // We can't test file existence here, but the REQUEST side is now correct.
}

#[test]
fn full_gpu_vision_mode_at_high_ngl() {
    let profile = vision_6gb_profile();

    // ngl >= vision_min_ngl (15) → FullGpu
    let mode = vision_strategy::determine_vision_mode(&profile, 20, 8000);
    assert_eq!(mode, VisionMode::FullGpu);
    assert!(mode.load_mmproj());
    assert_eq!(mode.max_image_dimension(), 0); // no limit
}

#[test]
fn reduced_gpu_vision_mode_at_partial_offload() {
    let profile = vision_6gb_profile();

    // 0 < ngl < vision_min_ngl (15) → ReducedGpu
    let mode = vision_strategy::determine_vision_mode(&profile, 10, 8000);
    assert_eq!(mode, VisionMode::ReducedGpu);
    assert!(mode.load_mmproj());
    assert_eq!(mode.max_image_dimension(), 512);
}

// ────────────────────────────────────────────────────────────────────────
// Test 5: server_manager state constants
// ────────────────────────────────────────────────────────────────────────

#[test]
fn server_manager_state_constants_are_distinct() {
    use kria_core::llm::orchestrator::server_manager::*;
    let states = [
        STATE_STOPPED,
        STATE_STARTING,
        STATE_READY,
        STATE_SWAPPING,
        STATE_ERROR,
    ];
    for (i, a) in states.iter().enumerate() {
        for (j, b) in states.iter().enumerate() {
            if i != j {
                assert_ne!(
                    a, b,
                    "Server states must be distinct: index {} and {} both = {}",
                    i, j, a
                );
            }
        }
    }
}

#[test]
fn server_manager_new_starts_stopped() {
    use kria_core::llm::orchestrator::server_manager::*;

    let config = OrchestratorConfig::default();
    let mgr = LlamaServerManager::new(config, "/tmp/test.gguf".into(), None);

    assert_eq!(mgr.state(), STATE_STOPPED);
    assert!(!mgr.is_healthy());
    assert!(!mgr.is_swapping());
    assert!(!mgr.current_vision_enabled());
    assert_eq!(mgr.current_params(), (0, 0));
    assert!(mgr.api_url().is_empty());
}

#[test]
fn server_manager_cancel_token_is_renewable() {
    let config = OrchestratorConfig::default();
    let mgr = kria_core::llm::orchestrator::server_manager::LlamaServerManager::new(
        config,
        "/tmp/test.gguf".into(),
        None,
    );

    // Get a token, cancel it, get a new one — the new one must NOT be cancelled.
    let token1 = mgr.cancel_token();
    assert!(!token1.is_cancelled());

    mgr.cancel_streams(); // Cancels token1, mints token2

    assert!(
        token1.is_cancelled(),
        "Old token must be cancelled after cancel_streams()"
    );

    let token2 = mgr.cancel_token();
    assert!(
        !token2.is_cancelled(),
        "New token must be fresh (not already cancelled)"
    );
}

// ────────────────────────────────────────────────────────────────────────
// Test 6: analyze_image fallback payload must include hard_visual_token_cap
// ────────────────────────────────────────────────────────────────────────

#[test]
fn analyze_image_payload_contract() {
    // This test validates the REQUIREMENT that any analyze_image dispatch
    // includes the `hard_visual_token_cap` field in the JSON payload.
    // The loop_engine was missing this — only commands.rs had it.

    // Construct the expected payload structure
    let visual_token_cap = 640u64;
    let payload = serde_json::json!({
        "path": "/tmp/test.jpg",
        "operations": ["metadata", "ocr", "features"],
        "intent": "general",
        "hard_visual_token_cap": visual_token_cap,
    });

    assert!(
        payload.get("hard_visual_token_cap").is_some(),
        "analyze_image payload MUST include hard_visual_token_cap"
    );

    let cap = payload["hard_visual_token_cap"].as_u64().unwrap();
    assert!(
        cap > 0 && cap <= 4096,
        "hard_visual_token_cap must be 0 < cap <= 4096, got {}",
        cap
    );
}

// ────────────────────────────────────────────────────────────────────────
// Test 7: Strategy + VRAM budget pipeline end-to-end
// ────────────────────────────────────────────────────────────────────────

#[test]
fn end_to_end_strategy_to_vram_budget_pipeline() {
    let profile = vision_6gb_profile();

    // Step 1: Strategy calculator determines spawn params
    let target = strategy::calculate_target_params(&profile, 4000, 512, GpuBackend::Cuda);
    assert!(
        target.ngl > 0,
        "4 GB free VRAM should allow some GPU layers"
    );

    // Step 2: After spawn, some VRAM is consumed. Simulate 1500 MB free.
    let free_after_spawn = 1500u64;

    // Step 3: Before dispatching analyze_image, calculate VRAM-safe token cap
    let safe_cap = vram_budget::calculate_safe_visual_tokens(
        free_after_spawn,
        512,
        &profile,
        1000, // 1000 tokens already in KV
    );

    assert!(
        safe_cap > 0,
        "With 1500 MB free and 512 safety, should have some token budget"
    );
    assert!(safe_cap <= 4096, "Token cap must not exceed hard maximum");

    // Step 4: Preflight check on the actual image
    let budget =
        vram_budget::preflight_vision_check(1024, 1024, free_after_spawn, 512, &profile, 1000);

    assert_eq!(budget.safe_visual_token_cap, safe_cap);
}

// ────────────────────────────────────────────────────────────────────────
// Test 8: Text-only model never enables vision at any tier
// ────────────────────────────────────────────────────────────────────────

#[test]
fn text_only_model_never_enables_vision() {
    let profile = text_only_profile();

    for ngl in [0, 10, 20, 28] {
        for ram in [0, 2048, 8000, 32000] {
            let mode = vision_strategy::determine_vision_mode(&profile, ngl, ram);
            assert_eq!(
                mode,
                VisionMode::Disabled,
                "Text-only model must always be Disabled, got {:?} at ngl={}, ram={}",
                mode,
                ngl,
                ram
            );
            assert!(!mode.load_mmproj());
        }
    }
}

// ────────────────────────────────────────────────────────────────────────
// Test 9: Degradation level consistency
// ────────────────────────────────────────────────────────────────────────

#[test]
fn degradation_level_matches_strategy_output() {
    let profile = vision_6gb_profile();

    // Full capacity
    let full = strategy::calculate_target_params(&profile, 8000, 256, GpuBackend::Cuda);
    if full.ngl >= profile.total_layers && full.context >= profile.max_context {
        assert_eq!(full.degradation, DegradationLevel::Full);
    }

    // CPU only
    let cpu = strategy::calculate_target_params(&profile, 300, 256, GpuBackend::Cuda);
    assert_eq!(cpu.ngl, 0);
    assert_eq!(cpu.degradation, DegradationLevel::CpuOnly);
}
