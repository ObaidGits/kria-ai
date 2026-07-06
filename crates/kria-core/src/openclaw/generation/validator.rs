//! A9.6 Validator — reject invalid generated skills before packaging.
//!
//! Reuses the FROZEN bundle layer (`Bundle::open` validates manifest + schema +
//! capabilities + required files). Adds A9-specific checks: no TODO/placeholder code,
//! capability/risk consistency, slug uniqueness against the registry.

use super::designer::SkillDesign;
use super::generator::GeneratedArtifacts;
use crate::openclaw::bundle::Bundle;
use std::path::Path;

/// A validation problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationIssue {
    ManifestInvalid(String),
    MissingFile(String),
    PlaceholderCode(String),
    EmptyHandler,
    NoTests,
    SlugConflict(String),
    CapabilityRiskMismatch(String),
}

/// The single validator (A9.6).
pub struct SkillValidator;

impl SkillValidator {
    /// Validate a materialized bundle directory + its design/artifacts.
    ///
    /// `existing_slugs` are already-installed skill ids (from the A5 registry) used to
    /// detect slug conflicts (never regenerate an installed skill — A9.0).
    pub fn validate(
        bundle_dir: &Path,
        design: &SkillDesign,
        artifacts: &GeneratedArtifacts,
        existing_slugs: &[String],
    ) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        // 1. Frozen bundle validation (manifest, schema, entry present).
        match Bundle::open(bundle_dir) {
            Ok(_) => {}
            Err(e) => issues.push(ValidationIssue::ManifestInvalid(e.to_string())),
        }

        // 2. Handler must be real production code — no placeholders (A9.5).
        let code = &artifacts.handler_code;
        if code.trim().is_empty() {
            issues.push(ValidationIssue::EmptyHandler);
        }
        for marker in [
            "TODO",
            "FIXME",
            "XXX",
            "unimplemented",
            "placeholder",
            "PLACEHOLDER",
        ] {
            if code.contains(marker) {
                issues.push(ValidationIssue::PlaceholderCode(marker.to_string()));
            }
        }

        // 3. Tests must exist.
        if artifacts.test_code.trim().is_empty() {
            issues.push(ValidationIssue::NoTests);
        }

        // 4. Slug conflict — do not regenerate an installed skill.
        if existing_slugs.iter().any(|s| s == &design.slug) {
            issues.push(ValidationIssue::SlugConflict(design.slug.clone()));
        }

        // 5. Capability/risk consistency: declared risk must match inferred risk.
        let inferred = super::designer::classify_risk(&design.capabilities);
        if inferred != design.risk {
            issues.push(ValidationIssue::CapabilityRiskMismatch(format!(
                "declared {:?} but inferred {:?}",
                design.risk, inferred
            )));
        }

        issues
    }
}
