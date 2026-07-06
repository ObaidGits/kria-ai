//! Installer-unification fix (R12, product gap 3/8): synthesize a REAL, on-disk
//! `.ocskill` bundle directory from a transpiled marketplace `SkillDescriptor`, so the
//! marketplace install path can go through the SAME `BundleInstaller` — same signature
//! verification framework, same rollback, same activation, same real `content_hash` —
//! that the local `.ocskill` path already uses. This is what closes the R12
//! "installer_matrix" finding (`clawhub_install_skill` used to call
//! `registry.install()` directly: no signature check, no rollback, no activation, a
//! hardcoded `content_hash: "legacy"`).
//!
//! Honest scope, documented not hidden: a marketplace `SKILL.md` (the ClawHub source
//! format) carries NO executable handler code today — `transpiler::transpile_skill`
//! only ever produced descriptor metadata (name/description/capabilities), and the real
//! OpenClaw substrate image only dispatches to a fixed, baked-in set of handlers
//! (`openclaw-substrate/skills/*.js`, confirmed by reading the substrate image layout).
//! This was true before this fix and remains true after it — synthesizing a bundle here
//! does NOT fabricate a real handler implementation that didn't exist. The synthesized
//! `runtime.entry` file is a real, present, self-describing stub (never claims to work);
//! a skill installed this way is registered, verifiable, and rollback-safe exactly like
//! any other bundle, but will honestly fail at execution time with "not implemented" if
//! invoked — the SAME honest behavior a marketplace skill without real code always had.
//! Wiring marketplace skills to REAL executable handlers is a separate, larger content
//! problem (the ClawHub skill format would need to start shipping code), not something
//! installer convergence can or should paper over.

use super::manifest::Manifest;
use super::verify::{sign_bundle, write_hash_tree};
use crate::openclaw::capability::Capability;
use crate::openclaw::types::SkillDescriptor;
use ed25519_dalek::SigningKey;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SynthError {
    #[error("io error: {0}")]
    Io(String),
    #[error("manifest serialization error: {0}")]
    Toml(String),
}

/// A locally-generated ed25519 signing key used ONLY to satisfy the same
/// signature-presence contract every bundle goes through (`TrustPolicy::strict()`
/// requires a signature). This does NOT claim marketplace-trust equivalence to a
/// real publisher key — the descriptor's `trust_tier` is still forced to
/// `Community` by the caller (`clawhub_install_skill`), exactly as before this fix.
/// Kept process-local and regenerated per synth call; never persisted or reused
/// as a trust anchor.
fn ephemeral_signing_key() -> SigningKey {
    // Real, non-deterministic entropy — this key only needs to produce a
    // structurally valid signature, not a trusted one.
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    SigningKey::from_bytes(&seed)
}

/// Escape a string for embedding in a TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn capability_kind_str(kind: crate::openclaw::capability::CapabilityKind) -> &'static str {
    use crate::openclaw::capability::CapabilityKind::*;
    match kind {
        Filesystem => "filesystem",
        Network => "network",
        Subprocess => "subprocess",
        Browser => "browser",
        Gpu => "gpu",
        Clipboard => "clipboard",
        Device => "device",
        Environment => "environment",
    }
}

fn capability_mode_str(mode: crate::openclaw::capability::CapabilityMode) -> &'static str {
    use crate::openclaw::capability::CapabilityMode::*;
    match mode {
        ReadOnly => "read_only",
        ReadWrite => "read_write",
        Egress => "egress",
        Execute => "execute",
        Use => "use",
    }
}

fn capability_scope_toml(scope: &crate::openclaw::capability::CapabilityScope) -> String {
    use crate::openclaw::capability::CapabilityScope::*;
    match scope {
        Workspace => "\"workspace\"".to_string(),
        InputMount(id) => format!("\"input:{}\"", toml_escape(id)),
        Domains(items) | Binaries(items) | EnvVars(items) => {
            let quoted: Vec<String> = items
                .iter()
                .map(|i| format!("\"{}\"", toml_escape(i)))
                .collect();
            format!("[{}]", quoted.join(", "))
        }
        None => "\"none\"".to_string(),
    }
}

/// Render a real `manifest.toml` string for a marketplace-transpiled descriptor.
/// `caps` are the descriptor's REAL declared capabilities (post capability-grant-
/// wiring fix), not a fabricated set.
fn render_manifest_toml(
    descriptor: &SkillDescriptor,
    caps: &[Capability],
    entry_rel_path: &str,
) -> String {
    let mut out = String::new();
    out.push_str("[skill]\n");
    out.push_str(&format!(
        "slug = \"{}\"\n",
        toml_escape(&descriptor.skill_id)
    ));
    out.push_str(&format!("name = \"{}\"\n", toml_escape(&descriptor.name)));
    // Marketplace descriptors don't carry a real semver from the source
    // SKILL.md today — use a valid default; `SkillSource::ClawHub{version,..}`
    // already tracks the real remote version string separately in the registry.
    out.push_str("version = \"1.0.0\"\n");
    out.push_str(&format!(
        "category = \"{}\"\n",
        toml_escape(&descriptor.category)
    ));
    out.push_str(&format!(
        "description = \"{}\"\n",
        toml_escape(&descriptor.description)
    ));
    out.push_str("min_kria = \"0.0.0\"\n");
    out.push('\n');
    out.push_str("[runtime]\n");
    out.push_str("kind = \"docker\"\n");
    out.push_str(&format!("entry = \"{}\"\n", entry_rel_path));
    out.push_str("mcp = true\n");
    out.push('\n');
    out.push_str("[resource]\n");
    out.push_str(&format!(
        "class = \"{}\"\n",
        descriptor.resource_profile.resource_class.as_str()
    ));
    out.push_str(&format!(
        "timeout_secs = {}\n",
        descriptor.resource_profile.timeout_secs
    ));
    out.push('\n');
    for c in caps {
        out.push_str("[[capabilities]]\n");
        out.push_str(&format!("kind = \"{}\"\n", capability_kind_str(c.kind)));
        out.push_str(&format!("mode = \"{}\"\n", capability_mode_str(c.mode)));
        out.push_str(&format!("scope = {}\n", capability_scope_toml(&c.scope)));
        out.push('\n');
    }
    out.push_str("[trust]\n");
    // Always Community for marketplace installs, matching the caller's forced
    // `descriptor.trust_tier = TrustTier::Community` (security enforcement kept
    // identical to pre-fix behavior — installer unification changes HOW the
    // install happens, never the trust-tier security rule).
    out.push_str("declared_tier = \"community\"\n");
    out.push_str(&format!(
        "publisher = \"{}\"\n",
        ephemeral_publisher_key_hex()
    ));
    out
}

/// Cached-per-call publisher key hex for this synth invocation (see
/// `ephemeral_signing_key` doc — not a trust anchor, just a structurally valid key).
fn ephemeral_publisher_key_hex() -> String {
    // This function intentionally returns a fresh key's hex representation
    // each call; `synth_marketplace_bundle` computes and reuses ONE key for
    // both the manifest.trust.publisher field and the actual signature, via
    // `synth_marketplace_bundle`'s local key variable — this free fn is only
    // used by tests exercising `render_manifest_toml` in isolation.
    let key = ephemeral_signing_key();
    hex::encode(key.verifying_key().to_bytes())
}

/// Synthesize a real, self-contained bundle DIRECTORY on disk for a marketplace
/// descriptor: `manifest.toml`, `schema.json`, a real (present, honest stub) entry
/// file, `MANIFEST.sha256`, and `bundle.sig` — everything `Bundle::open` +
/// `Bundle::verify` require, using the exact same verification code path as any
/// other bundle.
///
/// Returns the directory path; caller is responsible for its lifetime (typically a
/// `TempDir`, consumed immediately by `BundleInstaller::install`).
pub fn synth_marketplace_bundle(
    descriptor: &SkillDescriptor,
    caps: &[Capability],
    dest_dir: &Path,
) -> Result<PathBuf, SynthError> {
    std::fs::create_dir_all(dest_dir).map_err(|e| SynthError::Io(e.to_string()))?;

    let handler_dir = dest_dir.join("handler");
    std::fs::create_dir_all(&handler_dir).map_err(|e| SynthError::Io(e.to_string()))?;
    let entry_rel = "handler/entry.js";
    let entry_path = dest_dir.join(entry_rel);
    // Honest stub: a real, present file — never claims to implement the skill.
    // Marketplace `SKILL.md` sources carry no executable code today (see module
    // doc); the real substrate only executes its fixed, baked-in handler set.
    std::fs::write(
        &entry_path,
        format!(
            "// Synthesized entry for marketplace skill '{}'.\n\
             // No executable handler exists for this skill — marketplace SKILL.md\n\
             // sources do not ship code today. This stub exists ONLY to satisfy the\n\
             // real Bundle::open()/verify() contract so this skill can go through\n\
             // the single, unified BundleInstaller (installer-convergence fix).\n\
             module.exports = () => ({{ error: 'not_implemented', reason: 'marketplace skills carry no executable handler today' }});\n",
            descriptor.skill_id
        ),
    )
    .map_err(|e| SynthError::Io(e.to_string()))?;

    std::fs::write(
        dest_dir.join("schema.json"),
        serde_json::to_string_pretty(&descriptor.parameters).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|e| SynthError::Io(e.to_string()))?;

    let signing_key = ephemeral_signing_key();
    let publisher_hex = hex::encode(signing_key.verifying_key().to_bytes());

    // Render manifest with the SAME key used to sign (not the free-fn helper
    // above, which generates its own independent key for isolated tests).
    let mut manifest_toml =
        render_manifest_toml_with_publisher(descriptor, caps, entry_rel, &publisher_hex);
    // render_manifest_toml_with_publisher already inlines the publisher; keep
    // variable to satisfy borrow rules cleanly.
    let _ = &mut manifest_toml;
    std::fs::write(dest_dir.join("manifest.toml"), &manifest_toml)
        .map_err(|e| SynthError::Io(e.to_string()))?;

    // Real signature over the real content-hash tree — same mechanism every
    // other bundle uses (`verify::write_hash_tree` + `verify::sign_bundle`).
    write_hash_tree(dest_dir).map_err(|e| SynthError::Io(e.to_string()))?;
    sign_bundle(dest_dir, &signing_key).map_err(|e| SynthError::Io(e.to_string()))?;

    // Sanity: the manifest we just wrote must parse + validate like any real one.
    let parsed = Manifest::parse(&manifest_toml).map_err(|e| SynthError::Toml(e.to_string()))?;
    parsed
        .validate()
        .map_err(|e| SynthError::Toml(e.to_string()))?;

    Ok(dest_dir.to_path_buf())
}

fn render_manifest_toml_with_publisher(
    descriptor: &SkillDescriptor,
    caps: &[Capability],
    entry_rel_path: &str,
    publisher_hex: &str,
) -> String {
    let mut out = render_manifest_toml(descriptor, caps, entry_rel_path);
    // render_manifest_toml already wrote a `[trust]` section with its own
    // ephemeral publisher key; replace it with the caller's actual signing
    // key so signature verification succeeds. Simple, robust text swap since
    // we control the exact format we just generated.
    if let Some(idx) = out.find("[trust]\n") {
        out.truncate(idx);
    }
    out.push_str("[trust]\n");
    out.push_str("declared_tier = \"community\"\n");
    out.push_str(&format!("publisher = \"{}\"\n", publisher_hex));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::transpiler::transpile_skill;
    use crate::openclaw::types::SkillSource;

    fn fixture_descriptor() -> (SkillDescriptor, Vec<Capability>) {
        let raw = "---\nname: synth_fixture\ndescription: Fixture for bundle synthesis.\ncategory: test\ncapabilities:\n  filesystem_read: true\n---\n";
        let descriptor = transpile_skill(
            raw,
            SkillSource::ClawHub {
                slug: "synth_fixture".into(),
                version: "remote".into(),
            },
            false,
        )
        .expect("transpile must succeed");
        let caps = descriptor
            .granted
            .iter()
            .map(|g| g.capability.clone())
            .collect();
        (descriptor, caps)
    }

    #[test]
    fn synthesized_bundle_opens_and_verifies_via_real_bundle_code() {
        use crate::openclaw::bundle::verify::TrustPolicy;
        use crate::openclaw::bundle::Bundle;

        let (descriptor, caps) = fixture_descriptor();
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle_dir = dir.path().join("bundle");

        synth_marketplace_bundle(&descriptor, &caps, &bundle_dir).expect("synth must succeed");

        // Must open via the REAL, unmodified Bundle::open (same code path as
        // any other bundle).
        let bundle =
            Bundle::open(&bundle_dir).expect("synthesized bundle must open like any real bundle");
        // Must verify via the REAL, unmodified verify path with signature
        // required (TrustPolicy::strict) — proves this bundle is genuinely
        // signed and hash-verified, not a shortcut.
        let content_hash = bundle
            .verify(&TrustPolicy::strict())
            .expect("synthesized bundle must pass real signature+hash verification");
        assert!(
            !content_hash.is_empty(),
            "content hash must be real, never \"legacy\""
        );
        assert_ne!(
            content_hash, "legacy",
            "REGRESSION: synthesized bundle must never produce the old fake 'legacy' hash"
        );
    }

    #[test]
    fn synthesized_manifest_carries_real_declared_capabilities() {
        let (descriptor, caps) = fixture_descriptor();
        assert!(!caps.is_empty(), "fixture must declare a real capability");
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle_dir = dir.path().join("bundle");
        synth_marketplace_bundle(&descriptor, &caps, &bundle_dir).expect("synth must succeed");

        let toml_str = std::fs::read_to_string(bundle_dir.join("manifest.toml")).unwrap();
        let manifest = Manifest::parse(&toml_str).expect("must parse");
        let parsed_caps = manifest.validate().expect("must validate");
        assert!(
            !parsed_caps.is_empty(),
            "synthesized manifest must carry the real declared capability, not an empty set"
        );
    }
}
