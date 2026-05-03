//! Predictive VRAM budget estimator — pre-flight checks before vision tool
//! execution to prevent reactive OOM panics from the watchdog.
//!
//! ## Problem
//! The `analyze_image` tool generates visual tokens that enter the KV cache,
//! consuming VRAM proportional to `n_tokens × kv_per_1k_ctx_mb / 1024`.
//! Without a pre-flight check, the watchdog only sees the VRAM drop *after*
//! the tokens are allocated — then kills the process.
//!
//! ## Solution
//! Before dispatching `analyze_image`, call `calculate_safe_visual_tokens()`
//! to compute the maximum token budget that fits in the current VRAM headroom.
//! Inject this as `hard_visual_token_cap` into the tool arguments so the
//! sidecar preprocessor physically cannot exceed the budget.

use crate::config::ModelProfile;

/// Estimated VRAM cost breakdown for a pending vision operation.
#[derive(Debug, Clone, Copy)]
pub struct VramBudget {
    /// Estimated visual tokens the image will produce.
    pub estimated_visual_tokens: u32,
    /// Maximum visual tokens that fit in the current VRAM headroom.
    pub safe_visual_token_cap: u32,
    /// Whether the image must be downscaled to fit the budget.
    pub requires_downscale: bool,
    /// Suggested max resolution (width) if downscale is needed. 0 = no limit.
    pub suggested_max_width: u32,
}

/// Calculate the maximum number of visual tokens that can safely fit in
/// the current VRAM headroom without triggering the watchdog.
///
/// # Arguments
/// * `free_vram_mb` — current free VRAM from telemetry
/// * `safety_margin_mb` — VRAM held back for system stability
/// * `profile` — model profile for KV cache cost constants
/// * `current_ctx_used` — tokens already consumed in the KV cache
///
/// # Returns
/// Maximum visual tokens that fit. Returns 0 if no headroom exists.
pub fn calculate_safe_visual_tokens(
    free_vram_mb: u64,
    safety_margin_mb: u64,
    profile: &ModelProfile,
    current_ctx_used: u32,
) -> u32 {
    let headroom = free_vram_mb.saturating_sub(safety_margin_mb);
    if headroom == 0 || profile.kv_per_1k_ctx_mb == 0 {
        return 0;
    }

    // How many total tokens can fit in the available headroom?
    let total_tokens_from_headroom =
        (headroom * 1024) / profile.kv_per_1k_ctx_mb as u64;

    // Subtract tokens already in use
    let available_token_budget =
        total_tokens_from_headroom.saturating_sub(current_ctx_used as u64);

    // Cap at a sane maximum (no image should ever generate more than 4096 visual tokens)
    available_token_budget.min(4096) as u32
}

/// Estimate how many visual tokens an image of the given resolution will produce.
///
/// ViT-based vision projectors (used by Qwen2.5-VL, LLaVA, etc.) divide the
/// image into patches of `patch_size × patch_size` pixels. Each patch becomes
/// one visual token.
///
/// Default patch size is 14 (standard ViT-L/14).
pub fn estimate_visual_tokens(width: u32, height: u32, patch_size: u32) -> u32 {
    let patch = patch_size.max(1);
    let patches_w = (width + patch - 1) / patch;
    let patches_h = (height + patch - 1) / patch;
    patches_w * patches_h
}

/// Full pre-flight check: estimate cost, compare to budget, recommend action.
pub fn preflight_vision_check(
    image_width: u32,
    image_height: u32,
    free_vram_mb: u64,
    safety_margin_mb: u64,
    profile: &ModelProfile,
    current_ctx_used: u32,
) -> VramBudget {
    let patch_size = 14u32; // ViT-L/14 default
    let estimated = estimate_visual_tokens(image_width, image_height, patch_size);
    let safe_cap = calculate_safe_visual_tokens(
        free_vram_mb,
        safety_margin_mb,
        profile,
        current_ctx_used,
    );

    let requires_downscale = estimated > safe_cap && safe_cap > 0;

    // If downscale needed, compute the max width that fits the budget.
    // Maintain aspect ratio: tokens ∝ (width/patch)*(height/patch).
    // For a given cap: max_pixels = cap * patch² → max_width = sqrt(max_pixels * aspect)
    let suggested_max_width = if requires_downscale && safe_cap > 0 {
        let aspect = image_width as f64 / image_height.max(1) as f64;
        let max_pixels = safe_cap as f64 * (patch_size * patch_size) as f64;
        let max_w = (max_pixels * aspect).sqrt();
        // Round down to nearest multiple of patch_size for clean division
        let rounded = (max_w as u32 / patch_size) * patch_size;
        rounded.max(patch_size) // At minimum one patch wide
    } else {
        0
    };

    VramBudget {
        estimated_visual_tokens: estimated,
        safe_visual_token_cap: safe_cap,
        requires_downscale,
        suggested_max_width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelProfile;

    fn test_profile() -> ModelProfile {
        ModelProfile {
            kv_per_1k_ctx_mb: 100,
            ..ModelProfile::default()
        }
    }

    #[test]
    fn estimate_tokens_1024x1024() {
        // 1024/14 = 73.1 → 74 patches per side → 74*74 = 5476 tokens
        let tokens = estimate_visual_tokens(1024, 1024, 14);
        assert_eq!(tokens, 74 * 74);
    }

    #[test]
    fn estimate_tokens_512x512() {
        // 512/14 = 36.6 → 37 patches per side → 37*37 = 1369 tokens
        let tokens = estimate_visual_tokens(512, 512, 14);
        assert_eq!(tokens, 37 * 37);
    }

    #[test]
    fn estimate_tokens_256x256() {
        let tokens = estimate_visual_tokens(256, 256, 14);
        // 256/14 = 18.3 → 19 patches → 19*19 = 361
        assert_eq!(tokens, 19 * 19);
    }

    #[test]
    fn safe_tokens_with_headroom() {
        let p = test_profile();
        // 500 MB free, 100 MB safety → 400 MB headroom
        // 400 * 1024 / 100 = 4096 tokens total, 0 used → cap = 4096
        let cap = calculate_safe_visual_tokens(500, 100, &p, 0);
        assert_eq!(cap, 4096);
    }

    #[test]
    fn safe_tokens_with_existing_ctx() {
        let p = test_profile();
        // 500 MB free, 100 MB safety → 400 MB → 4096 tokens
        // 2000 already used → 2096 remaining
        let cap = calculate_safe_visual_tokens(500, 100, &p, 2000);
        assert_eq!(cap, 2096);
    }

    #[test]
    fn safe_tokens_no_headroom() {
        let p = test_profile();
        let cap = calculate_safe_visual_tokens(50, 100, &p, 0);
        assert_eq!(cap, 0);
    }

    #[test]
    fn preflight_triggers_downscale() {
        let p = test_profile();
        // 1024x1024 → 5476 tokens, but budget only allows ~2000
        let budget = preflight_vision_check(1024, 1024, 300, 100, &p, 0);
        assert!(budget.requires_downscale);
        assert!(budget.suggested_max_width > 0);
        assert!(budget.suggested_max_width < 1024);
    }

    #[test]
    fn preflight_no_downscale_when_fits() {
        let p = test_profile();
        // 256x256 → 361 tokens, plenty of headroom
        let budget = preflight_vision_check(256, 256, 1000, 100, &p, 0);
        assert!(!budget.requires_downscale);
        assert_eq!(budget.suggested_max_width, 0);
    }

    fn get_6gb_profile() -> ModelProfile {
        let mut p = ModelProfile::default();
        p.kv_per_1k_ctx_mb = 100; // 100MB per 1024 tokens
        p
    }

    #[test]
    fn verify_6gb_profile_dynamic_caps() {
        let profile = get_6gb_profile();
        let safety_margin = 512; // 512MB margin
        
        // Scenario A: Cap at ~1369 tokens (512x512)
        // 1369 tokens require ~133MB headroom. 
        // 512 (safety) + 133 (headroom) = 645MB Free VRAM
        let cap_512 = calculate_safe_visual_tokens(645, safety_margin, &profile, 0);
        
        // 133MB * 1024 / 100 = 1361.92 tokens. 
        assert!(cap_512 >= 1361 && cap_512 <= 1362, "Math failed: expected ~1361 tokens, got {}", cap_512);

        // Scenario B: Cap at ~361 tokens (256x256)
        // 361 tokens require ~35MB headroom.
        // 512 (safety) + 35 (headroom) = 547MB Free VRAM
        let cap_256 = calculate_safe_visual_tokens(547, safety_margin, &profile, 0);
        
        // 35MB * 1024 / 100 = 358.4 tokens.
        assert!(cap_256 >= 358 && cap_256 <= 359, "Math failed: expected ~358 tokens, got {}", cap_256);
    }

    // ── New tests: edge cases and regression coverage ─────────────────

    #[test]
    fn estimate_tokens_zero_dimensions() {
        // Zero dimensions should still produce at least 1 patch per side
        // (the max(1) on patch_size prevents division-by-zero).
        assert_eq!(estimate_visual_tokens(0, 0, 14), 0);
        assert_eq!(estimate_visual_tokens(0, 512, 14), 0);
        assert_eq!(estimate_visual_tokens(512, 0, 14), 0);
    }

    #[test]
    fn estimate_tokens_zero_patch_size_defaults_to_one() {
        // patch_size = 0 is clamped to 1 internally.
        let tokens = estimate_visual_tokens(100, 100, 0);
        assert_eq!(tokens, 100 * 100); // 1×1 patches = pixel count
    }

    #[test]
    fn estimate_tokens_non_square() {
        // 1920×1080, patch=14: ceil(1920/14)=138, ceil(1080/14)=78 → 10764
        let tokens = estimate_visual_tokens(1920, 1080, 14);
        assert_eq!(tokens, 138 * 78);
    }

    #[test]
    fn safe_tokens_capped_at_4096() {
        let p = test_profile();
        // Massive headroom (10 GB free, 100 MB safety) would give
        // 9900 * 1024 / 100 = 101376 tokens — but the hard cap is 4096.
        let cap = calculate_safe_visual_tokens(10000, 100, &p, 0);
        assert_eq!(cap, 4096);
    }

    #[test]
    fn safe_tokens_exhausted_by_context() {
        let p = test_profile();
        // 500 MB free, 100 MB safety → 400 MB → 4096 token budget.
        // But 5000 tokens already in KV → saturating_sub yields 0.
        let cap = calculate_safe_visual_tokens(500, 100, &p, 5000);
        assert_eq!(cap, 0);
    }

    #[test]
    fn safe_tokens_zero_kv_cost_returns_zero() {
        let mut p = test_profile();
        p.kv_per_1k_ctx_mb = 0; // Zero KV cost → undefined → bail
        let cap = calculate_safe_visual_tokens(1000, 100, &p, 0);
        assert_eq!(cap, 0);
    }

    #[test]
    fn preflight_zero_budget_does_not_downscale() {
        let p = test_profile();
        // No VRAM headroom → safe_cap = 0. requires_downscale is false
        // because you can't downscale to 0; it's a hard block.
        let budget = preflight_vision_check(1024, 1024, 50, 100, &p, 0);
        assert!(!budget.requires_downscale);
        assert_eq!(budget.safe_visual_token_cap, 0);
        assert_eq!(budget.suggested_max_width, 0);
    }

    #[test]
    fn preflight_downscale_width_is_patch_aligned() {
        let p = test_profile();
        // 1920×1080 → lots of tokens, restricted budget
        let budget = preflight_vision_check(1920, 1080, 300, 100, &p, 0);
        if budget.requires_downscale {
            assert_eq!(budget.suggested_max_width % 14, 0,
                "suggested_max_width must be a multiple of patch_size (14)");
            assert!(budget.suggested_max_width >= 14,
                "suggested_max_width must be at least one patch wide");
        }
    }
}
