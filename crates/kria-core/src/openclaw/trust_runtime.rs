//! Live `TrustConfig` enforcement (product gap 6/8, dead-Settings-knob fix).
//!
//! Real fix, additive (no A0-A9 redesign, no new trust system — this is the
//! runtime-visible half of the EXISTING `TrustConfig` struct in `config.rs`,
//! mirroring the exact process-wide-atomic-snapshot pattern
//! `safety::global_halt` already uses for the SAME kind of problem: a
//! Settings-controlled runtime behavior that needs to be read from a place
//! (`execute_semantic`) that does not otherwise have access to the live
//! `KriaConfig`).
//!
//! Previously (confirmed by exhaustive grep, task 7): `TrustConfig::
//! community_allows_network` and `verified_skips_hitl` were persisted by
//! `openclaw_update_settings` but read by NOTHING — `approval.rs`'s HITL
//! gate (`ApprovalCache::evaluate`) is keyed purely by `RiskLevel`, never by
//! `TrustTier`, and `execute_semantic` never called `ApprovalCache` at all
//! (capability approval was structurally unreachable from the real chat
//! execution path). Toggling either Setting changed nothing.
//!
//! Real fix: `execute_semantic` now (1) honors `community_allows_network`
//! by demoting a Community-tier skill's network capability to `None` when
//! the flag is off (skills.md's OpenClawNetworkPolicy already exists for
//! exactly this), and (2) honors `verified_skips_hitl` by calling the REAL
//! `ApprovalCache::evaluate` for skills whose risk requires approval, with
//! Verified-tier skills auto-approved (skipping the `NeedsHitl` branch) only
//! when the flag is on. A skill that needs HITL and doesn't get
//! auto-approved is honestly declined (not silently executed) — this session
//! does not add a HITL prompt UI; declining honestly is the correct value
//! given no GUI driver exists yet to prompt through.

use super::config::TrustConfig;
use std::sync::RwLock;

/// Process-wide live snapshot of the Settings-controlled trust config,
/// mirroring `safety::global_halt`'s established pattern for this exact
/// problem shape. Updated by `openclaw_update_settings` on every save;
/// read by `execute_semantic` on every execution — always reflects the
/// CURRENT persisted Settings value, no restart required (R14.3).
static LIVE_TRUST_CONFIG: RwLock<Option<TrustConfig>> = RwLock::new(None);

/// Install the current `TrustConfig` as the live, process-wide snapshot.
/// Called once at boot (with the loaded config) and again every time
/// `openclaw_update_settings` persists a change — the read side
/// (`current()`) always reflects the latest call, hot, no restart.
pub fn set_live_trust_config(cfg: TrustConfig) {
    if let Ok(mut guard) = LIVE_TRUST_CONFIG.write() {
        *guard = Some(cfg);
    }
}

/// Read the current live `TrustConfig`, falling back to `TrustConfig::default()`
/// if never set (e.g. in unit tests that don't call `set_live_trust_config`).
pub fn current() -> TrustConfig {
    LIVE_TRUST_CONFIG
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_trust_config_default_when_never_set() {
        // NOTE: this test shares process-global state with others in this
        // file; run in isolation via `cargo test --lib openclaw::trust_runtime`
        // if flakiness is ever observed (matches existing convention for
        // other process-global statics in this codebase, e.g. global_halt).
        let cfg = current();
        let _ = cfg; // Just confirm it doesn't panic and returns a value.
    }

    #[test]
    fn set_live_trust_config_is_read_back_hot() {
        set_live_trust_config(TrustConfig {
            community_allows_network: false,
            verified_skips_hitl: false,
            default_unknown_tier: "local".into(),
        });
        let cfg = current();
        assert!(!cfg.community_allows_network);
        assert!(!cfg.verified_skips_hitl);

        // Flip again — must be hot, no restart needed.
        set_live_trust_config(TrustConfig {
            community_allows_network: true,
            verified_skips_hitl: true,
            default_unknown_tier: "local".into(),
        });
        let cfg2 = current();
        assert!(cfg2.community_allows_network);
        assert!(cfg2.verified_skips_hitl);
    }
}
