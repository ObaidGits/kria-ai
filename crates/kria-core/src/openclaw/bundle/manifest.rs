//! `manifest.toml` — the single source of truth for a skill bundle (skill-package-contract §2).
//!
//! Parsed and validated at install time. The `SkillDescriptor` is a *derived projection* of this
//! manifest (package-contract §6) — never an independent record.

use crate::openclaw::capability::{Capability, CapabilityKind, CapabilityMode, CapabilityScope};
use serde::{Deserialize, Serialize};

/// The full manifest, mirroring `manifest.toml` sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub skill: SkillMeta,
    pub runtime: RuntimeMeta,
    pub resource: ResourceMeta,
    #[serde(default)]
    pub capabilities: Vec<ManifestCapability>,
    pub trust: TrustMeta,
    #[serde(default)]
    pub compat: CompatMeta,
    #[serde(default)]
    pub dependencies: DependenciesMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub intent: String,
    pub description: String,
    #[serde(default = "default_min_kria")]
    pub min_kria: String,
    #[serde(default)]
    pub license: String,
}

fn default_min_kria() -> String {
    "0.0.0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMeta {
    /// docker | wasm | firecracker | remote | cloud | gpu
    pub kind: String,
    pub entry: String,
    #[serde(default = "default_true")]
    pub mcp: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMeta {
    #[serde(default = "default_class")]
    pub class: String,
    #[serde(default)]
    pub cpu_millis: u32,
    #[serde(default)]
    pub memory_mb: u32,
    #[serde(default)]
    pub gpu: bool,
    #[serde(default)]
    pub storage_mb: u32,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,
}

fn default_class() -> String {
    "light".to_string()
}
fn default_timeout() -> u64 {
    30
}
fn default_max_output() -> usize {
    1024 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestCapability {
    pub kind: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Scope: "workspace", "input:<id>", a list of domains/binaries, or absent.
    #[serde(default)]
    pub scope: Option<toml::Value>,
}

fn default_mode() -> String {
    "read_only".to_string()
}

impl ManifestCapability {
    /// Convert to the frozen `Capability` object (capability-contract).
    pub fn to_capability(&self) -> Result<Capability, ManifestError> {
        let kind: CapabilityKind = self
            .kind
            .parse()
            .map_err(|e: String| ManifestError::InvalidCapability(e))?;
        let mode: CapabilityMode = self
            .mode
            .parse()
            .map_err(|e: String| ManifestError::InvalidCapability(e))?;
        let scope = self.parse_scope(kind)?;
        Ok(Capability { kind, mode, scope })
    }

    fn parse_scope(&self, kind: CapabilityKind) -> Result<CapabilityScope, ManifestError> {
        match &self.scope {
            None => Ok(match kind {
                CapabilityKind::Filesystem => CapabilityScope::Workspace,
                _ => CapabilityScope::None,
            }),
            Some(toml::Value::String(s)) => {
                if s == "workspace" {
                    Ok(CapabilityScope::Workspace)
                } else if let Some(id) = s.strip_prefix("input:") {
                    Ok(CapabilityScope::InputMount(id.to_string()))
                } else if s == "none" {
                    Ok(CapabilityScope::None)
                } else {
                    // A bare string scope for network/subprocess = single-item list.
                    Ok(scope_list(kind, vec![s.clone()]))
                }
            }
            Some(toml::Value::Array(arr)) => {
                let items: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                Ok(scope_list(kind, items))
            }
            Some(other) => Err(ManifestError::InvalidCapability(format!(
                "invalid scope value: {other}"
            ))),
        }
    }
}

fn scope_list(kind: CapabilityKind, items: Vec<String>) -> CapabilityScope {
    match kind {
        CapabilityKind::Subprocess => CapabilityScope::Binaries(items),
        CapabilityKind::Environment => CapabilityScope::EnvVars(items),
        _ => CapabilityScope::Domains(items),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustMeta {
    #[serde(default = "default_tier")]
    pub declared_tier: String,
    /// Publisher identity (stable key). Immutable within a slug.
    pub publisher: String,
}

fn default_tier() -> String {
    "community".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompatMeta {
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub deprecates: Vec<String>,
    #[serde(default)]
    pub rollback_to: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependenciesMeta {
    /// Skill dependencies: slug → semver requirement (e.g. "^1.2").
    #[serde(default)]
    pub skills: std::collections::BTreeMap<String, String>,
    /// Runtime binaries/tools the handler needs present in the substrate image.
    #[serde(default)]
    pub runtime: Vec<String>,
}

/// Errors from manifest parsing/validation.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("TOML parse error: {0}")]
    Parse(String),
    #[error("invalid slug '{0}' (expected oc_<alnum/underscore>)")]
    InvalidSlug(String),
    #[error("invalid version '{0}': {1}")]
    InvalidVersion(String, String),
    #[error("invalid min_kria '{0}': {1}")]
    InvalidMinKria(String, String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("unknown runtime kind: {0}")]
    UnknownRuntime(String),
    #[error("invalid capability: {0}")]
    InvalidCapability(String),
    #[error("description too long (max 200 chars)")]
    DescriptionTooLong,
}

impl Manifest {
    /// Parse a `manifest.toml` string.
    pub fn parse(toml_str: &str) -> Result<Self, ManifestError> {
        toml::from_str(toml_str).map_err(|e| ManifestError::Parse(e.to_string()))
    }

    /// Validate all required fields + formats (A2.2). Returns the parsed capabilities on success.
    pub fn validate(&self) -> Result<Vec<Capability>, ManifestError> {
        // slug: oc_ prefix + alnum/underscore, ≤ 64.
        let s = &self.skill.slug;
        if s.is_empty()
            || s.len() > 64
            || !s.starts_with("oc_")
            || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(ManifestError::InvalidSlug(s.clone()));
        }

        // version: semver.
        semver::Version::parse(&self.skill.version).map_err(|e| {
            ManifestError::InvalidVersion(self.skill.version.clone(), e.to_string())
        })?;

        // min_kria: semver.
        semver::Version::parse(&self.skill.min_kria).map_err(|e| {
            ManifestError::InvalidMinKria(self.skill.min_kria.clone(), e.to_string())
        })?;

        if self.skill.name.trim().is_empty() {
            return Err(ManifestError::MissingField("skill.name"));
        }
        if self.skill.description.trim().is_empty() {
            return Err(ManifestError::MissingField("skill.description"));
        }
        if self.skill.description.len() > 200 {
            return Err(ManifestError::DescriptionTooLong);
        }
        if self.trust.publisher.trim().is_empty() {
            return Err(ManifestError::MissingField("trust.publisher"));
        }

        // runtime kind must be recognised.
        match self.runtime.kind.to_ascii_lowercase().as_str() {
            "docker" | "wasm" | "firecracker" | "remote" | "cloud" | "gpu" => {}
            other => return Err(ManifestError::UnknownRuntime(other.to_string())),
        }
        if self.runtime.entry.trim().is_empty() {
            return Err(ManifestError::MissingField("runtime.entry"));
        }

        // capabilities parse into the frozen object.
        let mut caps = Vec::with_capacity(self.capabilities.len());
        for mc in &self.capabilities {
            caps.push(mc.to_capability()?);
        }
        Ok(caps)
    }

    pub fn semver(&self) -> semver::Version {
        semver::Version::parse(&self.skill.version)
            .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }

    /// Skill id used across KRIA (`oc_` prefixed). Identical to the slug in the bundle model.
    pub fn skill_id(&self) -> &str {
        &self.skill.slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[skill]
slug = "oc_calculator"
name = "Calculator"
version = "1.0.0"
category = "productivity"
tags = ["math"]
intent = "Evaluate arithmetic expressions."
description = "Evaluates an arithmetic expression and returns the result."
min_kria = "0.1.0"
license = "MIT"

[runtime]
kind = "docker"
entry = "handler/calculator.js"
mcp = true

[resource]
class = "light"
memory_mb = 256
timeout_secs = 30

[trust]
declared_tier = "verified"
publisher = "did:key:zTEST"

[compat]
rollback_to = "0.9.0"
"#;

    #[test]
    fn parses_and_validates() {
        let m = Manifest::parse(VALID).unwrap();
        let caps = m.validate().unwrap();
        assert_eq!(m.skill.slug, "oc_calculator");
        assert_eq!(m.semver(), semver::Version::new(1, 0, 0));
        assert!(caps.is_empty());
    }

    #[test]
    fn rejects_bad_slug() {
        let bad = VALID.replace(r#"slug = "oc_calculator""#, r#"slug = "calculator""#);
        let m = Manifest::parse(&bad).unwrap();
        assert!(matches!(m.validate(), Err(ManifestError::InvalidSlug(_))));
    }

    #[test]
    fn rejects_bad_version() {
        let bad = VALID.replace(r#"version = "1.0.0""#, r#"version = "not-semver""#);
        let m = Manifest::parse(&bad).unwrap();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::InvalidVersion(_, _))
        ));
    }

    #[test]
    fn rejects_unknown_runtime() {
        let bad = VALID.replace(r#"kind = "docker""#, r#"kind = "quantum""#);
        let m = Manifest::parse(&bad).unwrap();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::UnknownRuntime(_))
        ));
    }

    #[test]
    fn parses_capabilities() {
        let with_caps = format!(
            "{VALID}\n[[capabilities]]\nkind = \"network\"\nmode = \"egress\"\nscope = [\"api.example.com\"]\n"
        );
        let m = Manifest::parse(&with_caps).unwrap();
        let caps = m.validate().unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].kind, CapabilityKind::Network);
    }
}
