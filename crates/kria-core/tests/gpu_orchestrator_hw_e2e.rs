//! Hardware-gated E2E for the GPU Hardware Orchestrator sizing + telemetry path.
//!
//! Runs on the REAL local GPU via the `nvidia-smi` CLI profiler (the same `CliVramProfiler` the
//! orchestrator uses under `--no-default-features`). Skips cleanly when no NVIDIA GPU is present.
//!
//! It proves the root-cause fix for the "first prompt → LLM not reachable" flap:
//!   * with REAL measured free VRAM, startup sizing picks a GPU config (ngl > 0);
//!   * with a cold-start ZERO reading (the bug), sizing would pick CPU (ngl == 0) → the flap source.
//!
//! Run: `cargo test -p kria-core --test gpu_orchestrator_hw_e2e -- --nocapture`

use kria_core::config::ModelProfile;
use kria_core::llm::orchestrator::strategy::{
    calculate_target_params_measured, calculate_target_params_prod,
};
use kria_core::llm::orchestrator::GpuBackend;
use kria_core::platform::vram::build_profiler;

/// Representative profile for the Qwen3VL-4B-Instruct Q4 vision model used on the target machine
/// (36 transformer layers + vision projector). Footprint constants are conservative approximations;
/// the assertions only depend on the monotonic "more free VRAM → more layers" property.
fn qwen3vl_4b_profile() -> ModelProfile {
    ModelProfile {
        total_layers: 36,
        per_layer_vram_mb: 95,
        base_vram_overhead_mb: 450,
        kv_per_1k_ctx_mb: 80,
        min_context: 4096,
        max_context: 8192,
        has_vision_projector: true,
        vision_min_ngl: 15,
        mmproj_vram_mb: 1000,
    }
}

#[tokio::test]
async fn real_gpu_sizing_lands_on_gpu_not_cpu() {
    let profiler = build_profiler();
    let snap = profiler.snapshot().await;

    if snap.total_mb == 0 {
        eprintln!("SKIP: no NVIDIA GPU / nvidia-smi unavailable (total_mb == 0) — CPU-only host");
        return;
    }

    println!(
        "REAL GPU telemetry: free={} MB, total={} MB (CliVramProfiler / nvidia-smi)",
        snap.free_mb, snap.total_mb
    );

    let profile = qwen3vl_4b_profile();
    let backend = GpuBackend::Cuda;

    // 1. The BUG path: a cold-start zero reading sizes the model onto the CPU (ngl == 0). This is
    //    exactly what the stale runtime did before the fix → endless watchdog scale-up restarts.
    let cold = calculate_target_params_prod(&profile, 0, 512, backend);
    println!(
        "cold-start (free=0) sizing → ngl={} ({})",
        cold.ngl, cold.degradation
    );
    assert_eq!(
        cold.ngl, 0,
        "zero VRAM must size to CPU — this is the flap trigger the fix removes"
    );

    // 2. The FIX path: sizing on the REAL measured free VRAM must land on the GPU (ngl > 0).
    let live = calculate_target_params_prod(&profile, snap.free_mb, 512, backend);
    println!(
        "real-VRAM (free={}) sizing → ngl={} ctx={} ({})",
        snap.free_mb, live.ngl, live.context, live.degradation
    );
    assert!(
        live.ngl > 0,
        "with {} MB free the model MUST size onto the GPU (ngl>0), not CPU",
        snap.free_mb
    );

    // 3. Measured-first sizing (G1) over a synthetic stable window sizes onto the GPU too, and never
    //    exceeds the layer count of the single-reading prod sizer (it reserves volatility headroom).
    let samples = [snap.free_mb, snap.free_mb, snap.free_mb];
    let measured =
        calculate_target_params_measured(&profile, snap.free_mb, &samples, 512, None, backend);
    println!(
        "measured-first (stable window) sizing → ngl={} ctx={} ({})",
        measured.ngl, measured.context, measured.degradation
    );
    assert!(
        measured.ngl > 0,
        "measured-first must also land on GPU on a stable window"
    );
    assert!(
        measured.ngl <= live.ngl,
        "measured-first reserves headroom → never MORE layers than the raw prod sizer"
    );

    println!("PASS: real-GPU sizing lands on GPU; cold-start zero correctly maps to CPU (the fixed flap trigger).");
}

#[tokio::test]
async fn real_gpu_volatility_reserve_is_conservative_on_churn() {
    let profiler = build_profiler();
    let snap = profiler.snapshot().await;
    if snap.total_mb == 0 {
        eprintln!("SKIP: no NVIDIA GPU available");
        return;
    }
    let profile = qwen3vl_4b_profile();
    let backend = GpuBackend::Cuda;

    // Simulate a churny desktop: free VRAM swings (other apps opening/closing). Measured-first must
    // size for the FLOOR (smallest free), not the peak — so it never sizes into memory another app
    // is about to reclaim (the desktop-thrash class).
    let floor = snap.free_mb.saturating_sub(1500).max(1);
    let churny = [snap.free_mb, floor, snap.free_mb, floor];
    let measured =
        calculate_target_params_measured(&profile, snap.free_mb, &churny, 512, None, backend);
    let peak = calculate_target_params_prod(&profile, snap.free_mb, 512, backend);
    println!(
        "churn window {:?}: measured ngl={} vs peak ngl={}",
        churny, measured.ngl, peak.ngl
    );
    assert!(
        measured.ngl <= peak.ngl,
        "on a churny window, measured-first must not size for the transient peak"
    );
    println!("PASS: volatility reserve keeps sizing conservative under VRAM churn.");
}
