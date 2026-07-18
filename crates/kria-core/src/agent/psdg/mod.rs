//! Persistent Semantic Desktop Cognition (PSDG) — Batch 1.
//!
//! # Architecture
//!
//! The PSDG layer wires together existing disconnected components into a unified
//! persistent semantic desktop cognition runtime:
//!
//! ```text
//! PerceptionBus (existing)
//!     │ DesktopOp / FilesystemOp events
//!     ▼
//! PsdgCoordinator  ◄── background service (Priority::Background)
//!     │  writes via DesktopGraph API
//!     ▼
//! WorldModelStore  ◄── existing SQLite-backed (s,p,o) Bayesian graph
//!     │  (dedicated connection on the shared kria.db, WAL handles concurrency)
//!     ▲
//!     │ writes (fire-and-forget via spawn_blocking)
//! ├── EnvironmentStateTracker (post-grounding side-channel)
//! ├── BrowserStateTracker (post-CDP-fetch side-channel)
//! ├── IdeStateTracker (post-LSP-fetch side-channel)
//! └── StageExecutor PSDG bridge (workflow progress)
//!     │
//!     │ reads (bounded, confidence ≥ 0.5, < 20 facts)
//!     ▼
//! PsdgContextSnapshot  ◄── injected into TurnGate / AgentLoop system prompt
//! ```
//!
//! # Invariants
//!
//! 1. ALL writes are fire-and-forget (`spawn_blocking`). Write failures are logged
//!    at `debug` level and NEVER propagate to the caller.
//! 2. ALL reads are bounded (max `MAX_CONTEXT_FACTS` facts, confidence ≥ `MIN_READ_CONFIDENCE`).
//! 3. `PsdgHandle` is cheaply cloneable (`Arc` wrapper) — clone freely.
//! 4. `WorldModelStore` uses a dedicated WAL connection on the shared `kria.db`.
//!    SQLite WAL mode handles concurrent access safely.
//! 5. PSDG never bypasses safety/HITL gates — it is observational only.

pub mod context_injector;
pub mod coordinator;
pub mod env_tracker;
pub mod introspect;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tracing::{debug, info};

use crate::agent::world_model::{DesktopGraph, FactSource, WorldFact, WorldModelStore};

/// Maximum facts injected into per-turn context (token budget guard).
pub const MAX_CONTEXT_FACTS: usize = 20;

/// Minimum confidence for a fact to appear in context injection.
pub const MIN_READ_CONFIDENCE: f64 = 0.5;

/// Lightweight, cheaply-cloneable handle to the PSDG runtime.
///
/// All writes are fire-and-forget (`spawn_blocking`). All reads are bounded.
/// Clone this freely — it is `Arc`-backed.
/// Metrics snapshot from the last PSDG maintenance run.
#[derive(Debug, Clone, Default)]
pub struct PsdgMaintenanceMetrics {
    /// Total facts present before the last decay run.
    pub facts_before: u64,
    /// Total facts present after the last decay run.
    pub facts_after: u64,
    /// Facts archived (pruned) in the last decay run.
    pub facts_archived: u64,
    /// Facts whose confidence was decayed but remained above threshold.
    pub facts_decayed: u64,
    /// Contradictions resolved (old facts archived) in the last run.
    pub contradictions_resolved: u64,
    /// Archive facts pruned (deleted) in the last run.
    pub archive_pruned: u64,
    /// Unix timestamp of the last maintenance run.
    pub last_run_timestamp: u64,
}

/// Internal atomic metrics storage.
#[derive(Default)]
struct AtomicMetrics {
    facts_before: AtomicU64,
    facts_after: AtomicU64,
    facts_archived: AtomicU64,
    facts_decayed: AtomicU64,
    contradictions_resolved: AtomicU64,
    archive_pruned: AtomicU64,
    last_run_timestamp: AtomicU64,
}

impl AtomicMetrics {
    fn snapshot(&self) -> PsdgMaintenanceMetrics {
        PsdgMaintenanceMetrics {
            facts_before: self.facts_before.load(Ordering::Relaxed),
            facts_after: self.facts_after.load(Ordering::Relaxed),
            facts_archived: self.facts_archived.load(Ordering::Relaxed),
            facts_decayed: self.facts_decayed.load(Ordering::Relaxed),
            contradictions_resolved: self.contradictions_resolved.load(Ordering::Relaxed),
            archive_pruned: self.archive_pruned.load(Ordering::Relaxed),
            last_run_timestamp: self.last_run_timestamp.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
pub struct PsdgHandle {
    store: Arc<WorldModelStore>,
    /// Cumulative metrics for observability.
    metrics: Arc<AtomicMetrics>,
}

impl PsdgHandle {
    /// Create a new handle, opening a dedicated WAL connection to `db_path`.
    ///
    /// The db_path should match `MemoryStore`'s db_path. SQLite WAL mode
    /// handles concurrent access between the two connections safely.
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let store = Arc::new(WorldModelStore::open_path(db_path)?);
        Ok(Self {
            store,
            metrics: Arc::new(AtomicMetrics::default()),
        })
    }

    /// Create from an already-constructed store (for testing).
    pub fn from_store(store: Arc<WorldModelStore>) -> Self {
        Self {
            store,
            metrics: Arc::new(AtomicMetrics::default()),
        }
    }

    /// Access the underlying store (for advanced queries).
    pub fn store(&self) -> &WorldModelStore {
        &self.store
    }

    /// Cloned Arc to the store (for passing to sub-components).
    pub fn store_arc(&self) -> Arc<WorldModelStore> {
        self.store.clone()
    }

    // ─── Fire-and-forget write helpers ────────────────────────────────────────

    /// Record the currently focused desktop application.
    ///
    /// Called by `PsdgCoordinator` on `DesktopOp::FocusChanged` events.
    pub fn record_app_focus(&self, app_id: &str, app_name: &str) {
        let store = self.store.clone();
        let app_id = app_id.to_string();
        let app_name = app_name.to_string();
        tokio::task::spawn_blocking(move || {
            let graph = DesktopGraph::new(&store);
            if let Err(e) = graph.register_app(&app_id, &app_name) {
                debug!(target: "psdg", error = %e, app_id, "register_app failed (non-fatal)");
            }
            if let Err(e) = graph.set_focused_app(&app_id) {
                debug!(target: "psdg", error = %e, app_id, "set_focused_app failed (non-fatal)");
            }
        });
    }

    /// Record a browser navigation event (URL + title).
    ///
    /// Called by `BrowserCognitionEngine` after each state fetch.
    pub fn record_browser_navigation(&self, url: &str, title: &str) {
        let store = self.store.clone();
        let url = url.to_string();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || {
            let graph = DesktopGraph::new(&store);
            if let Err(e) = graph.register_browser_navigation("browser_primary", &url, &title) {
                debug!(target: "psdg", error = %e, url, "browser nav record failed (non-fatal)");
            }
        });
    }

    /// Record IDE workspace state.
    ///
    /// Called by `IdeCognitionEngine` after each state fetch.
    pub fn record_ide_state(
        &self,
        workspace_root: &str,
        active_file: Option<&str>,
        error_count: usize,
    ) {
        let store = self.store.clone();
        let workspace_root = workspace_root.to_string();
        let active_file = active_file.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            if let Err(e) = store.upsert(
                "ide_primary",
                "workspace_root",
                &workspace_root,
                0.95,
                FactSource::Detected,
                "IDE cognition engine",
            ) {
                debug!(target: "psdg", error = %e, "ide workspace_root write failed (non-fatal)");
            }
            if let Some(ref file) = active_file {
                if let Err(e) = store.upsert(
                    "ide_primary",
                    "active_file",
                    file,
                    0.90,
                    FactSource::Detected,
                    "IDE cognition engine",
                ) {
                    debug!(target: "psdg", error = %e, "ide active_file write failed (non-fatal)");
                }
            }
            if let Err(e) = store.upsert(
                "ide_primary",
                "error_count",
                &error_count.to_string(),
                0.95,
                FactSource::Detected,
                "IDE cognition engine",
            ) {
                debug!(target: "psdg", error = %e, "ide error_count write failed (non-fatal)");
            }
        });
    }

    /// Record workflow stage progress.
    ///
    /// Called by `StageExecutor` on stage completion/failure.
    pub fn record_workflow_stage(
        &self,
        workflow_id: &str,
        stage_label: &str,
        outcome: &str,
        artifacts: &[String],
    ) {
        let store = self.store.clone();
        let workflow_id = workflow_id.to_string();
        let stage_label = stage_label.to_string();
        let outcome = outcome.to_string();
        let artifacts_str = artifacts.join(", ");
        tokio::task::spawn_blocking(move || {
            // Persist stage outcome
            if let Err(e) = store.upsert(
                &workflow_id,
                &format!("stage_{}", stage_label),
                &outcome,
                0.99,
                FactSource::Detected,
                "StageExecutor PSDG bridge",
            ) {
                debug!(target: "psdg", error = %e, workflow_id, stage_label, "workflow stage write failed (non-fatal)");
                return;
            }
            // Mark as active workflow on desktop
            if outcome == "completed" || outcome == "in_progress" {
                let _ = store.upsert(
                    "desktop_environment",
                    "active_workflow",
                    &workflow_id,
                    0.95,
                    FactSource::Detected,
                    "StageExecutor PSDG bridge",
                );
            }
            // Persist artifacts
            if !artifacts_str.is_empty() {
                let _ = store.upsert(
                    &workflow_id,
                    "artifacts",
                    &artifacts_str,
                    0.99,
                    FactSource::Detected,
                    "StageExecutor PSDG bridge",
                );
            }
        });
    }

    /// Record arbitrary desktop fact (generic write path).
    ///
    /// Used by `EnvironmentStateTracker` and `PsdgCoordinator`.
    pub fn record_fact(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f64,
        source: FactSource,
        evidence: &str,
    ) {
        let store = self.store.clone();
        let subject = subject.to_string();
        let predicate = predicate.to_string();
        let object = object.to_string();
        let evidence = evidence.to_string();
        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                store.upsert(&subject, &predicate, &object, confidence, source, &evidence)
            {
                debug!(target: "psdg", error = %e, subject, predicate, "fact write failed (non-fatal)");
            }
        });
    }

    // ─── Bounded synchronous reads ────────────────────────────────────────────

    /// Get the currently focused application name. Returns `None` if unknown.
    pub fn get_focused_app(&self) -> Option<String> {
        self.store
            .query("desktop_environment", "focused_app")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Get the primary browser URL. Returns `None` if unknown.
    pub fn get_browser_url(&self) -> Option<String> {
        self.store
            .query("browser_primary", "current_url")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Get the primary browser page title. Returns `None` if unknown.
    pub fn get_browser_title(&self) -> Option<String> {
        self.store
            .query("browser_primary", "current_title")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Get the IDE workspace root. Returns `None` if unknown.
    pub fn get_ide_workspace(&self) -> Option<String> {
        self.store
            .query("ide_primary", "workspace_root")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Get the active IDE file. Returns `None` if unknown.
    pub fn get_ide_active_file(&self) -> Option<String> {
        self.store
            .query("ide_primary", "active_file")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Get the currently active workflow ID. Returns `None` if none.
    pub fn get_active_workflow(&self) -> Option<String> {
        self.store
            .query("desktop_environment", "active_workflow")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Get the terminal working directory. Returns `None` if unknown.
    pub fn get_terminal_cwd(&self) -> Option<String> {
        self.store
            .query("terminal_primary", "cwd")
            .ok()
            .flatten()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .map(|f| f.object)
    }

    /// Query all facts for a subject, bounded to `MAX_CONTEXT_FACTS`.
    ///
    /// Only returns facts with confidence ≥ `MIN_READ_CONFIDENCE`.
    pub fn query_subject_bounded(&self, subject: &str) -> Vec<WorldFact> {
        self.store
            .query_subject(subject)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .take(MAX_CONTEXT_FACTS)
            .collect()
    }

    /// Build a bounded context snapshot for LLM prompt injection.
    ///
    /// Collects all key desktop state facts with confidence ≥ 0.5.
    /// Total facts capped at `MAX_CONTEXT_FACTS` to stay within token budget.
    pub fn get_context_snapshot(&self) -> PsdgContextSnapshot {
        PsdgContextSnapshot {
            focused_app: self.get_focused_app(),
            browser_url: self.get_browser_url(),
            browser_title: self.get_browser_title(),
            ide_workspace: self.get_ide_workspace(),
            ide_active_file: self.get_ide_active_file(),
            active_workflow: self.get_active_workflow(),
            terminal_cwd: self.get_terminal_cwd(),
        }
    }

    /// Access the introspection surface for read-only graph inspection.
    ///
    /// Use this for debugging cognition state, inspecting beliefs,
    /// verifying context propagation, and checking graph health.
    pub fn introspect(&self) -> introspect::PsdgIntrospector<'_> {
        introspect::PsdgIntrospector::new(self)
    }

    /// Return a snapshot of the latest maintenance metrics.
    pub fn maintenance_metrics(&self) -> PsdgMaintenanceMetrics {
        self.metrics.snapshot()
    }

    /// Run confidence decay, archive stale facts, prune old archive, and resolve contradictions.
    ///
    /// Should be called periodically (e.g., once per hour) from background maintenance.
    /// Emits structured tracing and updates cumulative metrics.
    pub fn run_decay(&self) {
        let store = self.store.clone();
        let metrics = Arc::clone(&self.metrics);
        tokio::task::spawn_blocking(move || {
            let _span = tracing::info_span!("psdg_maintenance").entered();
            let start = std::time::Instant::now();

            // Pre-run stats
            let facts_before = store.stats().map(|s| s.total_facts as u64).unwrap_or(0);

            // 1. Prune archive older than 30 days to prevent unbounded growth.
            let archive_pruned = store.prune_archive(30).unwrap_or(0);

            // 2. Resolve contradictions: archive duplicate/conflicting facts.
            let contradictions_resolved = store.resolve_contradictions().unwrap_or(0);

            // 3. Decay and archive stale facts.
            let facts_archived = match store.decay_and_archive(0.1) {
                Ok(n) => n as u64,
                Err(e) => {
                    debug!(target: "psdg", error = %e, "fact decay failed (non-fatal)");
                    0
                }
            };

            // Post-run stats
            let facts_after = store.stats().map(|s| s.total_facts as u64).unwrap_or(0);
            let facts_decayed = facts_before
                .saturating_sub(facts_after)
                .saturating_sub(facts_archived);

            // Update metrics atomically
            metrics.facts_before.store(facts_before, Ordering::Relaxed);
            metrics.facts_after.store(facts_after, Ordering::Relaxed);
            metrics
                .facts_archived
                .store(facts_archived, Ordering::Relaxed);
            metrics
                .facts_decayed
                .store(facts_decayed, Ordering::Relaxed);
            metrics
                .contradictions_resolved
                .store(contradictions_resolved as u64, Ordering::Relaxed);
            metrics
                .archive_pruned
                .store(archive_pruned as u64, Ordering::Relaxed);
            metrics.last_run_timestamp.store(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                Ordering::Relaxed,
            );

            info!(
                target: "psdg_maintenance",
                facts_before,
                facts_after,
                facts_archived,
                facts_decayed,
                contradictions_resolved,
                archive_pruned,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "PSDG maintenance completed"
            );
        });
    }
}

/// A bounded snapshot of the PSDG semantic state for LLM context injection.
///
/// Used by `TurnGate` and `AgentLoop` to inject desktop context into
/// per-turn system prompts. All fields are `Option<String>` — missing
/// context is silently omitted from the prompt block.
#[derive(Debug, Clone, Default)]
pub struct PsdgContextSnapshot {
    /// Currently focused desktop application.
    pub focused_app: Option<String>,
    /// Active browser URL.
    pub browser_url: Option<String>,
    /// Active browser page title.
    pub browser_title: Option<String>,
    /// IDE workspace root directory.
    pub ide_workspace: Option<String>,
    /// Currently active IDE file.
    pub ide_active_file: Option<String>,
    /// Active workflow ID (if any).
    pub active_workflow: Option<String>,
    /// Terminal working directory.
    pub terminal_cwd: Option<String>,
}

impl PsdgContextSnapshot {
    /// Returns `true` if the snapshot contains any non-None values.
    pub fn is_empty(&self) -> bool {
        self.focused_app.is_none()
            && self.browser_url.is_none()
            && self.browser_title.is_none()
            && self.ide_workspace.is_none()
            && self.ide_active_file.is_none()
            && self.active_workflow.is_none()
            && self.terminal_cwd.is_none()
    }

    /// Format as a compact system-prompt block (≤ ~200 tokens).
    ///
    /// Returns `None` if the snapshot is empty (no context to inject).
    pub fn to_prompt_block(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut lines = Vec::with_capacity(8);
        lines.push("## Desktop Context (live)".to_string());
        if let Some(ref app) = self.focused_app {
            lines.push(format!("- Focused app: {app}"));
        }
        if let Some(ref url) = self.browser_url {
            let title_suffix = self
                .browser_title
                .as_ref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default();
            lines.push(format!("- Browser: {url}{title_suffix}"));
        }
        if let Some(ref ws) = self.ide_workspace {
            lines.push(format!("- IDE workspace: {ws}"));
        }
        if let Some(ref file) = self.ide_active_file {
            lines.push(format!("- Active file: {file}"));
        }
        if let Some(ref cwd) = self.terminal_cwd {
            lines.push(format!("- Terminal cwd: {cwd}"));
        }
        if let Some(ref wf) = self.active_workflow {
            lines.push(format!("- Active workflow: {wf}"));
        }
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_handle() -> PsdgHandle {
        let tmp = NamedTempFile::new().unwrap();
        PsdgHandle::open(tmp.path()).unwrap()
    }

    #[test]
    fn empty_snapshot_returns_none() {
        let handle = make_handle();
        let snap = handle.get_context_snapshot();
        assert!(snap.is_empty());
        assert!(snap.to_prompt_block().is_none());
    }

    #[test]
    fn snapshot_with_app_has_content() {
        let handle = make_handle();
        // Directly write a fact synchronously for testing
        handle
            .store()
            .upsert(
                "desktop_environment",
                "focused_app",
                "firefox",
                0.95,
                FactSource::Detected,
                "test",
            )
            .unwrap();
        let snap = handle.get_context_snapshot();
        assert_eq!(snap.focused_app.as_deref(), Some("firefox"));
        assert!(!snap.is_empty());
        let block = snap.to_prompt_block().unwrap();
        assert!(block.contains("firefox"));
    }

    #[test]
    fn query_subject_bounded_respects_confidence() {
        let handle = make_handle();
        handle
            .store()
            .upsert(
                "test_app",
                "is_a",
                "application",
                0.99,
                FactSource::Detected,
                "t",
            )
            .unwrap();
        handle
            .store()
            .upsert(
                "test_app",
                "low_conf",
                "value",
                0.1,
                FactSource::Inferred,
                "t",
            )
            .unwrap();
        let facts = handle.query_subject_bounded("test_app");
        // Only the high-confidence fact should appear
        assert!(facts.iter().all(|f| f.confidence >= MIN_READ_CONFIDENCE));
    }
}
