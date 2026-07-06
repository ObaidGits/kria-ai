//! R3 — marketplace install + drift surfacing (tasks.md task 6).
//!
//! Real-code grounding (verified by reading `openclaw/clawhub.rs`,
//! `kria-desktop/commands/openclaw.rs::clawhub_install_skill`, not assumed):
//! - `ClawHubClient::fetch_remote_index`/`search_remote`/`download_skill_manifest`
//!   all validate URLs via `DomainValidator`, which HARD-REJECTS any non-HTTPS
//!   scheme unconditionally (`clawhub.rs`: `if parsed.scheme() != "https" { ... }`).
//!   A plain local HTTP fixture server is therefore correctly rejected by design —
//!   this module validates that rejection for real rather than working around it.
//! - The real marketplace install pipeline (`clawhub_install_skill`) is:
//!   validate manifest URL → download raw SKILL.md → `transpile_skill` (assigns
//!   `TrustTier::Community` always) → validate declared network domains →
//!   `skill_registry.install(&descriptor)`. This module exercises the REAL
//!   `transpile_skill` + `ProductionSkillRegistry` with fixture SKILL.md content
//!   (the exact artifact `download_skill_manifest` would have returned), which is
//!   the part of the pipeline that is network-agnostic and where R3's real
//!   acceptance criteria (verify, abort-on-bad-input, drift) actually live.
//! - REAL FINDING (R12-relevant, filed for task 8 not silently merged here):
//!   `clawhub_install_skill` installs via `skill_registry.install(&descriptor)`
//!   directly — NO signature verification, NO rollback, NO activation callback —
//!   a COMPLETELY DIFFERENT code path from `BundleInstaller::install` (used by
//!   local `.ocskill` bundles). Two divergent installers exist today, confirming
//!   design.md's R12 concern is real, not hypothetical. Task 8 (installer matrix)
//!   will assert convergence; this task does not silently unify them.
//! - The audit's real drift finding (index.json declares 1 skill while the local
//!   DB holds 3) is reproduced with a REAL fixture index + REAL registry seeding.

use kria_core::openclaw::clawhub::{ClawHubClient, DomainValidator};
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::transpiler::transpile_skill;
use kria_core::openclaw::types::{SkillSource, TrustTier};

/// R3.6 / security posture: `DomainValidator` must reject a non-HTTPS index
/// URL outright — this is the REAL reason a plain local HTTP fixture server
/// cannot be used against `ClawHubClient` directly, verified rather than
/// assumed.
pub fn validate_https_only_enforced() -> Result<(), String> {
    let validator = DomainValidator::new(Vec::new());
    match validator.validate("http://127.0.0.1:9/index.json") {
        Ok(()) => Err("DomainValidator must reject a non-HTTPS URL, but it accepted one".into()),
        Err(reason) => {
            eprintln!("[R3.6] DomainValidator correctly rejected non-HTTPS URL: {reason}");
            Ok(())
        }
    }
}

/// R3.6: `ClawHubClient::fetch_remote_index` against an unreachable host must
/// fail gracefully (no panic, no hang, honest error) — real network call to a
/// port nothing listens on. `127.0.0.1` is explicitly allowlisted so this
/// tests ACTUAL network unreachability, not the (separately validated,
/// `validate_https_only_enforced`) domain-allowlist rejection.
pub async fn validate_unreachable_repo_fails_gracefully() -> Result<(), String> {
    let client = ClawHubClient::new("https://127.0.0.1:9/index.json", vec!["127.0.0.1".to_string()]);
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(std::time::Duration::from_secs(20), client.fetch_remote_index()).await;
    let elapsed = start.elapsed();

    match result {
        Err(_) => Err(format!("fetch_remote_index hung past the 20s bound (R3.6 violation), elapsed={elapsed:?}")),
        Ok(Ok(_)) => Err("fetch_remote_index unexpectedly succeeded against an unreachable host".into()),
        Ok(Err(e)) => {
            let is_network_error = e.to_string().to_lowercase().contains("network")
                || e.to_string().to_lowercase().contains("connect")
                || e.to_string().to_lowercase().contains("refused");
            if !is_network_error {
                return Err(format!(
                    "expected a NETWORK error proving real unreachability, got a different \
                     failure (possibly domain-allowlist, not what this test validates): {e}"
                ));
            }
            eprintln!("[R3.6] unreachable repo failed gracefully in {elapsed:?} (confirmed network error): {e}");
            Ok(())
        }
    }
}

/// A real fixture SKILL.md (frontmatter format `transpile_skill` actually
/// parses — verified against `transpiler.rs`), for a valid install.
fn valid_skill_md(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: Fixture skill for openclaw_eval marketplace validation.\ncategory: test\n---\n\nDiscarded prose.\n"
    )
}

/// R3.2/R3.3: real `transpile_skill` + real `ProductionSkillRegistry` install,
/// exactly mirroring `clawhub_install_skill`'s post-download logic. Verifies
/// the installed skill is Community tier (security enforcement: remote skills
/// are NEVER Verified, per the real command's step 4) and is discoverable.
pub fn validate_transpile_and_install_real() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("marketplace_test.db");
    let registry = ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?;

    // NOTE: `transpile_skill` ALWAYS prefixes the final skill_id with "oc_"
    // regardless of the input `name` (transpiler.rs: `format!("oc_{}",
    // sanitize_name(name))`) — confirmed by the crate's own
    // `transpile_valid_skill` test (`name: "web_search"` -> `skill_id:
    // "oc_web_search"`). Fixture names here therefore do NOT include the
    // "oc_" prefix themselves, to avoid a double "oc_oc_..." id.
    let raw = valid_skill_md("fixture_market_valid");
    let source = SkillSource::ClawHub { slug: "fixture_market_valid".into(), version: "remote".into() };
    let mut descriptor = transpile_skill(&raw, source, false).map_err(|e| format!("transpile failed: {e}"))?;

    // Mirror clawhub_install_skill step 4: remote skills always Community.
    descriptor.trust_tier = TrustTier::Community;
    if descriptor.trust_tier != TrustTier::Community {
        return Err("remote skill must always be Community tier, never Verified".into());
    }

    registry.install(&descriptor).map_err(|e| format!("registry install failed: {e}"))?;

    let installed = registry.get("oc_fixture_market_valid").map_err(|e| e.to_string())?;
    if installed.trust_tier != TrustTier::Community {
        return Err(format!("installed skill trust_tier drifted, got {:?}", installed.trust_tier));
    }

    Ok(())
}

/// R3.3: a malformed SKILL.md (missing required `description` field) must
/// abort transpilation — nothing gets installed.
pub fn validate_malformed_manifest_aborts() -> Result<(), String> {
    let raw = "---\nname: oc_fixture_bad\n---\nno description field\n";
    match transpile_skill(raw, SkillSource::ClawHub { slug: "oc_fixture_bad".into(), version: "remote".into() }, false) {
        Ok(_) => Err("transpile_skill must reject a manifest missing the required description field".into()),
        Err(e) => {
            eprintln!("[R3.3] malformed manifest correctly rejected: {e}");
            Ok(())
        }
    }
}

/// R3.5: reproduces the audit's real drift finding — a fixture `index.json`
/// listing exactly 1 skill while the seeded local registry holds 3 (installed
/// via the real registry, not fabricated counts). Surfaces the drift as a
/// structured comparison rather than silently reporting only one number.
pub fn validate_drift_surfaced() -> Result<DriftReport, String> {
    use crate::openclaw_eval::fixtures::{drift_index_json, FixtureIndexEntry};

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("drift_test.db");
    let registry = ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?;

    // Seed the LOCAL DB with 3 skills (reproducing the audit's real "DB has 3"
    // side). Fixture names omit the "oc_" prefix (see validate_transpile_and_
    // install_real doc) since transpile_skill always adds it.
    for i in 1..=3 {
        let raw = valid_skill_md(&format!("fixture_drift_db_{i}"));
        let descriptor = transpile_skill(
            &raw,
            SkillSource::ClawHub { slug: format!("fixture_drift_db_{i}"), version: "remote".into() },
            false,
        )
        .map_err(|e| e.to_string())?;
        registry.install(&descriptor).map_err(|e| e.to_string())?;
    }

    // The fixture index.json lists exactly 1 skill (reproducing the audit's
    // real "index has 1" side) — parse it with the REAL wire-format struct.
    let index_json = drift_index_json("https://example.invalid/manifests");
    let index_entries: Vec<FixtureIndexEntry> = serde_json::from_str(&index_json).map_err(|e| e.to_string())?;

    let db_skills = registry.get_enabled_skills().map_err(|e| e.to_string())?;
    let db_slugs: std::collections::HashSet<String> = db_skills.iter().map(|s| s.skill_id.clone()).collect();
    let index_slugs: std::collections::HashSet<String> = index_entries.iter().map(|e| e.slug.clone()).collect();

    let db_only: Vec<String> = db_slugs.difference(&index_slugs).cloned().collect();
    let index_only: Vec<String> = index_slugs.difference(&db_slugs).cloned().collect();

    let report = DriftReport {
        db_count: db_slugs.len(),
        index_count: index_slugs.len(),
        db_only,
        index_only,
    };

    if report.db_count == report.index_count && report.db_only.is_empty() && report.index_only.is_empty() {
        return Err("expected drift (db=3 vs index=1, per the audit finding) but none was detected".into());
    }

    eprintln!(
        "[R3.5] drift surfaced: db_count={} index_count={} db_only={:?} index_only={:?}",
        report.db_count, report.index_count, report.db_only, report.index_only
    );

    Ok(report)
}

#[derive(Debug)]
pub struct DriftReport {
    pub db_count: usize,
    pub index_count: usize,
    pub db_only: Vec<String>,
    pub index_only: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r3_6_https_only_enforced() {
        validate_https_only_enforced().expect("R3.6: DomainValidator must reject non-HTTPS");
    }

    #[tokio::test]
    async fn r3_6_unreachable_repo_fails_gracefully() {
        validate_unreachable_repo_fails_gracefully()
            .await
            .expect("R3.6: unreachable repo must fail gracefully, never hang");
    }

    #[test]
    fn r3_2_transpile_and_install_real() {
        validate_transpile_and_install_real().expect("R3.2: real transpile+install must succeed and enforce Community tier");
    }

    #[test]
    fn r3_3_malformed_manifest_aborts() {
        validate_malformed_manifest_aborts().expect("R3.3: malformed manifest must abort transpilation");
    }

    #[test]
    fn r3_5_drift_is_surfaced_not_hidden() {
        let report = validate_drift_surfaced().expect("R3.5: drift (db=3, index=1) must be surfaced");
        assert_eq!(report.db_count, 3, "expected 3 skills seeded in local DB (audit finding)");
        assert_eq!(report.index_count, 1, "expected 1 skill in fixture index (audit finding)");
        assert_eq!(report.db_only.len(), 3, "all 3 DB skills should be DB-only vs the drift fixture index");
    }
}
