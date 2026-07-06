//! Process-global GPU policy tunables (redesign G1/G2).
//!
//! These three knobs were originally env-only (`KRIA_GPU_AUTOSCALE`, `KRIA_CUDA_RESERVE_MB`,
//! `KRIA_VRAM_VOLATILITY_CAP_MB`). They are now ALSO settable from the app's Settings UI via
//! [`apply_settings`], which the desktop calls from the loaded `OrchestratorConfig` at startup and
//! again whenever settings are saved — so a user can tune them without touching environment
//! variables or recompiling.
//!
//! Precedence: an explicitly-set environment variable always WINS (power-user / CI override);
//! otherwise the value last applied from config is used; otherwise a safe default.
//!
//! Reads are lock-free atomics so the hot paths (sizing, watchdog loop) pay no cost.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Default CUDA runtime reserve (MB): kernels, cuBLAS/cuDNN workspace, allocator fragmentation.
pub const DEFAULT_CUDA_RESERVE_MB: u64 = 1024;
/// Default volatility-reserve ceiling (MB): headroom for other GPU apps reclaiming VRAM.
pub const DEFAULT_VOLATILITY_CAP_MB: u64 = 1536;
/// Floor so a misconfigured tiny reserve can never invite an OOM.
const MIN_CUDA_RESERVE_MB: u64 = 256;

static AUTOSCALE: AtomicBool = AtomicBool::new(false);
static CUDA_RESERVE_MB: AtomicU64 = AtomicU64::new(DEFAULT_CUDA_RESERVE_MB);
static VOLATILITY_CAP_MB: AtomicU64 = AtomicU64::new(DEFAULT_VOLATILITY_CAP_MB);

/// Apply settings from config (Settings UI / config.toml). Clamps the CUDA reserve to a safe floor.
/// Called by the desktop at startup and on every settings save so changes take effect live.
pub fn apply_settings(autoscale: bool, cuda_reserve_mb: u64, volatility_cap_mb: u64) {
    AUTOSCALE.store(autoscale, Ordering::Relaxed);
    CUDA_RESERVE_MB.store(cuda_reserve_mb.max(MIN_CUDA_RESERVE_MB), Ordering::Relaxed);
    VOLATILITY_CAP_MB.store(volatility_cap_mb, Ordering::Relaxed);
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        )
    })
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
}

/// Whether the watchdog may opportunistically scale the LLM UP (a restart). Default OFF. Env
/// `KRIA_GPU_AUTOSCALE` overrides the config value.
pub fn autoscale_enabled() -> bool {
    env_bool("KRIA_GPU_AUTOSCALE").unwrap_or_else(|| AUTOSCALE.load(Ordering::Relaxed))
}

/// CUDA runtime VRAM reserve (MB). Env `KRIA_CUDA_RESERVE_MB` overrides the config value.
pub fn cuda_reserve_mb() -> u64 {
    env_u64("KRIA_CUDA_RESERVE_MB")
        .unwrap_or_else(|| CUDA_RESERVE_MB.load(Ordering::Relaxed))
        .max(MIN_CUDA_RESERVE_MB)
}

/// Volatility-reserve ceiling (MB). Env `KRIA_VRAM_VOLATILITY_CAP_MB` overrides the config value.
pub fn volatility_cap_mb() -> u64 {
    env_u64("KRIA_VRAM_VOLATILITY_CAP_MB")
        .unwrap_or_else(|| VOLATILITY_CAP_MB.load(Ordering::Relaxed))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Tests that mutate the process-global tunables must hold this lock so they don't race the
    /// sizing tests in `strategy.rs` (which also lock it) under the parallel test runner.
    pub(crate) static SETTINGS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn apply_clamps_cuda_reserve_to_floor() {
        let _g = SETTINGS_TEST_LOCK.lock().unwrap();
        apply_settings(true, 10, 800);
        // env not set in this test → config value used, clamped to floor.
        if std::env::var("KRIA_CUDA_RESERVE_MB").is_err() {
            assert_eq!(cuda_reserve_mb(), MIN_CUDA_RESERVE_MB);
        }
        if std::env::var("KRIA_VRAM_VOLATILITY_CAP_MB").is_err() {
            assert_eq!(volatility_cap_mb(), 800);
        }
        if std::env::var("KRIA_GPU_AUTOSCALE").is_err() {
            assert!(autoscale_enabled());
        }
        // restore defaults so other tests are unaffected
        apply_settings(false, DEFAULT_CUDA_RESERVE_MB, DEFAULT_VOLATILITY_CAP_MB);
    }
}
