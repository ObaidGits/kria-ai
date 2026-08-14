//! Proves every OS-control runtime seam is composed in the live aggregate.
//!
//! # Why this is a source-level check
//!
//! `live.rs` can only be compiled with `os-control-live`, and that feature is
//! mutually exclusive with the `os-control-test` composition this suite runs under
//! (a hard `compile_error!`). So the live aggregate cannot be *instantiated* here.
//! What can be checked — and what actually matters — is that no seam was left
//! returning `None`.
//!
//! # The failure this prevents
//!
//! Adding a domain takes four edits: the port, the runtime seam, the handler, and
//! the live composition. Miss the last one and everything still compiles, every
//! test passes, the tool appears in the registry, and the user gets
//! "not available" forever with nothing explaining why. That exact mistake
//! happened twice during this build — `desktop_state.rs` was compiled but never
//! registered, and nine domains had seams with no composition. Hence this test.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Collapse whitespace so a multi-line signature and a trailing comma in
/// `(&self,)` cannot hide a seam from the scan.
fn normalized(path: &PathBuf) -> String {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Every `fn <name>(&self) -> Option<&dyn …>` seam declared in a file.
fn seams(path: &PathBuf) -> BTreeSet<String> {
    let text = normalized(path);
    let mut found = BTreeSet::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("fn") {
        rest = &rest[start + 2..];
        let Some(open) = rest.find('(') else { break };
        let name = &rest[..open];
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            let after = &rest[open..];
            if after.starts_with("(&self)->Option<&dyn")
                || after.starts_with("(&self,)->Option<&dyn")
            {
                found.insert(name.to_string());
            }
        }
    }
    found
}

fn os_control_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/os_control")
}

#[test]
fn every_runtime_seam_is_composed_in_the_live_aggregate() {
    let dir = os_control_dir();
    let declared = seams(&dir.join("runtime.rs"));
    let composed = seams(&dir.join("live.rs"));

    assert!(
        declared.len() >= 29,
        "expected at least 29 runtime seams, found {}: the scan is probably broken rather than the \
         code",
        declared.len()
    );

    let missing: Vec<&String> = declared.difference(&composed).collect();
    assert!(
        missing.is_empty(),
        "these OS-control domains have a runtime seam but are NOT composed in live.rs, so every \
         one of their tools answers `Unavailable` on a real machine: {missing:?}"
    );
}

#[test]
fn no_seam_in_the_live_aggregate_is_hard_coded_to_none() {
    // A seam composed as a literal `None` compiles and passes the test above while
    // still being permanently unavailable — the exact hole that check would miss.
    let text = std::fs::read_to_string(os_control_dir().join("live.rs")).expect("read live.rs");
    let offenders: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            // A field initializer of the form `name: None,` in compose_with.
            line.ends_with(": None,")
                && !line.starts_with("//")
                // `snapshot: None` is legitimate: it means "not probed", and the
                // aggregate carries it as an Option by design.
                && !line.starts_with("snapshot:")
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "these live.rs fields are hard-coded to None, so their domain can never work: {offenders:?}"
    );
}
