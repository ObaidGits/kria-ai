//! Phase 6 — Workspace Operational Memory.
//!
//! # Core Mission
//!
//! Persist cross-app operational context to PSDG so KRIA understands:
//! - What branch the user is on
//! - What tickets are open
//! - What build failures are active
//! - What deployments are running
//! - What debugging session is active
//!
//! This is the "operational coworker memory" — the context a team member
//! would naturally carry in their head while working.
//!
//! # Architecture
//!
//! `WorkspaceMemory` writes to `WorldModelStore` via `PsdgHandle`.
//! All writes are fire-and-forget. All reads are bounded.
//!
//! Subject prefixes:
//! - `workspace.*` — project-level facts (branch, path, language)
//! - `git.*` — version control facts (branch, last commit, remote)
//! - `build.*` — build status (last result, errors, artifacts)
//! - `test.*` — test results (last run, failures, coverage)
//! - `deploy.*` — deployment facts (target, status, last deploy)
//! - `debug.*` — debugging session (target, breakpoints, active error)
//! - `browser_context.*` — browser-level context (active ticket, PR, docs)
//! - `ide_context.*` — IDE-level context (open files, diagnostics, last edit)

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::psdg::PsdgHandle;
use crate::agent::world_model::FactSource;

/// Maximum number of build errors to persist (bounded).
const MAX_BUILD_ERRORS: usize = 5;
/// Maximum number of open tickets to persist.
#[allow(dead_code)]
const MAX_OPEN_TICKETS: usize = 10;
/// Maximum number of unresolved diagnostics to persist.
#[allow(dead_code)]
const MAX_DIAGNOSTICS: usize = 10;

// ─── Workspace Facts ──────────────────────────────────────────────────────────

/// The active workspace/project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Absolute path to the project root.
    pub root: PathBuf,
    /// Project name (derived from directory name or Cargo.toml/package.json).
    pub name: String,
    /// Primary programming language.
    pub language: Option<String>,
    /// Build system (cargo, npm, gradle, make, etc.).
    pub build_system: Option<String>,
}

/// Git version control facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitContext {
    /// Current branch name.
    pub branch: String,
    /// Short hash of the last commit.
    pub last_commit_hash: Option<String>,
    /// Last commit message (first line).
    pub last_commit_msg: Option<String>,
    /// Remote name (usually "origin").
    pub remote: Option<String>,
    /// Whether there are uncommitted changes.
    pub has_uncommitted: bool,
}

/// Build status facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStatus {
    /// Whether the last build succeeded.
    pub succeeded: bool,
    /// Key build errors (bounded to MAX_BUILD_ERRORS).
    pub errors: Vec<String>,
    /// Build output location.
    pub artifact_path: Option<PathBuf>,
    /// Epoch seconds of the last build.
    pub last_build_at: u64,
}

/// Test results facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    /// Number of passing tests.
    pub passed: u32,
    /// Number of failing tests.
    pub failed: u32,
    /// Names of failing tests (bounded to 5).
    pub failing_tests: Vec<String>,
    /// Epoch seconds of the last test run.
    pub last_run_at: u64,
}

/// Deployment status facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployStatus {
    /// Deployment target (e.g., "staging", "production").
    pub target: String,
    /// Whether the last deployment succeeded.
    pub succeeded: bool,
    /// Deployed version or commit hash.
    pub deployed_version: Option<String>,
    /// Epoch seconds of the last deployment.
    pub last_deploy_at: u64,
    /// Active deployment pipeline URL (if any).
    pub pipeline_url: Option<String>,
}

/// Active debugging session context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSession {
    /// The binary/target being debugged.
    pub target_binary: String,
    /// Active error message being investigated.
    pub active_error: Option<String>,
    /// File and line of the active breakpoint.
    pub breakpoint_location: Option<String>,
    /// Whether the debugger is currently attached.
    pub debugger_attached: bool,
}

/// Open development tickets (Jira/GitHub).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketContext {
    /// Ticket ID (e.g., "KR-123" or "github/issues/42").
    pub ticket_id: String,
    /// Ticket title/summary.
    pub title: String,
    /// Current status.
    pub status: String,
    /// URL (if available).
    pub url: Option<String>,
}

// ─── WorkspaceMemory ──────────────────────────────────────────────────────────

/// Workspace operational memory backed by PSDG WorldModelStore.
///
/// Provides a typed API over the PSDG fact graph for workspace-level context.
/// All writes are fire-and-forget. All reads are bounded.
pub struct WorkspaceMemory {
    psdg: PsdgHandle,
}

impl WorkspaceMemory {
    /// Create a new workspace memory backed by a PSDG handle.
    pub fn new(psdg: PsdgHandle) -> Self {
        Self { psdg }
    }

    // ── Write operations ───────────────────────────────────────────────────

    /// Record the active workspace/project.
    pub fn record_workspace(&self, info: &WorkspaceInfo) {
        let store = self.psdg.store_arc();
        let root = info.root.to_string_lossy().to_string();
        let name = info.name.clone();
        let lang = info.language.clone().unwrap_or_else(|| "unknown".into());
        let build = info
            .build_system
            .clone()
            .unwrap_or_else(|| "unknown".into());

        tokio::task::spawn_blocking(move || {
            let _ = store.upsert(
                "workspace",
                "root",
                &root,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "workspace",
                "name",
                &name,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "workspace",
                "language",
                &lang,
                0.85,
                FactSource::Inferred,
                "workspace_memory",
            );
            let _ = store.upsert(
                "workspace",
                "build_system",
                &build,
                0.85,
                FactSource::Inferred,
                "workspace_memory",
            );
            debug!(target: "workspace_memory", "Workspace recorded: {}", name);
        });
    }

    /// Record the current git context.
    pub fn record_git(&self, ctx: &GitContext) {
        let store = self.psdg.store_arc();
        let branch = ctx.branch.clone();
        let has_uncommitted = ctx.has_uncommitted.to_string();
        let last_msg = ctx.last_commit_msg.clone().unwrap_or_default();
        let last_hash = ctx.last_commit_hash.clone().unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            let _ = store.upsert(
                "git",
                "branch",
                &branch,
                0.97,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "git",
                "has_uncommitted_changes",
                &has_uncommitted,
                0.9,
                FactSource::Detected,
                "workspace_memory",
            );
            if !last_hash.is_empty() {
                let _ = store.upsert(
                    "git",
                    "last_commit_hash",
                    &last_hash,
                    0.95,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            if !last_msg.is_empty() {
                let _ = store.upsert(
                    "git",
                    "last_commit_message",
                    &last_msg,
                    0.90,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            debug!(target: "workspace_memory", "Git context recorded: branch={}", branch);
        });
    }

    /// Record build status.
    pub fn record_build(&self, status: &BuildStatus) {
        let store = self.psdg.store_arc();
        let succeeded = status.succeeded.to_string();
        let errors: Vec<String> = status
            .errors
            .iter()
            .take(MAX_BUILD_ERRORS)
            .cloned()
            .collect();
        let errors_json = serde_json::to_string(&errors).unwrap_or_default();
        let last_at = status.last_build_at.to_string();
        let artifact = status
            .artifact_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            let confidence = if succeeded == "true" { 0.95 } else { 0.9 };
            let _ = store.upsert(
                "build",
                "last_succeeded",
                &succeeded,
                confidence,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "build",
                "last_at",
                &last_at,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            if !errors_json.is_empty() && errors_json != "[]" {
                let _ = store.upsert(
                    "build",
                    "errors_json",
                    &errors_json,
                    0.9,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            if !artifact.is_empty() {
                let _ = store.upsert(
                    "build",
                    "artifact_path",
                    &artifact,
                    0.9,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            debug!(target: "workspace_memory", "Build status recorded: success={}", succeeded);
        });
    }

    /// Record test results.
    pub fn record_test_results(&self, results: &TestResults) {
        let store = self.psdg.store_arc();
        let passed = results.passed.to_string();
        let failed = results.failed.to_string();
        let failing: Vec<String> = results.failing_tests.iter().take(5).cloned().collect();
        let failing_json = serde_json::to_string(&failing).unwrap_or_default();
        let last_at = results.last_run_at.to_string();

        tokio::task::spawn_blocking(move || {
            let _ = store.upsert(
                "test",
                "passed_count",
                &passed,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "test",
                "failed_count",
                &failed,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "test",
                "last_run_at",
                &last_at,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            if !failing_json.is_empty() && failing_json != "[]" {
                let _ = store.upsert(
                    "test",
                    "failing_tests_json",
                    &failing_json,
                    0.9,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            debug!(target: "workspace_memory", "Test results recorded: {} passed, {} failed", passed, failed);
        });
    }

    /// Record deployment status.
    pub fn record_deploy(&self, status: &DeployStatus) {
        let store = self.psdg.store_arc();
        let target = status.target.clone();
        let succeeded = status.succeeded.to_string();
        let version = status.deployed_version.clone().unwrap_or_default();
        let last_at = status.last_deploy_at.to_string();
        let url = status.pipeline_url.clone().unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            let subject = format!("deploy_{}", target);
            let conf = if succeeded == "true" { 0.95 } else { 0.85 };
            let _ = store.upsert(
                &subject,
                "succeeded",
                &succeeded,
                conf,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                &subject,
                "last_at",
                &last_at,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            if !version.is_empty() {
                let _ = store.upsert(
                    &subject,
                    "version",
                    &version,
                    0.9,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            if !url.is_empty() {
                let _ = store.upsert(
                    &subject,
                    "pipeline_url",
                    &url,
                    0.85,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            debug!(target: "workspace_memory", "Deploy status recorded: target={}, success={}", target, succeeded);
        });
    }

    /// Record an active debugging session.
    pub fn record_debug_session(&self, session: &DebugSession) {
        let store = self.psdg.store_arc();
        let binary = session.target_binary.clone();
        let error = session.active_error.clone().unwrap_or_default();
        let bp = session.breakpoint_location.clone().unwrap_or_default();
        let attached = session.debugger_attached.to_string();

        tokio::task::spawn_blocking(move || {
            let _ = store.upsert(
                "debug",
                "target_binary",
                &binary,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                "debug",
                "debugger_attached",
                &attached,
                0.95,
                FactSource::Detected,
                "workspace_memory",
            );
            if !error.is_empty() {
                let _ = store.upsert(
                    "debug",
                    "active_error",
                    &error,
                    0.9,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            if !bp.is_empty() {
                let _ = store.upsert(
                    "debug",
                    "breakpoint",
                    &bp,
                    0.85,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            debug!(target: "workspace_memory", "Debug session recorded: target={}", binary);
        });
    }

    /// Record an open ticket.
    pub fn record_ticket(&self, ticket: &TicketContext) {
        let store = self.psdg.store_arc();
        let subject = format!(
            "ticket_{}",
            ticket.ticket_id.replace('/', "_").replace('-', "_")
        );
        let title = ticket.title.clone();
        let status = ticket.status.clone();
        let url = ticket.url.clone().unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            let _ = store.upsert(
                &subject,
                "title",
                &title,
                0.9,
                FactSource::Detected,
                "workspace_memory",
            );
            let _ = store.upsert(
                &subject,
                "status",
                &status,
                0.9,
                FactSource::Detected,
                "workspace_memory",
            );
            if !url.is_empty() {
                let _ = store.upsert(
                    &subject,
                    "url",
                    &url,
                    0.85,
                    FactSource::Detected,
                    "workspace_memory",
                );
            }
            debug!(target: "workspace_memory", "Ticket recorded: {}", subject);
        });
    }

    // ── Read operations ────────────────────────────────────────────────────

    /// Get the current workspace root and name.
    pub fn get_workspace(&self) -> Option<WorkspaceInfo> {
        let store = self.psdg.store();
        let root = store.query("workspace", "root").ok()??;
        let name = store.query("workspace", "name").ok()??.object;
        let language = store.query("workspace", "language").ok()?.map(|f| f.object);
        let build_system = store
            .query("workspace", "build_system")
            .ok()?
            .map(|f| f.object);

        Some(WorkspaceInfo {
            root: PathBuf::from(&root.object),
            name,
            language,
            build_system,
        })
    }

    /// Get the current git branch.
    pub fn get_branch(&self) -> Option<String> {
        self.psdg
            .store()
            .query("git", "branch")
            .ok()?
            .filter(|f| f.confidence >= 0.7)
            .map(|f| f.object)
    }

    /// Get the last build status.
    pub fn get_build_status(&self) -> Option<bool> {
        self.psdg
            .store()
            .query("build", "last_succeeded")
            .ok()?
            .filter(|f| f.confidence >= 0.7)
            .map(|f| f.object.as_str() == "true")
    }

    /// Get build errors (deserialized from JSON).
    pub fn get_build_errors(&self) -> Vec<String> {
        self.psdg
            .store()
            .query("build", "errors_json")
            .ok()
            .flatten()
            .and_then(|f| serde_json::from_str::<Vec<String>>(&f.object).ok())
            .unwrap_or_default()
    }

    /// Get the active debugging target binary.
    pub fn get_debug_target(&self) -> Option<String> {
        self.psdg
            .store()
            .query("debug", "target_binary")
            .ok()?
            .filter(|f| f.confidence >= 0.6)
            .map(|f| f.object)
    }

    /// Get the active debugging error being investigated.
    pub fn get_active_error(&self) -> Option<String> {
        self.psdg
            .store()
            .query("debug", "active_error")
            .ok()?
            .filter(|f| f.confidence >= 0.6)
            .map(|f| f.object)
    }

    /// Build a human-readable operational context summary.
    ///
    /// Used to inject workspace context into the system prompt.
    pub fn context_summary(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(ws) = self.get_workspace() {
            parts.push(format!("project: {} ({})", ws.name, ws.root.display()));
        }
        if let Some(branch) = self.get_branch() {
            parts.push(format!("branch: {}", branch));
        }
        if let Some(ok) = self.get_build_status() {
            let errors = self.get_build_errors();
            if !ok && !errors.is_empty() {
                parts.push(format!(
                    "build: FAILING — {}",
                    errors.first().map(|s| s.as_str()).unwrap_or("error")
                ));
            } else if !ok {
                parts.push("build: FAILING".into());
            } else {
                parts.push("build: passing".into());
            }
        }
        if let Some(debug_target) = self.get_debug_target() {
            parts.push(format!("debugging: {}", debug_target));
            if let Some(err) = self.get_active_error() {
                parts.push(format!("active error: {}", err));
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_memory() -> (WorkspaceMemory, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let psdg = PsdgHandle::open(tmp.path()).unwrap();
        (WorkspaceMemory::new(psdg), tmp)
    }

    #[tokio::test]
    async fn record_and_read_workspace() {
        let (mem, _tmp) = make_memory();
        // Write directly to store (bypass spawn_blocking for test)
        mem.psdg
            .store()
            .upsert(
                "workspace",
                "root",
                "/home/user/project",
                0.95,
                FactSource::Detected,
                "test",
            )
            .unwrap();
        mem.psdg
            .store()
            .upsert(
                "workspace",
                "name",
                "my-project",
                0.95,
                FactSource::Detected,
                "test",
            )
            .unwrap();

        let ws = mem.get_workspace().unwrap();
        assert_eq!(ws.name, "my-project");
    }

    #[tokio::test]
    async fn record_and_read_branch() {
        let (mem, _tmp) = make_memory();
        mem.psdg
            .store()
            .upsert(
                "git",
                "branch",
                "feature/new-api",
                0.97,
                FactSource::Detected,
                "test",
            )
            .unwrap();
        assert_eq!(mem.get_branch().as_deref(), Some("feature/new-api"));
    }

    #[tokio::test]
    async fn build_failure_is_reported() {
        let (mem, _tmp) = make_memory();
        mem.psdg
            .store()
            .upsert(
                "build",
                "last_succeeded",
                "false",
                0.9,
                FactSource::Detected,
                "test",
            )
            .unwrap();
        mem.psdg
            .store()
            .upsert(
                "build",
                "errors_json",
                r#"["cannot find function `main`"]"#,
                0.9,
                FactSource::Detected,
                "test",
            )
            .unwrap();

        assert_eq!(mem.get_build_status(), Some(false));
        let errors = mem.get_build_errors();
        assert!(!errors.is_empty());
    }

    #[tokio::test]
    async fn context_summary_includes_all_available_facts() {
        let (mem, _tmp) = make_memory();
        mem.psdg
            .store()
            .upsert(
                "workspace",
                "root",
                "/kria",
                0.95,
                FactSource::Detected,
                "test",
            )
            .unwrap();
        mem.psdg
            .store()
            .upsert(
                "workspace",
                "name",
                "kria",
                0.95,
                FactSource::Detected,
                "test",
            )
            .unwrap();
        mem.psdg
            .store()
            .upsert("git", "branch", "main", 0.97, FactSource::Detected, "test")
            .unwrap();

        let summary = mem.context_summary();
        assert!(summary.is_some());
        let s = summary.unwrap();
        assert!(s.contains("kria"));
        assert!(s.contains("main"));
    }

    #[tokio::test]
    async fn empty_store_context_summary_is_none() {
        let (mem, _tmp) = make_memory();
        assert!(mem.context_summary().is_none());
    }
}
