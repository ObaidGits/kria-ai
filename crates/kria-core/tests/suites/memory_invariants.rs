//! Architecture invariant gates (memory-upgrade Task 20 / I-1, I-2).
//!
//! Repository-level checks that fail automatically if the memory architecture's
//! boundaries are violated. These are cheap source scans over
//! `src/memory/**.rs` — no runtime needed.
//!
//! Enforced invariants:
//! * **I-1a (single authority connection):** raw `Connection::open` for the
//!   memory database only appears in `memory/db/` — every other module goes
//!   through the `Database` handle / `AuthorityTx`.
//! * **I-1b (FTS writes through stores):** the `memories_fts` index is only
//!   written from the storage layer (`stores/`) + the two authority-txn writers
//!   that own FTS mutation (`write_policy/slow.rs`, `lifecycle.rs`). No other
//!   module touches the derived index directly.
//! * **I-2 (single public façade):** `MemorySystem` exists as the composition
//!   root/public façade in `memory/api.rs`.
//!
//! Note on the original spec's stricter "only `api` is `pub`" wording: the
//! shipped architecture intentionally exposes the cognitive engines (goals,
//! plans, reasoning, causal, graph_intel, …) as `pub` modules so they are
//! directly usable + unit-testable, while `MemorySystem` remains the single
//! orchestrating façade. That supersedes the literal I-2 wording (documented in
//! tasks.md Task 20); this gate enforces the façade's existence instead.

use std::path::{Path, PathBuf};

fn memory_dir() -> PathBuf {
    // The memory subsystem was lifted out of this crate into `kria-memory`, so its
    // sources are a sibling crate away rather than under `src/`. This invariant suite
    // reads the source text directly, which is why it has to follow the move.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kria-core lives inside crates/, so it has a parent")
        .join("kria-memory/src")
}

/// Recursively collect `.rs` files under `dir`.
fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out
}

/// Path relative to `src/memory`, using forward slashes.
fn rel(p: &Path) -> String {
    let base = memory_dir();
    p.strip_prefix(&base)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Strip `//`, `///`, `//!` line comments so doc/comment mentions of a pattern
/// don't trip the source-code invariant (we only care about real code).
fn code_only(src: &str) -> String {
    src.lines()
        .map(|line| match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keep only the code that ships, dropping everything from the first `#[cfg(test)]`
/// onward.
///
/// The invariant below is about how the RUNNING system reaches the database. A unit
/// test spinning up its own throwaway database breaks nothing, so counting test code
/// as a violation reports failures that cannot be fixed — and a guard that cries wolf
/// gets switched off, which is worse than having no guard.
fn shipping_code_only(src: &str) -> String {
    let code = code_only(src);
    match code.find("#[cfg(test)]") {
        Some(idx) => code[..idx].to_string(),
        None => code,
    }
}

#[test]
fn i1a_single_authority_connection() {
    // Only `db/` may open a raw SQLite connection for the memory authority.
    //
    // Two precisions matter here, both learned from this test firing on five
    // harmless lines:
    //
    // 1. Only shipping code counts — see `shipping_code_only`.
    // 2. Only `Connection::open(` counts, NOT `open_in_memory()`. An in-memory
    //    database has no file, is discarded when the process exits, and cannot
    //    corrupt or race the real store. Treating the two as the same thing is what
    //    made this test look like it had found an architectural breach when every
    //    hit was a unit test building a scratch database.
    let offenders: Vec<String> = rs_files(&memory_dir())
        .into_iter()
        .filter(|p| {
            let r = rel(p);
            !r.starts_with("db/")
                && shipping_code_only(&std::fs::read_to_string(p).unwrap())
                    .contains("Connection::open(")
        })
        .map(|p| rel(&p))
        .collect();
    assert!(
        offenders.is_empty(),
        "I-1a violated: raw `Connection::open` outside memory/db/: {offenders:?}"
    );
}

#[test]
fn i1b_fts_writes_only_in_storage_layer() {
    // Files permitted to mutate the `memories_fts` derived index.
    const ALLOWED: &[&str] = &["stores/", "write_policy/slow.rs", "lifecycle.rs"];
    let offenders: Vec<String> = rs_files(&memory_dir())
        .into_iter()
        .filter(|p| {
            let r = rel(p);
            if ALLOWED.iter().any(|a| r.starts_with(a) || r == *a) {
                return false;
            }
            code_only(&std::fs::read_to_string(p).unwrap()).contains("memories_fts")
        })
        .map(|p| rel(&p))
        .collect();
    assert!(
        offenders.is_empty(),
        "I-1b violated: `memories_fts` written outside the storage layer: {offenders:?}"
    );
}

#[test]
fn i2_single_public_facade_exists() {
    // `api/mod.rs`, not `api.rs`: the façade has always been a directory module, so
    // this assertion was reading a path that never existed and failing for the wrong
    // reason. It went unnoticed because this suite was not in the verification gate.
    let api = std::fs::read_to_string(memory_dir().join("api/mod.rs")).unwrap();
    assert!(
        api.contains("pub struct MemorySystem"),
        "I-2: MemorySystem façade must exist in memory/api.rs"
    );
}
