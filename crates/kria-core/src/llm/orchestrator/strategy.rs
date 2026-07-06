//! Layer strategy calculator — determines optimal (ngl, context, vision)
//! parameters given available VRAM and model profile.

use super::vision_strategy::{self, VisionMode};
use super::GpuBackend;
use crate::config::ModelProfile;

/// Degradation level representing how much the model is operating below
/// its optimal configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    /// All layers on GPU, full context, vision enabled.
    Full,
    /// All layers on GPU but context is reduced.
    ReducedContext,
    /// Some layers offloaded to CPU.
    PartialOffload,
    /// Heavy CPU offload, reduced context.
    HeavyOffload,
    /// Full CPU inference (ngl=0).
    CpuOnly,
}

impl DegradationLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ReducedContext => "reduced_context",
            Self::PartialOffload => "partial_offload",
            Self::HeavyOffload => "heavy_offload",
            Self::CpuOnly => "cpu_only",
        }
    }
}

impl std::fmt::Display for DegradationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Calculated target parameters for a llama-server spawn.
#[derive(Debug, Clone)]
pub struct TargetParams {
    /// Number of GPU layers to offload.
    pub ngl: u32,
    /// Context window size in tokens.
    pub context: u32,
    /// Whether to enable vision (load mmproj).
    /// **Deprecated**: prefer `vision_mode`. Kept for backward compatibility
    /// with consumers that haven't been migrated yet.
    pub enable_vision: bool,
    /// Multi-tiered vision capability — replaces the binary enable_vision flag.
    pub vision_mode: VisionMode,
    /// Current degradation level.
    pub degradation: DegradationLevel,
}

/// Calculate optimal llama-server parameters given available VRAM.
///
/// Algorithm:
/// 1. Reserve `safety_margin_mb` from available VRAM
/// 2. Subtract base overhead (CUDA context + embeddings)
/// 3. Maximize ngl from remaining budget
/// 4. Allocate remaining VRAM to context window
/// 5. Determine VisionMode tier based on ngl and free RAM
/// CUDA runtime VRAM reserve (MB) — kernels, cuBLAS/cuDNN workspaces, and allocator fragmentation
/// that `base_vram_overhead_mb` does not capture. Production callers add this to `safety_margin_mb`
/// before calling [`calculate_target_params`], so the sizer leaves real headroom and does not
/// over-commit a small GPU (the ngl=36-on-6GB OOM/timeout loop). Override with
/// `KRIA_CUDA_RESERVE_MB` to tune during hardware validation without a recompile.
pub fn cuda_runtime_reserve_mb() -> u64 {
    super::gpu_policy::cuda_reserve_mb()
}

/// Production sizing entry point: identical to [`calculate_target_params`] but folds the CUDA
/// runtime reserve into the safety margin so the live orchestrator/watchdog never over-commit a
/// small GPU. Unit tests call the pure `calculate_target_params` directly (no reserve) to assert
/// the deterministic math; production code calls this.
pub fn calculate_target_params_prod(
    profile: &ModelProfile,
    free_vram_mb: u64,
    safety_margin_mb: u64,
    backend: GpuBackend,
) -> TargetParams {
    calculate_target_params(
        profile,
        free_vram_mb,
        safety_margin_mb.saturating_add(cuda_runtime_reserve_mb()),
        backend,
    )
}

// ── G1: measured-first sizing (redesign) ──────────────────────────────────────────────────────
//
// The desktop pain was sizing against a fluctuating *total/total-free* figure: Chrome/Discord/
// games change free VRAM constantly, so any reactive resize thrashed. The redesign sizes once, at
// load time, over a low-percentile ("sustained floor") of recent MEASURED free-VRAM readings minus
// a volatility reserve. We size for the floor and stay (lock, G3) — we do NOT chase transient peaks.

/// Default ceiling for the volatility reserve (MB). Settable via Settings UI / config
/// (`orchestrator.vram_volatility_cap_mb`); env `KRIA_VRAM_VOLATILITY_CAP_MB` overrides.
fn volatility_reserve_cap_mb() -> u64 {
    super::gpu_policy::volatility_cap_mb()
}

/// Lowest sample of a recent free-VRAM window — the "sustained floor" we size against. Empty input
/// returns 0 (caller will fall back to a single live reading). This is the conservative pick: on a
/// churny desktop the floor is well below the instantaneous peak, so we never size into memory that
/// another app is about to reclaim.
pub fn sustained_floor_mb(free_samples: &[u64]) -> u64 {
    free_samples.iter().copied().min().unwrap_or(0)
}

/// Volatility reserve (MB): headroom for other apps reclaiming VRAM. Derived from the *spread*
/// (max − min) of recent readings so it is near-zero on a stable/dedicated GPU and larger on a
/// churny desktop — i.e. it adapts to observed volatility rather than being hardcoded. We reserve
/// half the observed spread, clamped to a cap so a single outlier cannot starve sizing.
pub fn volatility_reserve_mb(free_samples: &[u64]) -> u64 {
    if free_samples.len() < 2 {
        return 0;
    }
    let min = free_samples.iter().copied().min().unwrap_or(0);
    let max = free_samples.iter().copied().max().unwrap_or(0);
    let spread = max.saturating_sub(min);
    (spread / 2).min(volatility_reserve_cap_mb())
}

/// Bounded calibration correction (MB) for the CUDA runtime overhead. `learned_overhead_mb` is the
/// real overhead observed on the first successful load (measured = reserved − actually-used). The
/// correction is clamped to ±50% of the configured default so calibration can refine sizing but can
/// never become the primary signal nor push sizing into an unsafe region (redesign G1).
pub fn calibrated_cuda_reserve_mb(learned_overhead_mb: Option<u64>) -> u64 {
    let default = cuda_runtime_reserve_mb();
    match learned_overhead_mb {
        None => default,
        Some(learned) => {
            let lo = default / 2; // −50%
            let hi = default.saturating_add(default / 2); // +50%
            learned.clamp(lo, hi)
        }
    }
}

/// Measured-first production sizing (redesign G1). Sizes over the *sustained floor* of recent
/// measured free-VRAM samples minus a telemetry-variance-derived volatility reserve, with a bounded
/// calibration correction folded into the safety margin. Falls back to the single `live_free_vram_mb`
/// reading when no history is available. This is the one-shot loader path; it is NOT a steady-state
/// loop (steady state = the Resident Lock, G3).
pub fn calculate_target_params_measured(
    profile: &ModelProfile,
    live_free_vram_mb: u64,
    free_samples: &[u64],
    safety_margin_mb: u64,
    learned_cuda_overhead_mb: Option<u64>,
    backend: GpuBackend,
) -> TargetParams {
    // CPU/Metal paths ignore the VRAM floor (handled inside calculate_target_params).
    if backend == GpuBackend::CpuOnly || backend == GpuBackend::Metal {
        return calculate_target_params(profile, live_free_vram_mb, safety_margin_mb, backend);
    }

    let floor = {
        let f = sustained_floor_mb(free_samples);
        if f == 0 {
            live_free_vram_mb
        } else {
            // Never size above the most recent live reading either (defensive).
            f.min(live_free_vram_mb.max(f))
        }
    };
    let reserve = volatility_reserve_mb(free_samples);
    let budget_free = floor.saturating_sub(reserve);
    let margin =
        safety_margin_mb.saturating_add(calibrated_cuda_reserve_mb(learned_cuda_overhead_mb));
    calculate_target_params(profile, budget_free, margin, backend)
}

pub fn calculate_target_params(
    profile: &ModelProfile,
    free_vram_mb: u64,
    safety_margin_mb: u64,
    backend: GpuBackend,
) -> TargetParams {
    // Query free system RAM for CPU vision feasibility check.
    // This is a fast syscall (~1μs) so it's safe to call inline.
    let free_ram_mb = {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.available_memory() / (1024 * 1024)
    };
    // macOS Metal: all layers are always offloaded (unified memory).
    // Only context is adjusted based on free RAM.
    if backend == GpuBackend::Metal {
        return calculate_metal_params(profile, free_vram_mb);
    }

    // CPU-only: no GPU layers, max context. Vision still possible on CPU
    // if enough RAM is available for the mmproj weights.
    if backend == GpuBackend::CpuOnly {
        let vm = vision_strategy::determine_vision_mode(profile, 0, free_ram_mb);
        return TargetParams {
            ngl: 0,
            context: profile.max_context,
            enable_vision: vm.is_enabled(),
            vision_mode: vm,
            degradation: DegradationLevel::CpuOnly,
        };
    }

    // CUDA path: VRAM budget calculation. The CUDA runtime reserve (kernels, cuBLAS workspace,
    // allocator fragmentation) is folded into `safety_margin_mb` by the production callers via
    // [`cuda_runtime_reserve_mb`] — keeping this function a pure, deterministic sizer.
    let available = free_vram_mb.saturating_sub(safety_margin_mb);

    // Reserve VRAM for vision projector (mmproj) when present
    let mmproj_cost = if profile.has_vision_projector {
        profile.mmproj_vram_mb as u64
    } else {
        0
    };

    // Not enough for even base overhead + mmproj → CPU only.
    // Vision may still be possible via CPU RAM.
    if available < profile.base_vram_overhead_mb as u64 + mmproj_cost {
        let vm = vision_strategy::determine_vision_mode(profile, 0, free_ram_mb);
        return TargetParams {
            ngl: 0,
            context: profile.min_context,
            enable_vision: vm.is_enabled(),
            vision_mode: vm,
            degradation: DegradationLevel::CpuOnly,
        };
    }

    let budget_after_base = available - profile.base_vram_overhead_mb as u64 - mmproj_cost;

    // Calculate max layers that fit
    let max_layers_from_budget = if profile.per_layer_vram_mb > 0 {
        (budget_after_base / profile.per_layer_vram_mb as u64) as u32
    } else {
        profile.total_layers
    };
    let ngl = max_layers_from_budget.min(profile.total_layers);

    // Remaining VRAM after layers → context
    let vram_used_by_layers = ngl as u64 * profile.per_layer_vram_mb as u64;
    let remaining_for_ctx = budget_after_base.saturating_sub(vram_used_by_layers);

    let context = if profile.kv_per_1k_ctx_mb > 0 {
        let ctx_from_vram = ((remaining_for_ctx * 1024) / profile.kv_per_1k_ctx_mb as u64) as u32;
        ctx_from_vram
            .max(profile.min_context)
            .min(profile.max_context)
    } else {
        profile.max_context
    };

    // Determine vision tier based on ngl and available system RAM.
    let vision_mode = vision_strategy::determine_vision_mode(profile, ngl, free_ram_mb);
    let enable_vision = vision_mode.is_enabled();

    let degradation = degradation_level(ngl, context, profile);

    TargetParams {
        ngl,
        context,
        enable_vision,
        vision_mode,
        degradation,
    }
}

/// Calculate parameters for Apple Silicon (Metal backend).
/// All layers are always on GPU (unified memory), context adapts to free RAM.
fn calculate_metal_params(profile: &ModelProfile, free_ram_mb: u64) -> TargetParams {
    let ngl = profile.total_layers; // Always full offload on Metal

    // Context scales with available RAM
    let context = if profile.kv_per_1k_ctx_mb > 0 {
        // Reserve ~2GB for system use
        let usable = free_ram_mb.saturating_sub(2048);
        let ctx = ((usable * 1024) / profile.kv_per_1k_ctx_mb as u64) as u32;
        ctx.max(profile.min_context).min(profile.max_context)
    } else {
        profile.max_context
    };

    // Metal always has full GPU layers, so vision is always FullGpu if projector exists.
    let vision_mode = vision_strategy::determine_vision_mode(profile, ngl, free_ram_mb);
    let enable_vision = vision_mode.is_enabled();
    let degradation = if context >= profile.max_context {
        DegradationLevel::Full
    } else {
        DegradationLevel::ReducedContext
    };

    TargetParams {
        ngl,
        context,
        enable_vision,
        vision_mode,
        degradation,
    }
}

/// Determine the degradation level from current ngl and context.
pub fn degradation_level(ngl: u32, context: u32, profile: &ModelProfile) -> DegradationLevel {
    if ngl == 0 {
        DegradationLevel::CpuOnly
    } else if ngl < profile.total_layers / 2 {
        DegradationLevel::HeavyOffload
    } else if ngl < profile.total_layers {
        if context < profile.max_context / 2 {
            DegradationLevel::HeavyOffload
        } else {
            DegradationLevel::PartialOffload
        }
    } else if context < profile.max_context {
        DegradationLevel::ReducedContext
    } else {
        DegradationLevel::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_profile() -> ModelProfile {
        ModelProfile {
            total_layers: 35,
            per_layer_vram_mb: 128,
            base_vram_overhead_mb: 200,
            kv_per_1k_ctx_mb: 100,
            min_context: 4096,
            max_context: 8192,
            has_vision_projector: true,
            mmproj_vram_mb: 0,
            vision_min_ngl: 15,
        }
    }

    #[test]
    fn full_vram_gives_full_params() {
        let p = test_profile();
        // 6GB free = 6144 MB
        let result = calculate_target_params(&p, 6144, 256, GpuBackend::Cuda);
        assert_eq!(result.ngl, 35); // All layers fit: (6144-256-200)/128 = 44 > 35
        assert!(result.context >= p.min_context);
        assert!(result.enable_vision);
        assert_eq!(result.degradation, DegradationLevel::Full);
    }

    #[test]
    fn low_vram_forces_cpu_only() {
        let p = test_profile();
        let result = calculate_target_params(&p, 300, 256, GpuBackend::Cuda);
        assert_eq!(result.ngl, 0);
        assert_eq!(result.context, p.min_context);
        assert!(matches!(
            result.vision_mode,
            VisionMode::CpuVision | VisionMode::Disabled
        ));
        assert_eq!(result.enable_vision, result.vision_mode.has_vision());
        assert_eq!(result.degradation, DegradationLevel::CpuOnly);
    }

    #[test]
    fn moderate_vram_gives_partial_offload() {
        let p = test_profile();
        // 3GB = 3072 MB. Budget = 3072-256-200 = 2616. Layers = 2616/128 = 20
        let result = calculate_target_params(&p, 3072, 256, GpuBackend::Cuda);
        assert!(result.ngl > 0 && result.ngl < 35);
        assert!(result.ngl >= 15); // Vision should be on
        assert!(result.enable_vision);
    }

    #[test]
    fn vision_disabled_below_ngl_15() {
        let p = test_profile();
        // ~2GB = 2048 MB. Budget = 2048-256-200 = 1592. Layers = 1592/128 = 12
        let result = calculate_target_params(&p, 2048, 256, GpuBackend::Cuda);
        assert!(result.ngl < 15);
        assert_eq!(result.vision_mode, VisionMode::ReducedGpu);
        assert!(result.enable_vision);
    }

    #[test]
    fn metal_always_full_layers() {
        let p = test_profile();
        let result = calculate_target_params(&p, 4096, 256, GpuBackend::Metal);
        assert_eq!(result.ngl, p.total_layers);
        assert!(result.enable_vision);
    }

    #[test]
    fn cpu_only_backend() {
        let p = test_profile();
        let result = calculate_target_params(&p, 8192, 256, GpuBackend::CpuOnly);
        assert_eq!(result.ngl, 0);
        assert!(matches!(
            result.vision_mode,
            VisionMode::CpuVision | VisionMode::Disabled
        ));
        assert_eq!(result.enable_vision, result.vision_mode.has_vision());
        assert_eq!(result.degradation, DegradationLevel::CpuOnly);
    }

    #[test]
    fn context_floor_enforced() {
        let p = test_profile();
        // Very low VRAM — context should still be >= min_context
        let result = calculate_target_params(&p, 512, 256, GpuBackend::Cuda);
        assert!(result.context >= p.min_context);
    }

    #[test]
    fn mmproj_vram_reduces_available_layers() {
        let mut p = test_profile();
        p.mmproj_vram_mb = 1300;
        // 6GB free = 6144 MB.
        // Without mmproj: budget = 6144-256-200 = 5688. Layers = 5688/128 = 44 → capped at 35
        // With mmproj:    budget = 6144-256-200-1300 = 4388. Layers = 4388/128 = 34
        let result = calculate_target_params(&p, 6144, 256, GpuBackend::Cuda);
        assert_eq!(result.ngl, 34);
        assert!(result.enable_vision);
    }

    // ── G1: measured-first sizing tests ──────────────────────────────────────

    #[test]
    fn sustained_floor_is_the_minimum_sample() {
        assert_eq!(sustained_floor_mb(&[6000, 4400, 5200, 4800]), 4400);
        assert_eq!(sustained_floor_mb(&[]), 0);
    }

    #[test]
    fn volatility_reserve_is_zero_on_stable_gpu() {
        // Dedicated/stable GPU: identical readings → no spread → no reserve.
        assert_eq!(volatility_reserve_mb(&[4400, 4400, 4400]), 0);
        assert_eq!(volatility_reserve_mb(&[4400]), 0);
    }

    #[test]
    fn volatility_reserve_grows_with_spread_and_is_capped() {
        let _g = super::super::gpu_policy::tests::SETTINGS_TEST_LOCK
            .lock()
            .unwrap();
        // spread 2000 → half = 1000, under cap.
        assert_eq!(volatility_reserve_mb(&[6000, 4000]), 1000);
        // Huge spread is clamped to the live cap (settable global; derive to stay stable).
        let cap = volatility_reserve_cap_mb();
        assert_eq!(volatility_reserve_mb(&[1_000_000, 0]), cap);
    }

    #[test]
    fn calibration_correction_is_bounded_to_50pct() {
        let _g = super::super::gpu_policy::tests::SETTINGS_TEST_LOCK
            .lock()
            .unwrap();
        // Bound is relative to the live default reserve (now a settable global), so derive it
        // rather than hardcoding — keeps the test stable regardless of process settings.
        let d = cuda_runtime_reserve_mb();
        let lo = d / 2;
        let hi = d.saturating_add(d / 2);
        assert_eq!(calibrated_cuda_reserve_mb(Some(0)), lo);
        assert_eq!(calibrated_cuda_reserve_mb(Some(u64::MAX)), hi);
        // a value inside the band passes through unchanged
        let mid = (lo + hi) / 2;
        assert_eq!(calibrated_cuda_reserve_mb(Some(mid)), mid);
        // None → default
        assert_eq!(calibrated_cuda_reserve_mb(None), d);
    }

    #[test]
    fn measured_sizing_uses_floor_minus_reserve_not_peak() {
        let _g = super::super::gpu_policy::tests::SETTINGS_TEST_LOCK
            .lock()
            .unwrap();
        let p = test_profile();
        // Live reading is a transient peak of 6000, but the floor is 4000 with a 2000 spread →
        // 1000 reserve → budget_free = 3000. Sizing must reflect the FLOOR, not the 6000 peak.
        let peak =
            calculate_target_params_measured(&p, 6000, &[6000, 4000], 256, None, GpuBackend::Cuda);
        // Compare to sizing directly at the floor-minus-reserve budget with default cuda reserve.
        let expected =
            calculate_target_params(&p, 3000, 256 + cuda_runtime_reserve_mb(), GpuBackend::Cuda);
        assert_eq!(peak.ngl, expected.ngl);
        assert!(
            peak.ngl < p.total_layers,
            "must not size for the transient peak"
        );
    }

    #[test]
    fn measured_sizing_falls_back_to_live_when_no_history() {
        let _g = super::super::gpu_policy::tests::SETTINGS_TEST_LOCK
            .lock()
            .unwrap();
        let p = test_profile();
        let r = calculate_target_params_measured(&p, 6144, &[], 256, None, GpuBackend::Cuda);
        let expected =
            calculate_target_params(&p, 6144, 256 + cuda_runtime_reserve_mb(), GpuBackend::Cuda);
        assert_eq!(r.ngl, expected.ngl);
    }

    #[test]
    fn measured_sizing_cpu_backend_ignores_vram_floor() {
        let p = test_profile();
        let r = calculate_target_params_measured(&p, 0, &[0, 0], 256, None, GpuBackend::CpuOnly);
        assert_eq!(r.ngl, 0);
        assert_eq!(r.degradation, DegradationLevel::CpuOnly);
    }
}
