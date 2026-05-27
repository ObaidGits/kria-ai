//! Eval harness — manages a per-test temporary working directory.
//!
//! Each `EvalHarness` owns a `tempfile::TempDir` that is cleaned up on drop.
//! Callers write source files into it, run commands inside it, and then let it
//! drop at test teardown.

use std::path::{Path, PathBuf};

/// Temporary directory owner for one integration eval case.
///
/// The inner `TempDir` is deleted when this struct is dropped.
/// Panics on construction if the OS cannot allocate a temp dir.
pub struct EvalHarness {
    dir: tempfile::TempDir,
}

impl EvalHarness {
    /// Create a new harness backed by a fresh temp dir.
    ///
    /// Tries candidate base directories in order so we work even when `/tmp`
    /// has restrictive permissions (e.g. 0755 instead of 1777).
    pub fn new() -> std::io::Result<Self> {
        let mut candidates: Vec<std::path::PathBuf> = vec![std::env::temp_dir()];

        // User cache dir as first fallback (always writable by owner)
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(
                std::path::PathBuf::from(&home)
                    .join(".cache")
                    .join("kria-eval"),
            );
        }
        candidates.push(std::path::PathBuf::from("/var/tmp"));

        for base in &candidates {
            let _ = std::fs::create_dir_all(base);
            if let Ok(dir) = tempfile::Builder::new()
                .prefix("kria-eval-")
                .tempdir_in(base)
            {
                return Ok(Self { dir });
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "no writable temp dir found (tried /tmp, $HOME/.cache/kria-eval, /var/tmp)",
        ))
    }

    /// The root path of the temp dir.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Build an absolute path for a file inside the harness.
    pub fn file_path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Synchronously write a file into the harness directory.
    pub fn write_sync(&self, name: &str, content: &str) -> std::io::Result<PathBuf> {
        let path = self.file_path(name);
        std::fs::write(&path, content)?;
        Ok(path)
    }
}
