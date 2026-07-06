//! A9.3 Skill Designer + A9.4 Capability Inference.
//!
//! Turns a `SkillRequirement` into a `SkillDesign` (name/slug/manifest fields/schema/
//! capabilities/deps/risk/examples/docs) that satisfies A0 contracts. Capability
//! inference is automatic — no manual capability declaration.

use super::requirements::SkillRequirement;
use crate::safety::RiskLevel;
use serde::{Deserialize, Serialize};

/// A complete skill design (A9.3). Everything needed to emit an `.ocskill` bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDesign {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub version: String,
    /// Inferred capabilities (A9.4) as manifest capability strings.
    pub capabilities: Vec<String>,
    /// External dependencies.
    pub dependencies: Vec<String>,
    /// Assessed risk (derived from capabilities — never trust author claim).
    pub risk: RiskLevel,
    /// JSON schema for the skill parameters.
    pub schema: serde_json::Value,
    /// Example invocations.
    pub examples: Vec<SkillExample>,
    /// Generated documentation (markdown).
    pub documentation: String,
    /// Runtime kind (docker for A9).
    pub runtime_kind: String,
    /// Entry file (relative), e.g. "handler/main.js".
    pub entry: String,
    /// Resource class hint: light / medium / heavy.
    pub resource_class: String,
}

/// An example invocation of the skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillExample {
    pub description: String,
    pub params: serde_json::Value,
}

/// High-risk capabilities that require human approval before install (A9.0.3).
pub const HIGH_RISK_CAPABILITIES: &[&str] = &[
    "filesystem_write",
    "filesystem_delete",
    "shell",
    "subprocess",
    "browser",
    "database_write",
    "environment_secrets",
    "gpu",
    "system_settings",
    "registry_modify",
    "network_egress",
    "user_credentials",
];

/// Infer capabilities from a requirement (A9.4). Deterministic keyword + field analysis
/// over the extracted requirement, unioned with any explicitly implied capabilities.
pub fn infer_capabilities(req: &SkillRequirement) -> Vec<String> {
    let mut caps: Vec<String> = req.implied_capabilities.clone();

    let haystack = format!(
        "{} {} {}",
        req.intent.to_lowercase(),
        req.constraints.join(" ").to_lowercase(),
        req.tags.join(" ").to_lowercase()
    );

    let add = |caps: &mut Vec<String>, c: &str| {
        if !caps.iter().any(|x| x == c) {
            caps.push(c.to_string());
        }
    };

    // Filesystem.
    if haystack.contains("read")
        || haystack.contains("load")
        || haystack.contains("open")
        || haystack.contains("scan")
        || haystack.contains("exif")
        || haystack.contains("csv")
        || haystack.contains("pdf")
        || haystack.contains("zip")
        || haystack.contains("image")
    {
        add(&mut caps, "filesystem_read");
    }
    if haystack.contains("write")
        || haystack.contains("save")
        || haystack.contains("rename")
        || haystack.contains("merge")
        || haystack.contains("convert")
        || haystack.contains("resize")
        || haystack.contains("extract")
        || haystack.contains("output")
    {
        add(&mut caps, "filesystem_write");
    }
    if haystack.contains("delete") || haystack.contains("remove") || haystack.contains("cleanup") {
        add(&mut caps, "filesystem_delete");
    }
    // Network.
    if haystack.contains("download")
        || haystack.contains("fetch")
        || haystack.contains("http")
        || haystack.contains("url")
        || haystack.contains("web")
        || haystack.contains("api")
    {
        add(&mut caps, "network_egress");
    }
    // Subprocess / shell.
    if haystack.contains("run command")
        || haystack.contains("shell")
        || haystack.contains("exec")
        || haystack.contains("ffmpeg")
        || haystack.contains("imagemagick")
    {
        add(&mut caps, "subprocess");
    }
    // GPU.
    if haystack.contains("gpu") || haystack.contains("cuda") || haystack.contains("inference") {
        add(&mut caps, "gpu");
    }

    caps
}

/// Classify risk from inferred capabilities (A9.4). RED if any high-risk write/exec cap,
/// YELLOW for network/browser, GREEN otherwise.
pub fn classify_risk(capabilities: &[String]) -> RiskLevel {
    let has = |c: &str| capabilities.iter().any(|x| x == c);
    if has("subprocess")
        || has("shell")
        || has("filesystem_delete")
        || has("system_settings")
        || has("registry_modify")
        || has("user_credentials")
    {
        RiskLevel::Red
    } else if has("filesystem_write")
        || has("network_egress")
        || has("browser")
        || has("database_write")
        || has("gpu")
    {
        RiskLevel::Yellow
    } else {
        RiskLevel::Green
    }
}

/// Which capabilities in a design require human approval before install (A9.0.3).
pub fn capabilities_requiring_approval(capabilities: &[String]) -> Vec<String> {
    capabilities
        .iter()
        .filter(|c| HIGH_RISK_CAPABILITIES.contains(&c.as_str()))
        .cloned()
        .collect()
}
