//! P0.5 — Brain/Hands CI neutrality gate (spec R5.1 / R9.1 / R23).
//!
//! The Brain must never reference a provider-native type. `crate::openclaw` and
//! `mcp::client` are forbidden anywhere under `src/capability/` EXCEPT the
//! provider adapters in `src/capability/acl/`. This runs as a normal `cargo test`
//! so the invariant is enforced in CI, not just by manual grep.
//!
//! This gate file is itself excluded from the scan (it necessarily names the
//! forbidden tokens as scan patterns).

use std::fs;
use std::path::Path;

fn scan(dir: &Path, violations: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // The anti-corruption boundary is allowed to name providers.
            if path.file_name().and_then(|n| n.to_str()) == Some("acl") {
                continue;
            }
            scan(&path, violations);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // Exclude this gate file itself (names the tokens as patterns).
        if path.file_name().and_then(|n| n.to_str()) == Some("neutrality.rs") {
            continue;
        }
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        for (lineno, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("///") {
                continue;
            }
            // (1) Provider-native module/type references.
            let native_ref = line.contains("crate::openclaw")
                || line.contains("mcp::client")
                || line.contains("crate::mcp::");
            // (2) Hardcoded provider-name literals used for BRANCHING. The Brain
            //     treats provider ids as open-vocabulary data — it must never
            //     compare against or match a specific provider name. We flag a
            //     provider-name string literal only when it appears with a
            //     comparison/branch operator on the same line (avoids false
            //     positives on legitimate open-string handling).
            // Unambiguous provider-ID literals (NOT kind/family value strings
            // like "mcp"): a concrete provider name, or the "mcp:<name>" id
            // prefix. These must never be compared/matched in the Brain.
            let has_provider_literal = line.contains("\"openclaw\"") || line.contains("\"mcp:");
            let is_branch = line.contains("==")
                || line.contains(".contains(")
                || line.contains("starts_with")
                || line.contains("=> ");
            let provider_branch = has_provider_literal && is_branch;

            if native_ref || provider_branch {
                violations.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }
}

#[test]
fn brain_hands_neutrality_gate() {
    let cap_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/capability");
    let mut violations = Vec::new();
    scan(&cap_dir, &mut violations);
    assert!(
        violations.is_empty(),
        "Brain/Hands neutrality violated — provider-native refs outside acl/:\n{}",
        violations.join("\n")
    );
}
