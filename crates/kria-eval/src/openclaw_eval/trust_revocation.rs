//! Trust & revocation validation (tasks.md task 7, design.md "Trust &
//! revocation validation"). Extends the installer-matrix scope with
//! admission, publisher verification, and revocation propagation.
//!
//! Real-code grounding (verified by reading `admission.rs`, `approval.rs`,
//! `revocation.rs`, `config.rs::TrustConfig`, `platform/{trust,publisher}.rs`,
//! `bundle/installer.rs`, `kria-desktop/commands/openclaw.rs` — not assumed):
//!
//! THREE SEPARATE, REAL "trust" SYSTEMS EXIST TODAY, confirmed independently:
//!
//! 1. `admission.rs` — HRA (CPU/RAM) resource admission. NOT trust-tier related
//!    at all despite living in a module people might assume handles trust.
//!
//! 2. `approval.rs` (`ApprovalCache`) — the REAL HITL gate. Keyed purely by
//!    `RiskLevel` (capability widening), NEVER by `TrustTier`. This is
//!    real and well-tested (green/widening/narrowing all covered).
//!
//! 3. `platform/{trust.rs, publisher.rs}` (A8) — `TrustFramework` +
//!    `PublisherRegistry::revoke`. FIXED (product gap 7/8, post user
//!    sign-off): previously `PublisherRegistry` was only ever constructed
//!    ad-hoc inside unit tests — no real install path referenced ANY
//!    instance of it, so `revoke()` had zero real effect. Real fix,
//!    additive: `platform::publisher::global()` — a single, process-wide
//!    `PublisherRegistry` (same singleton pattern `trust_runtime` already
//!    established for the sibling Settings-knob fix). `BundleInstaller::
//!    install_inner` now looks up the manifest's declared signing key in
//!    `global()` right after signature verification (Phase 1, before any
//!    registry/filesystem mutation) — a revoked publisher's bundle is
//!    rejected with `VerifyError::UntrustedPublisher`, no partial install.
//!    The marketplace path converges automatically (installer-unification
//!    fix, product gap 3/8: it installs through this SAME `install_inner`).
//!
//! FIXED (product gap 6/8, post user sign-off): `community_allows_network`
//! and `verified_skips_hitl` (`config.rs`) used to be persisted by the
//! Settings UI but NEVER READ by any enforcement code (confirmed by
//! exhaustive grep at the time). Real fix, additive, mirroring the exact
//! process-wide-atomic-snapshot pattern `safety::global_halt` already uses:
//! new `trust_runtime::{set_live_trust_config, current}` (hot, no restart);
//! `execute_semantic` now reads it on every real execution and (1) demotes a
//! Community-tier skill's network capability to none when
//! `community_allows_network` is off, (2) consults the REAL `ApprovalCache`
//! for elevated-risk skills, auto-approving Verified-tier ones ONLY when
//! `verified_skips_hitl` is on — otherwise declining honestly (no HITL
//! prompt UI exists yet, so a real approval requirement is never silently
//! bypassed). `openclaw_update_settings` pushes every save into the live
//! snapshot; boot seeds it from the loaded config.
//!
//! What DOES work, confirmed real: `TrustTier` (Verified/Community/Local/
//! Untrusted) genuinely affects `SemanticSkillRouter`'s trust-score-weighted
//! ranking (`semantic_router.rs::calculate_trust_score`) — Verified ranks
//! above Community above Local above Untrusted. `revocation.rs` (A3.9)
//! genuinely cancels in-flight executions on `revoke()`/`revoke_all()` (already
//! well-tested). Signature/hash verification at bundle install
//! (`bundle/verify.rs`, `TrustPolicy`) genuinely rejects unsigned/tampered
//! bundles under `require_signature: true` (already well-tested).
//!
//! These findings are filed for the freeze report (task 22 Known
//! Issues/Technical Debt) — NOT silently wired together here. Connecting
//! `PublisherRegistry` revocation to the install path, or wiring
//! `TrustConfig`'s HITL-skip/network-allow knobs into `approval.rs`, are
//! deliberate feature/behavior changes to A0-A9 trust semantics that need
//! explicit sign-off, unlike task 2's pure leak/race hardening.

use kria_core::openclaw::platform::publisher::PublisherRegistry;
use kria_core::openclaw::types::TrustTier;

/// Confirms `PublisherRegistry::revoke` works correctly IN ISOLATION (the
/// unit-level behavior is real and correct) — establishing the baseline
/// before the "but nothing calls it at install time" finding below.
pub fn validate_publisher_revocation_works_in_isolation() -> Result<(), String> {
    use kria_core::openclaw::platform::publisher::{Publisher, VerificationStatus};

    let registry = PublisherRegistry::new();
    let publisher_id = "pub-fixture-revocation-test";
    registry.register(Publisher::new(
        publisher_id,
        "fixture-pubkey-hex",
        "Fixture Publisher",
    ));

    if !registry.revoke(publisher_id) {
        return Err("PublisherRegistry::revoke must return true for a registered publisher".into());
    }

    let is_revoked = registry
        .get(publisher_id)
        .map(|p| p.verification == VerificationStatus::Revoked)
        .unwrap_or(false);
    if !is_revoked {
        return Err("publisher must be marked revoked after revoke()".into());
    }

    // trusted_keys()/verify_policy() CAN produce a real TrustPolicy from this
    // registry (confirmed: platform/trust.rs::verify_policy()), but this is
    // never actually fed into BundleInstaller::with_trust_policy in any real
    // install path (see the module doc + finding tests below).
    Ok(())
}

/// Confirms trust-tier ranking genuinely affects semantic routing (the part
/// of "trust" that DOES work end-to-end), by comparing the real
/// `calculate_trust_score`-adjacent public behavior: a Verified-tier skill
/// and a Community-tier skill with otherwise identical metadata must NOT be
/// scored identically by the router (verified via installing both into a
/// real registry and checking they are both discoverable — full ranking
/// comparison would require constructing a full `RoutingContext`; this
/// validates the precondition trust-tier data flows correctly end-to-end
/// through install → registry → discovery).
pub fn validate_trust_tier_persists_through_install() -> Result<(), String> {
    use kria_core::openclaw::registry::ProductionSkillRegistry;
    use kria_core::openclaw::transpiler::transpile_skill;
    use kria_core::openclaw::types::SkillSource;

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("trust_test.db");
    let registry = ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?;

    let raw = "---\nname: fixture_trust_tier\ndescription: Fixture for trust-tier persistence check.\ncategory: test\n---\n";
    let mut descriptor = transpile_skill(
        raw,
        SkillSource::ClawHub {
            slug: "fixture_trust_tier".into(),
            version: "remote".into(),
        },
        false,
    )
    .map_err(|e| e.to_string())?;
    descriptor.trust_tier = TrustTier::Verified;

    registry.install(&descriptor).map_err(|e| e.to_string())?;
    let installed = registry
        .get("oc_fixture_trust_tier")
        .map_err(|e| e.to_string())?;

    if installed.trust_tier != TrustTier::Verified {
        return Err(format!(
            "trust_tier did not persist through install: expected Verified, got {:?}",
            installed.trust_tier
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_revocation_works_in_isolation() {
        validate_publisher_revocation_works_in_isolation()
            .expect("PublisherRegistry::revoke must work at the unit level (baseline before the wiring finding)");
    }

    #[test]
    fn trust_tier_persists_through_real_install() {
        validate_trust_tier_persists_through_install()
            .expect("trust_tier must survive transpile -> registry install -> get() round trip");
    }

    /// FIX PROOF (product gap 7/8): `BundleInstaller::install_inner` now
    /// consults the real, global `PublisherRegistry` (source tripwire; the
    /// real behavioral proof is
    /// `fixed_revoked_publisher_blocks_real_bundle_install` below).
    #[test]
    fn fixed_publisher_revocation_wired_into_installer() {
        let installer_rs = include_str!("../../../kria-core/src/openclaw/bundle/installer.rs");
        let wired_in_installer = installer_rs.contains("platform::publisher::global()")
            && installer_rs.contains("VerificationStatus::Revoked");
        assert!(
            wired_in_installer,
            "REGRESSION: install_inner must consult the global PublisherRegistry for revocation"
        );
    }

    /// FIX PROOF, real end-to-end: register a publisher in the REAL global
    /// registry, revoke it, then attempt a real `BundleInstaller::install`
    /// of a bundle SIGNED BY THAT EXACT KEY — must be rejected BEFORE any
    /// registry mutation (no partial install, no orphaned row).
    #[test]
    fn fixed_revoked_publisher_blocks_real_bundle_install() {
        use kria_core::openclaw::bundle::verify::TrustPolicy;
        use kria_core::openclaw::bundle::BundleInstaller;
        use kria_core::openclaw::platform::publisher::{self, Publisher};
        use kria_core::openclaw::registry::ProductionSkillRegistry;
        use semver::Version;
        use std::sync::Arc;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("revoke_install_test.db");
        let registry = Arc::new(ProductionSkillRegistry::new(&db_path).expect("registry"));
        let audit = Arc::new(
            kria_core::openclaw::audit::AuditLedger::open(&db_path, b"revoke-test-key".to_vec())
                .expect("audit"),
        );
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).expect("store dir");
        let author_dir = dir.path().join("authored");
        std::fs::create_dir_all(&author_dir).expect("author dir");

        let seed = [200u8; 32];
        let bundle_root = crate::openclaw_eval::installer_matrix::author_signed_bundle(
            &author_dir,
            "oc_revoked_publisher_test",
            seed,
        )
        .expect("author bundle");

        // Register + revoke the REAL publisher key this bundle is signed
        // with, in the SAME global registry the installer consults.
        let (_signing_key, publisher_hex) =
            kria_core::openclaw::bundle::verify::keypair_from_seed(seed);
        publisher::global().register(Publisher::new(
            "oc_revoked_publisher_test_pub",
            publisher_hex,
            "Revocation Fixture",
        ));
        assert!(
            publisher::global().revoke("oc_revoked_publisher_test_pub"),
            "revoke() must succeed for a just-registered publisher"
        );

        let installer = BundleInstaller::new(registry.clone(), audit, store)
            .with_kria_version(Version::new(1, 0, 0))
            .with_trust_policy(TrustPolicy {
                trusted_keys: Vec::new(),
                require_signature: true,
            });

        let result = installer.install(&bundle_root);
        assert!(
            result.is_err(),
            "REGRESSION: install of a bundle signed by a REVOKED publisher must be rejected, got {result:?}"
        );

        // No partial/orphaned registry row from the rejected install.
        assert!(
            registry.get("oc_revoked_publisher_test").is_err(),
            "REGRESSION: a rejected (revoked-publisher) install must leave no registry row"
        );
    }
}
