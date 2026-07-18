//! Consent-gated cold-start scanners (memory-upgrade P9 / R8).
//!
//! Produces *previewable* [`ScanCandidate`]s for each [`ScanSource`], strictly
//! behind [`ColdStartConsent::gate`] (deny-by-default). Nothing here writes to
//! memory — importing is a separate, explicit step
//! ([`MemorySystem::cold_start_import`](crate::memory::api::MemorySystem::cold_start_import))
//! so the user always previews + approves before anything is ingested.
//!
//! Scanners are bounded (file counts, depth, bytes), skip noisy/secret paths,
//! and never follow symlinks — safe to run on a real home directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use walkdir::WalkDir;

use crate::memory::cold_start::{ColdStartConsent, ScanCandidate, ScanSource};
use crate::memory::db::Database;
use crate::memory::error::MemoryResult;

/// Extensions worth indexing during cold start (docs + notes + source).
const INDEXABLE_EXT: &[&str] = &[
    "md", "markdown", "txt", "rst", "org", "pdf", "rs", "py", "ts", "tsx", "js", "jsx", "go",
    "java", "kt", "c", "cpp", "h", "hpp", "rb", "php", "swift", "lua", "sh", "sql", "json", "toml",
    "yaml", "yml",
];

/// Directory names never descended into (noise / secrets / build output).
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".next",
    ".idea",
    ".vscode",
    "vendor",
    ".gradle",
    ".cargo",
];

/// Filename fragments that indicate secrets — never previewed/imported.
const SECRET_HINTS: &[&str] = &[
    ".env",
    "id_rsa",
    "id_ed25519",
    ".pem",
    ".key",
    "credentials",
    "secret",
    ".p12",
    ".pfx",
    ".keystore",
];

fn is_secretish(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECRET_HINTS.iter().any(|h| lower.contains(h))
}

/// Minimum token length considered for the high-entropy secret heuristic.
const ENTROPY_MIN_TOKEN_LEN: usize = 24;
/// Shannon entropy (bits/char) above which a long token is treated as a secret
/// (random keys sit ~4.5–6; natural-language words sit ~2.5–3.5).
const ENTROPY_SECRET_BITS: f32 = 4.0;

/// Shannon entropy (bits per character) of a token.
fn shannon_entropy(token: &str) -> f32 {
    if token.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    let mut n = 0u32;
    for b in token.bytes() {
        counts[b as usize] += 1;
        n += 1;
    }
    let nf = n as f32;
    let mut h = 0.0f32;
    for &c in counts.iter() {
        if c > 0 {
            let p = c as f32 / nf;
            h -= p * p.log2();
        }
    }
    h
}

/// A high-entropy, key-shaped token (long, base64/hex-ish, mixed case/digits)
/// that no labelled regex caught — likely an unlabelled credential.
fn has_high_entropy_secret(text: &str) -> bool {
    for token in text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '_' || c == '-' || c == '=')
    }) {
        if token.len() < ENTROPY_MIN_TOKEN_LEN {
            continue;
        }
        let has_digit = token.bytes().any(|b| b.is_ascii_digit());
        let has_alpha = token.bytes().any(|b| b.is_ascii_alphabetic());
        // Keys mix letters + digits; skip pure-alpha prose and pure-digit ids.
        if has_digit && has_alpha && shannon_entropy(token) >= ENTROPY_SECRET_BITS {
            return true;
        }
    }
    false
}

/// Content-level secret detection for cold-start import (S1). Beyond the
/// filename-substring filter, this inspects the *bytes* of a candidate file so
/// in-file secrets (a token pasted into a `.md`/`.txt`, a key in a config)
/// cannot slip in. Reuses the deterministic [`sensitivity`](crate::memory::sensitivity)
/// detectors (labelled keys/tokens/PII) and adds an unlabelled high-entropy
/// token heuristic. Fail-safe: any hit means "do not import this file".
pub fn content_has_secret(text: &str) -> bool {
    if crate::memory::sensitivity::classify(text).class == crate::memory::types::Sensitivity::Secret
    {
        return true;
    }
    has_high_entropy_secret(text)
}

fn default_root() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub struct ColdStartScanner {
    db: Arc<Database>,
}

impl ColdStartScanner {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn consent(&self) -> ColdStartConsent {
        ColdStartConsent::new(self.db.clone())
    }

    /// Gated preview for `source`. Errors (deny-by-default) unless the user has
    /// granted consent for that source. `root` overrides the scan root for the
    /// filesystem/workspace/git scanners.
    pub fn preview(
        &self,
        source: ScanSource,
        root: Option<&str>,
        limit: usize,
    ) -> MemoryResult<Vec<ScanCandidate>> {
        self.consent().gate(source)?; // hard gate — no scan without consent
        let cands = match source {
            ScanSource::Filesystem | ScanSource::Workspace => self.scan_files(root, limit),
            ScanSource::Git => self.scan_git(root, limit),
            ScanSource::Shell => self.scan_shell(limit),
        };
        Ok(cands)
    }

    fn scan_files(&self, root: Option<&str>, limit: usize) -> Vec<ScanCandidate> {
        let root = root.map(PathBuf::from).unwrap_or_else(default_root);
        let mut out = Vec::new();
        for entry in WalkDir::new(&root)
            .max_depth(6)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip noise dirs + hidden dirs (but allow the root itself).
                if e.file_type().is_dir() {
                    if let Some(name) = e.file_name().to_str() {
                        if SKIP_DIRS.contains(&name) {
                            return false;
                        }
                        if name.starts_with('.') && e.depth() > 0 {
                            return false;
                        }
                    }
                }
                true
            })
            .filter_map(|e| e.ok())
        {
            if out.len() >= limit {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_str().unwrap_or("");
            if is_secretish(name) {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !INDEXABLE_EXT.contains(&ext.as_str()) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            // Skip empties + very large files (streaming ingest is out of scope here).
            if size == 0 || size > 5_000_000 {
                continue;
            }
            let detail = format!("{ext} · {} KB", size / 1024);
            out.push(ScanCandidate {
                source: "filesystem".to_string(),
                path: path.display().to_string(),
                detail,
            });
        }
        out
    }

    fn scan_git(&self, root: Option<&str>, limit: usize) -> Vec<ScanCandidate> {
        let root = root.map(PathBuf::from).unwrap_or_else(default_root);
        let mut out = Vec::new();
        for entry in WalkDir::new(&root)
            .max_depth(4)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    if let Some(name) = e.file_name().to_str() {
                        // Descend into .git only enough to detect the repo; skip other noise.
                        if name != ".git" && SKIP_DIRS.contains(&name) {
                            return false;
                        }
                    }
                }
                true
            })
            .filter_map(|e| e.ok())
        {
            if out.len() >= limit {
                break;
            }
            if entry.file_type().is_dir() && entry.file_name() == ".git" {
                if let Some(repo) = entry.path().parent() {
                    out.extend(self.git_commits(repo, 5));
                }
            }
        }
        out.truncate(limit);
        out
    }

    fn git_commits(&self, repo: &Path, n: usize) -> Vec<ScanCandidate> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .arg("log")
            .arg(format!("-n{n}"))
            .arg("--pretty=format:%h %s")
            .output();
        let repo_name = repo
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo")
            .to_string();
        match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| ScanCandidate {
                    source: "git".to_string(),
                    path: repo.display().to_string(),
                    detail: format!("{repo_name}: {l}"),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn scan_shell(&self, limit: usize) -> Vec<ScanCandidate> {
        let home = default_root();
        let mut out = Vec::new();
        for hist in [".bash_history", ".zsh_history"] {
            let path = home.join(hist);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let lines: Vec<&str> = content
                    .lines()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty() && !is_secretish(l))
                    .collect();
                for line in lines.iter().rev().take(limit.saturating_sub(out.len())) {
                    out.push(ScanCandidate {
                        source: "shell".to_string(),
                        path: path.display().to_string(),
                        detail: (*line).to_string(),
                    });
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::cold_start::ColdStartConsent;

    #[test]
    fn content_secret_scan_catches_labelled_and_high_entropy() {
        // Labelled secrets (reused sensitivity detectors).
        assert!(content_has_secret(
            "aws key AKIAIOSFODNN7EXAMPLE in my notes"
        ));
        assert!(content_has_secret("password = hunter2please123"));
        assert!(content_has_secret("db = postgres://user:pass@localhost/db"));
        // Unlabelled high-entropy token (e.g. a pasted API key).
        assert!(content_has_secret(
            "here is the key sk9Kf83jGH72kdLPq0zXcV5bNm18RtYw"
        ));
        // Ordinary prose + long ordinary words are NOT secrets.
        assert!(!content_has_secret(
            "the deployment pipeline runs the integration tests before release"
        ));
        assert!(!content_has_secret(
            "internationalization and antidisestablishmentarianism are long words"
        ));
    }

    #[test]
    fn entropy_scan_does_not_flag_common_identifiers() {
        // AUD-04 regression: common near-uniform identifiers that appear in
        // ordinary notes must NOT be treated as secrets (avoid over-skipping).
        // Git SHA (40 hex chars, entropy ~ log2(16) = 4.0 max, real ones dip below).
        assert!(!content_has_secret(
            "see commit 356a192b7913b04c54574d18c28d46e6395428ab for the fix"
        ));
        // A UUID (hyphen-split → each segment < 24 chars → never a secret token).
        assert!(!content_has_secret(
            "run id 550e8400-e29b-41d4-a716-446655440000 completed"
        ));
        // A prose sentence with no long alphanumeric token.
        assert!(!content_has_secret(
            "the memory workspace shows health metrics and the enrichment backlog"
        ));
    }

    #[test]
    fn entropy_scan_flags_real_key_shaped_tokens() {
        // AUD-04: high-entropy, key-shaped strings are still caught without
        // embedding vendor credential prefixes that trigger push protection.
        assert!(content_has_secret(
            "opaque 16C7e42F292c6912E7710c838347Ae178B4aQ9z"
        ));
        assert!(content_has_secret(
            "opaque 51H8xQ2eZvKYlo2Cabcd1234EFGH5678ijklMNop"
        ));
    }

    #[test]
    fn preview_is_denied_without_consent() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let scanner = ColdStartScanner::new(db.clone());
        // Deny-by-default: no consent granted yet.
        let r = scanner.preview(ScanSource::Filesystem, Some("/tmp"), 5);
        assert!(r.is_err(), "scan must be gated by consent");
    }

    #[test]
    fn preview_scans_files_after_consent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "hello cold start").unwrap();
        std::fs::write(dir.path().join("secret.env"), "TOKEN=abc").unwrap();
        let db = Arc::new(Database::open_in_memory().unwrap());
        ColdStartConsent::new(db.clone())
            .grant(ScanSource::Filesystem)
            .unwrap();
        let scanner = ColdStartScanner::new(db.clone());
        let cands = scanner
            .preview(
                ScanSource::Filesystem,
                Some(dir.path().to_str().unwrap()),
                50,
            )
            .unwrap();
        assert!(cands.iter().any(|c| c.path.ends_with("notes.md")));
        // Secret files are never previewed.
        assert!(!cands.iter().any(|c| c.path.ends_with("secret.env")));
    }
}
