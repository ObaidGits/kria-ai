//! A9.5 Production Code Generator (bundle materializer).
//!
//! Writes a design + generated artifacts into a canonical `.ocskill` bundle directory:
//! `manifest.toml`, `schema.json`, the handler entry, `README.md`, `tests/`. The result
//! is a normal bundle consumed by the FROZEN A2 pipeline (verify → sign → install).
//! No parallel packaging system (A9.9/A9.15).

use super::designer::SkillDesign;
use super::generator::GeneratedArtifacts;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("io error: {0}")]
    Io(String),
    #[error("invalid design: {0}")]
    InvalidDesign(String),
}

/// Map a design's capability strings to manifest `[[capabilities]]` array-of-tables
/// entries (kind/mode/scope) — the format the frozen manifest contract expects.
fn capabilities_toml(caps: &[String]) -> String {
    let mut out = String::new();
    let mut emit = |kind: &str, mode: &str, scope: &str| {
        out.push_str("[[capabilities]]\n");
        out.push_str(&format!("kind = \"{kind}\"\n"));
        out.push_str(&format!("mode = \"{mode}\"\n"));
        if !scope.is_empty() {
            out.push_str(&format!("scope = {scope}\n"));
        }
        out.push('\n');
    };

    let mut fs_write = false;
    let mut fs_read = false;
    for c in caps {
        match c.as_str() {
            "filesystem_write" | "filesystem_delete" => fs_write = true,
            "filesystem_read" => fs_read = true,
            "network_egress" => emit("network", "egress", "[\"*\"]"),
            "subprocess" | "shell" => emit("subprocess", "execute", "[\"*\"]"),
            "browser" => emit("browser", "use", ""),
            "gpu" => emit("gpu", "use", ""),
            "environment_secrets" => emit("environment", "use", ""),
            _ => {}
        }
    }
    // Collapse filesystem into a single capability at the highest needed mode.
    if fs_write {
        emit("filesystem", "read_write", "\"workspace\"");
    } else if fs_read {
        emit("filesystem", "read_only", "\"workspace\"");
    }

    out
}

/// Materialize the bundle directory. `publisher_hex` is the ed25519 public key (hex) of
/// the generating publisher (identity for signing, A9.9). Returns the bundle root.
pub fn emit_bundle(
    design: &SkillDesign,
    artifacts: &GeneratedArtifacts,
    dest_root: &Path,
    publisher_hex: &str,
) -> Result<PathBuf, CodegenError> {
    if design.slug.is_empty() {
        return Err(CodegenError::InvalidDesign("empty slug".into()));
    }

    let bundle_dir = dest_root.join(&design.slug);
    let entry_rel = if design.entry.is_empty() {
        "handler/main.js"
    } else {
        &design.entry
    };
    let entry_path = bundle_dir.join(entry_rel);
    if let Some(parent) = entry_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CodegenError::Io(e.to_string()))?;
    }

    // 1. Handler entry (production code — no placeholders).
    std::fs::write(&entry_path, &artifacts.handler_code)
        .map_err(|e| CodegenError::Io(e.to_string()))?;

    // 2. schema.json.
    let schema = serde_json::to_string_pretty(&design.schema)
        .map_err(|e| CodegenError::Io(e.to_string()))?;
    std::fs::write(bundle_dir.join("schema.json"), schema)
        .map_err(|e| CodegenError::Io(e.to_string()))?;

    // 3. manifest.toml (satisfies A0/manifest contract).
    let tags_toml = design
        .tags
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "'")))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = format!(
        r#"[skill]
slug = "{slug}"
name = "{name}"
version = "{version}"
category = "{category}"
description = "{description}"
min_kria = "0.1.0"
tags = [{tags}]

[runtime]
kind = "{runtime}"
entry = "{entry}"

[resource]
class = "{resource_class}"

[trust]
declared_tier = "community"
publisher = "{publisher}"

{capabilities}"#,
        slug = design.slug,
        name = design.name.replace('"', "'"),
        version = design.version,
        category = design.category,
        description = design.description.replace('"', "'"),
        tags = tags_toml,
        runtime = design.runtime_kind,
        entry = entry_rel,
        resource_class = design.resource_class,
        publisher = publisher_hex,
        capabilities = capabilities_toml(&design.capabilities),
    );
    std::fs::write(bundle_dir.join("manifest.toml"), manifest)
        .map_err(|e| CodegenError::Io(e.to_string()))?;

    // 4. README.md (documentation + examples).
    let mut readme = format!(
        "# {}\n\n{}\n\n## Examples\n\n",
        design.name, design.documentation
    );
    for ex in &design.examples {
        readme.push_str(&format!(
            "- {}\n\n```json\n{}\n```\n\n",
            ex.description,
            serde_json::to_string_pretty(&ex.params).unwrap_or_default()
        ));
    }
    readme.push_str(&artifacts.examples_doc);
    std::fs::write(bundle_dir.join("README.md"), readme)
        .map_err(|e| CodegenError::Io(e.to_string()))?;

    // 5. tests/ (sandbox tests).
    std::fs::create_dir_all(bundle_dir.join("tests"))
        .map_err(|e| CodegenError::Io(e.to_string()))?;
    std::fs::write(bundle_dir.join("tests/skill_test.js"), &artifacts.test_code)
        .map_err(|e| CodegenError::Io(e.to_string()))?;

    Ok(bundle_dir)
}
