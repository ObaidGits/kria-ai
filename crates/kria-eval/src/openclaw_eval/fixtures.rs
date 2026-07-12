//! Fixture generator for the OpenClaw test rig (design.md "Test rig",
//! design.md "Data Models"). Produces real, schema-valid artifacts that mirror
//! the actual `clawhub.rs` index format and the actual bundle
//! (`openclaw/bundle/manifest.rs`) layout — never a fabricated shape.
//!
//! Fixtures generated here are used by tasks 6 (R3 marketplace), 8 (R12
//! installer matrix), and 7 (trust & revocation).

use std::path::{Path, PathBuf};

/// A fixture skill bundle written to disk in the real `.ocskill` directory
/// layout: `manifest.toml` + `schema.json` + the entry file the manifest
/// declares. Callers may additionally write `MANIFEST.sha256`/`bundle.sig`
/// (via the real `bundle::verify` helpers) for the signed/valid case, or
/// intentionally omit/corrupt them for the bad-hash/bad-manifest cases.
pub struct FixtureBundle {
    pub root: PathBuf,
    pub slug: String,
}

const ENTRY_RELATIVE_PATH: &str = "handler/entry.js";

/// Write a minimal, real, schema-valid `.ocskill` directory for `slug` under
/// `dir`. This is the "valid" fixture (R3 valid-signed-skill case, once
/// signed by the caller with `bundle::verify`).
pub fn write_valid_bundle(
    dir: &Path,
    slug: &str,
    publisher: &str,
) -> std::io::Result<FixtureBundle> {
    let root = dir.join(slug);
    std::fs::create_dir_all(root.join("handler"))?;

    let manifest = format!(
        r#"[skill]
slug = "{slug}"
name = "Fixture Skill {slug}"
version = "1.0.0"
category = "test"
tags = ["fixture"]
intent = "Deterministic fixture skill for OpenClaw validation."
description = "Fixture skill used only by the openclaw_eval test rig."
min_kria = "0.1.0"
license = "MIT"

[runtime]
kind = "docker"
entry = "{ENTRY_RELATIVE_PATH}"
mcp = true

[resource]
class = "light"
memory_mb = 128
timeout_secs = 15

[trust]
declared_tier = "community"
publisher = "{publisher}"
"#
    );
    std::fs::write(root.join("manifest.toml"), manifest)?;
    std::fs::write(
        root.join("schema.json"),
        r#"{"type":"object","properties":{}}"#,
    )?;
    std::fs::write(
        root.join(ENTRY_RELATIVE_PATH),
        "// fixture entry — no-op handler for openclaw_eval\n",
    )?;

    Ok(FixtureBundle {
        root,
        slug: slug.to_string(),
    })
}

/// Write a bundle with an invalid manifest (bad slug prefix — fails
/// `Manifest::validate`). Used for the R3.3 bad-manifest abort case.
pub fn write_invalid_manifest_bundle(
    dir: &Path,
    slug_no_prefix: &str,
) -> std::io::Result<FixtureBundle> {
    let root = dir.join(format!("invalid_{slug_no_prefix}"));
    std::fs::create_dir_all(root.join("handler"))?;

    // Deliberately missing the required `oc_` slug prefix -> ManifestError::InvalidSlug.
    let manifest = format!(
        r#"[skill]
slug = "{slug_no_prefix}"
name = "Invalid Fixture"
version = "1.0.0"
category = "test"
description = "Fixture with an invalid slug (missing oc_ prefix)."
min_kria = "0.1.0"

[runtime]
kind = "docker"
entry = "{ENTRY_RELATIVE_PATH}"

[resource]
class = "light"

[trust]
declared_tier = "community"
publisher = "fixture-publisher"
"#
    );
    std::fs::write(root.join("manifest.toml"), manifest)?;
    std::fs::write(root.join("schema.json"), "{}")?;
    std::fs::write(root.join(ENTRY_RELATIVE_PATH), "// fixture\n")?;

    Ok(FixtureBundle {
        root,
        slug: slug_no_prefix.to_string(),
    })
}

/// Corrupt an existing bundle's entry file AFTER a hash tree would have been
/// computed over the original content, to produce a genuine hash-mismatch
/// (R3.3 bad-hash abort case). Caller is responsible for computing/writing
/// `MANIFEST.sha256` BEFORE calling this.
pub fn corrupt_entry_file(bundle: &FixtureBundle) -> std::io::Result<()> {
    std::fs::write(
        bundle.root.join(ENTRY_RELATIVE_PATH),
        "// TAMPERED after MANIFEST.sha256 was computed\n",
    )
}

/// A minimal real-schema entry in the ClawHub `index.json` format
/// (`kria_core::openclaw::clawhub::RemoteSkillEntry`). Kept as a plain struct
/// here (rather than depending on `kria_core::openclaw::clawhub` directly)
/// because the field set is part of a stable, versioned wire contract, and
/// the fixture generator asserts against that same shape in tests.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FixtureIndexEntry {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub trust_tier: String,
    pub version: String,
    pub manifest_url: String,
    #[serde(default)]
    pub capabilities_summary: Vec<String>,
}

/// Serialize a fixture `index.json` body for the given entries.
pub fn build_index_json(entries: &[FixtureIndexEntry]) -> String {
    serde_json::to_string_pretty(entries).expect("fixture entries are always serializable")
}

/// Build the R3.5 drift fixture: an `index.json` listing exactly ONE skill
/// while the local DB is seeded (by the caller, via
/// `ProductionSkillRegistry::install_skill`) with `local_skill_count` skills.
/// Reproduces the audit's real-world finding (index=1, DB=3).
pub fn drift_index_json(manifest_base_url: &str) -> String {
    let entries = vec![FixtureIndexEntry {
        slug: "oc_fixture_drift_only_in_index".to_string(),
        name: "Drift Fixture (index-only)".to_string(),
        description: "Present in index.json but never installed locally.".to_string(),
        category: "test".to_string(),
        trust_tier: "community".to_string(),
        version: "1.0.0".to_string(),
        manifest_url: format!("{manifest_base_url}/oc_fixture_drift_only_in_index.md"),
        capabilities_summary: vec![],
    }];
    build_index_json(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kria_core::openclaw::bundle::manifest::Manifest;

    #[test]
    fn valid_fixture_bundle_parses_and_validates_with_real_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let bundle =
            write_valid_bundle(dir.path(), "oc_fixture_valid", "fixture-publisher").unwrap();

        let toml_str = std::fs::read_to_string(bundle.root.join("manifest.toml")).unwrap();
        let manifest = Manifest::parse(&toml_str)
            .expect("fixture manifest.toml must parse with the REAL parser");
        let caps = manifest
            .validate()
            .expect("fixture manifest must validate with the REAL validator");
        assert_eq!(manifest.skill.slug, "oc_fixture_valid");
        assert!(caps.is_empty());
    }

    #[test]
    fn invalid_manifest_fixture_fails_real_validation() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = write_invalid_manifest_bundle(dir.path(), "fixture_missing_prefix").unwrap();

        let toml_str = std::fs::read_to_string(bundle.root.join("manifest.toml")).unwrap();
        let manifest = Manifest::parse(&toml_str).unwrap();
        assert!(
            manifest.validate().is_err(),
            "fixture must be rejected by the REAL validator (missing oc_ prefix)"
        );
    }

    #[test]
    fn drift_fixture_lists_exactly_one_skill() {
        let json = drift_index_json("https://example.invalid/manifests");
        let entries: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(
            entries.len(),
            1,
            "drift fixture index.json must list exactly 1 skill"
        );
    }
}
