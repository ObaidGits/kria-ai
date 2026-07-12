//! [`CapabilityKind`] and [`CapabilityFamily`] — neutral classifications of a
//! capability (spec R9.2 / R17). Inferred **non-destructively** from an existing
//! [`CapabilityDescriptor`] (provider id, tags, effects, extensions, I/O) so no
//! breaking field is added to the descriptor and older descriptors keep working.
//!
//! These are open-vocabulary at the edges (`Other`) so a new provider/kind or a
//! new capability family needs no core change — they exist for reasoning,
//! substitution, and telemetry ergonomics, never for hardcoded routing.

use serde::{Deserialize, Serialize};

use crate::capability::descriptor::CapabilityDescriptor;

/// The execution substrate a capability runs on (spec R9.2). Open via `Other`.
/// Distinct from [`CapabilityFamily`] (what it *does*).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// A KRIA-native in-process tool.
    Native,
    /// An installed OpenClaw skill (Docker substrate).
    Installed,
    /// GUI/desktop automation.
    Gui,
    /// Browser automation.
    Browser,
    /// A raw Docker capability.
    Docker,
    /// A workflow (e.g. n8n / HTN graph).
    Workflow,
    /// A cloud API capability.
    CloudApi,
    /// An MCP-server tool.
    Mcp,
    /// A remote agent / executor.
    RemoteAgent,
    /// A human-in-the-loop capability.
    Human,
    /// A KRIA-synthesized (generated) capability.
    Synthesized,
    /// Unknown / not-yet-classified (open vocabulary).
    Other(String),
}

/// What a capability *does*, for focused discovery + substitution (spec R17).
/// Open via `Other`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFamily {
    Ocr,
    Vision,
    Pdf,
    Browser,
    Filesystem,
    Translation,
    Automation,
    Coding,
    Network,
    Media,
    Data,
    Reasoning,
    /// Unknown / uncategorized (open vocabulary).
    Other(String),
}

/// Infer the [`CapabilityKind`] from a descriptor **without ever branching on a
/// hardcoded provider name** (spec R9.1/R9.2 — the Brain must be provider-neutral).
///
/// The substrate is declared by the Hands and read by the Brain, in precedence:
/// 1. explicit `extensions["kind"]` (the provider adapter declares its substrate);
/// 2. the `synthesized` marker;
/// 3. the descriptor's declared `host_requirement` (docker/browser/cloud/...);
/// 4. effect-class signals (gui/desktop/browser/network);
/// 5. open fallback `Other(provider_id)`.
///
/// A new provider therefore classifies correctly by DECLARING its kind, with no
/// kria-core change and no name matching. These are inputs to reasoning, not
/// routing.
pub fn infer_kind(d: &CapabilityDescriptor) -> CapabilityKind {
    // 1. Explicit declaration wins.
    if let Some(k) = d.extensions.get("kind").and_then(|v| v.as_str()) {
        return parse_kind(k);
    }
    // 2. Synthesized marker (generated capabilities declare this).
    if d.extensions
        .get("synthesized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return CapabilityKind::Synthesized;
    }
    // 3. Declared host requirement — neutral descriptor metadata, not a name.
    if let Some(host) = d
        .expectations
        .as_ref()
        .and_then(|e| e.host_requirement.as_deref())
    {
        let h = host.to_ascii_lowercase();
        if h.contains("docker") {
            return CapabilityKind::Docker;
        }
        if h.contains("browser") || h.contains("chrome") {
            return CapabilityKind::Browser;
        }
        if h.contains("cloud") {
            return CapabilityKind::CloudApi;
        }
        if h.contains("remote") || h.contains("ssh") {
            return CapabilityKind::RemoteAgent;
        }
    }
    // 4. Effect-class signals.
    let classes = effect_classes(d);
    if classes
        .iter()
        .any(|c| c.contains("gui") || c.contains("desktop"))
    {
        return CapabilityKind::Gui;
    }
    if classes.iter().any(|c| c.contains("browser")) {
        return CapabilityKind::Browser;
    }
    if classes
        .iter()
        .any(|c| c.contains("network") || c.contains("net"))
    {
        return CapabilityKind::CloudApi;
    }
    // 5. Open fallback — carries the provider id as data, never matched in code.
    CapabilityKind::Other(d.provider_id.clone())
}

/// Infer the [`CapabilityFamily`] from a descriptor's tags/name/description/IO
/// without mutating it. An explicit `extensions["family"]` wins.
pub fn infer_family(d: &CapabilityDescriptor) -> CapabilityFamily {
    if let Some(f) = d.extensions.get("family").and_then(|v| v.as_str()) {
        return parse_family(f);
    }
    // Build a lowercase haystack from open, provider-supplied text (tags first).
    let mut hay = String::new();
    for t in &d.tags {
        hay.push_str(&t.id.to_ascii_lowercase());
        hay.push(' ');
    }
    hay.push_str(&d.name.to_ascii_lowercase());
    hay.push(' ');
    hay.push_str(&d.description.to_ascii_lowercase());
    for io in d.inputs.iter().chain(d.outputs.iter()) {
        hay.push(' ');
        hay.push_str(&io.to_ascii_lowercase());
    }
    // Ordered checks (most specific first). These classify, they do not route.
    const RULES: &[(&str, &str)] = &[
        ("ocr", "ocr"),
        ("pdf", "pdf"),
        ("translat", "translation"),
        ("browser", "browser"),
        ("scrape", "browser"),
        ("image", "vision"),
        ("vision", "vision"),
        ("file", "filesystem"),
        ("directory", "filesystem"),
        ("zip", "filesystem"),
        ("archive", "filesystem"),
        ("http", "network"),
        ("url", "network"),
        // Data utilities before "coding" — "encode"/"decode" contain "code".
        ("json", "data"),
        ("csv", "data"),
        ("hash", "data"),
        ("base64", "data"),
        ("regex", "coding"),
        ("code", "coding"),
        ("audio", "media"),
        ("video", "media"),
        ("automat", "automation"),
    ];
    for (needle, fam) in RULES {
        if hay.contains(needle) {
            return parse_family(fam);
        }
    }
    CapabilityFamily::Other("uncategorized".into())
}

fn effect_classes(d: &CapabilityDescriptor) -> Vec<String> {
    d.effects
        .classes
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn parse_kind(s: &str) -> CapabilityKind {
    match s.to_ascii_lowercase().as_str() {
        "native" => CapabilityKind::Native,
        "installed" => CapabilityKind::Installed,
        "gui" => CapabilityKind::Gui,
        "browser" => CapabilityKind::Browser,
        "docker" => CapabilityKind::Docker,
        "workflow" => CapabilityKind::Workflow,
        "cloud_api" | "cloud" => CapabilityKind::CloudApi,
        "mcp" => CapabilityKind::Mcp,
        "remote_agent" | "remote" => CapabilityKind::RemoteAgent,
        "human" => CapabilityKind::Human,
        "synthesized" => CapabilityKind::Synthesized,
        other => CapabilityKind::Other(other.to_string()),
    }
}

fn parse_family(s: &str) -> CapabilityFamily {
    match s.to_ascii_lowercase().as_str() {
        "ocr" => CapabilityFamily::Ocr,
        "vision" => CapabilityFamily::Vision,
        "pdf" => CapabilityFamily::Pdf,
        "browser" => CapabilityFamily::Browser,
        "filesystem" => CapabilityFamily::Filesystem,
        "translation" => CapabilityFamily::Translation,
        "automation" => CapabilityFamily::Automation,
        "coding" => CapabilityFamily::Coding,
        "network" => CapabilityFamily::Network,
        "media" => CapabilityFamily::Media,
        "data" => CapabilityFamily::Data,
        "reasoning" => CapabilityFamily::Reasoning,
        other => CapabilityFamily::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{CapabilityDescriptor, CapabilityTag};

    fn desc(provider: &str, name: &str, tags: &[&str]) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            provider,
            "cap",
            name,
            "",
            serde_json::json!({"type": "object"}),
        );
        d.tags = tags.iter().map(|t| CapabilityTag::new(*t)).collect();
        d
    }

    #[test]
    fn kind_from_declared_metadata_not_provider_name() {
        // The Hands DECLARE their substrate; the Brain reads it (no name branching).
        let mut installed = desc("some-provider", "x", &[]);
        installed
            .extensions
            .insert("kind".into(), serde_json::json!("installed"));
        assert_eq!(infer_kind(&installed), CapabilityKind::Installed);

        let mut mcp = desc("any-registry", "x", &[]);
        mcp.extensions
            .insert("kind".into(), serde_json::json!("mcp"));
        assert_eq!(infer_kind(&mcp), CapabilityKind::Mcp);

        // Undeclared ⇒ open fallback carrying the provider id (never matched).
        assert_eq!(
            infer_kind(&desc("brand-new-provider", "x", &[])),
            CapabilityKind::Other("brand-new-provider".into())
        );
    }

    #[test]
    fn kind_from_declared_host_requirement() {
        use crate::capability::descriptor::Expectations;
        let mut d = desc("p", "x", &[]);
        d.expectations = Some(Expectations {
            host_requirement: Some("docker".into()),
            ..Default::default()
        });
        assert_eq!(infer_kind(&d), CapabilityKind::Docker);
    }

    #[test]
    fn kind_explicit_extension_wins() {
        let mut d = desc("openclaw", "x", &[]);
        d.extensions
            .insert("kind".into(), serde_json::json!("synthesized"));
        assert_eq!(infer_kind(&d), CapabilityKind::Synthesized);
    }

    #[test]
    fn family_from_tags() {
        assert_eq!(
            infer_family(&desc("openclaw", "OCR reader", &["media.image.ocr"])),
            CapabilityFamily::Ocr
        );
        assert_eq!(
            infer_family(&desc("openclaw", "Zip a folder", &["fs.archive"])),
            CapabilityFamily::Filesystem
        );
        assert_eq!(
            infer_family(&desc("openclaw", "Base64 encode", &["data.base64"])),
            CapabilityFamily::Data
        );
    }

    #[test]
    fn family_explicit_extension_wins() {
        let mut d = desc("openclaw", "x", &[]);
        d.extensions
            .insert("family".into(), serde_json::json!("vision"));
        assert_eq!(infer_family(&d), CapabilityFamily::Vision);
    }
}
