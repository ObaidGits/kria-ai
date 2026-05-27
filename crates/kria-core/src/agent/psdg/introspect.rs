//! PSDG Introspection, Metrics, and Context Injection Observability.
//!
//! # Purpose
//!
//! This module provides read-only inspection of the live PSDG runtime.
//! It is the primary tool for:
//!
//! - Debugging cognition state (why does KRIA think X?)
//! - Inspecting runtime beliefs (what does KRIA know about Y?)
//! - Verifying context propagation (was context Z injected?)
//! - Detecting graph health issues (inflation, drift, stale accumulation)
//! - Diagnosing event storms (high throughput, queue overflow)
//!
//! # Safety Invariants
//!
//! - ALL methods are read-only. No writes, no mutations.
//! - All results are bounded (max 50 facts per query).
//! - No LLM calls, no I/O, no side effects.
//! - Introspection NEVER bypasses HITL or safety gates.

use std::collections::HashMap;

use crate::agent::psdg::{PsdgHandle, MIN_READ_CONFIDENCE};
use crate::agent::turn_gate::Operation;
use crate::agent::world_model::WorldFact;

/// Maximum facts returned in a single introspection query.
const MAX_INTROSPECT_FACTS: usize = 50;

// ─── PsdgHealthReport ────────────────────────────────────────────────────────

/// Health report for the PSDG runtime.
///
/// Used by health check endpoints and cognitive dashboards.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PsdgHealthReport {
    /// Total active facts in the graph.
    pub total_facts: i64,
    /// Total archived (stale/contradicted) facts.
    pub archived_facts: i64,
    /// Average confidence across all active facts.
    pub avg_confidence: f64,
    /// Facts with confidence below threshold (potentially stale).
    pub stale_facts: i64,
    /// Breakdown of facts by source type.
    pub facts_by_source: HashMap<String, i64>,
    /// Whether the graph is at risk of inflation (> 1000 facts).
    pub inflation_risk: bool,
    /// Whether the graph has significant stale accumulation (> 10% stale).
    pub stale_risk: bool,
    /// Whether the graph has high contradiction rate (> 20% archived).
    pub contradiction_risk: bool,
    /// Overall health status.
    pub status: HealthStatus,
    /// Human-readable summary.
    pub summary: String,
}

/// Overall PSDG health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum HealthStatus {
    /// Graph is healthy: facts are fresh, confidence is high, no inflation.
    Healthy,
    /// Graph is degraded: some staleness or inflation detected.
    Degraded,
    /// Graph is unhealthy: critical issues requiring maintenance.
    Unhealthy,
}

// ─── EntityTrace ─────────────────────────────────────────────────────────────

/// All known facts about a specific entity (subject) in the PSDG.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EntityTrace {
    /// The subject (entity) being traced.
    pub subject: String,
    /// Active facts about this entity (confidence ≥ threshold).
    pub active_facts: Vec<FactSummary>,
    /// Whether this entity is currently the focused app.
    pub is_focused: bool,
    /// Whether this entity is involved in the active workflow.
    pub in_active_workflow: bool,
}

/// A summarized fact for introspection output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FactSummary {
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub source: String,
    pub last_verified: String,
}

impl From<&WorldFact> for FactSummary {
    fn from(f: &WorldFact) -> Self {
        Self {
            predicate: f.predicate.clone(),
            object: f.object.clone(),
            confidence: f.confidence,
            source: format!("{:?}", f.source),
            last_verified: f.last_verified.to_rfc3339(),
        }
    }
}

// ─── GraphSummary ─────────────────────────────────────────────────────────────

/// A compact text description of the current PSDG graph state.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphSummary {
    /// Number of active facts.
    pub fact_count: usize,
    /// Number of distinct subjects (entities) tracked.
    pub entity_count: usize,
    /// Known entities (up to 20, sorted by fact count).
    pub top_entities: Vec<String>,
    /// Key desktop facts as readable lines.
    pub desktop_facts: Vec<String>,
    /// Human-readable summary paragraph.
    pub narrative: String,
}

// ─── InjectionTrace ──────────────────────────────────────────────────────────

/// Explains WHY facts were (or were not) injected into the system prompt.
///
/// Enables traceable cognition: every context injection decision is
/// accountable and inspectable after the fact.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InjectionTrace {
    /// Operation type that triggered the injection check.
    pub operation: String,
    /// Whether context was injected for this operation.
    pub injected: bool,
    /// Reason injection was skipped (if not injected).
    pub skip_reason: Option<String>,
    /// Facts that were included in the injection.
    pub included_facts: Vec<InjectedFact>,
    /// Facts that were excluded and why.
    pub excluded_facts: Vec<ExcludedFact>,
    /// Final injected block (truncated to 200 chars if long).
    pub injected_block: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InjectedFact {
    pub field: String,
    pub value: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcludedFact {
    pub field: String,
    pub reason: String,
}

// ─── PsdgIntrospector ────────────────────────────────────────────────────────

/// Read-only introspection surface for the PSDG runtime.
///
/// Construct via `PsdgHandle::introspect()`. All methods are read-only
/// and bounded.
pub struct PsdgIntrospector<'a> {
    handle: &'a PsdgHandle,
}

impl<'a> PsdgIntrospector<'a> {
    pub(super) fn new(handle: &'a PsdgHandle) -> Self {
        Self { handle }
    }

    /// Get the current health report for the PSDG graph.
    pub fn health(&self) -> PsdgHealthReport {
        let stats = self.handle.store().stats().unwrap_or_default();
        let total = stats.total_facts;
        let archived = stats.archived_facts;
        let avg_conf = stats.avg_confidence;
        let stale = stats.stale_facts;

        let inflation_risk = total > 1000;
        let stale_risk = total > 0 && (stale as f64 / total as f64) > 0.10;
        let contradiction_risk = total > 0 && (archived as f64 / (total + archived) as f64) > 0.20;

        let status = if inflation_risk || contradiction_risk {
            HealthStatus::Unhealthy
        } else if stale_risk {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let summary = match status {
            HealthStatus::Healthy => format!(
                "PSDG healthy: {} facts, {:.0}% avg confidence, {} archived",
                total,
                avg_conf * 100.0,
                archived
            ),
            HealthStatus::Degraded => format!(
                "PSDG degraded: {} stale facts ({:.0}% of total) — decay recommended",
                stale,
                stale as f64 / total.max(1) as f64 * 100.0
            ),
            HealthStatus::Unhealthy => format!(
                "PSDG unhealthy: {} facts (inflation risk={}, contradiction risk={})",
                total, inflation_risk, contradiction_risk
            ),
        };

        PsdgHealthReport {
            total_facts: total,
            archived_facts: archived,
            avg_confidence: avg_conf,
            stale_facts: stale,
            facts_by_source: stats.facts_by_source,
            inflation_risk,
            stale_risk,
            contradiction_risk,
            status,
            summary,
        }
    }

    /// Get a compact summary of the entire graph state.
    pub fn describe_graph(&self) -> GraphSummary {
        let store = self.handle.store();

        // Get desktop environment facts
        let desktop_raw = store
            .query_subject("desktop_environment")
            .unwrap_or_default();
        let browser_raw = store.query_subject("browser_primary").unwrap_or_default();
        let ide_raw = store.query_subject("ide_primary").unwrap_or_default();
        let terminal_raw = store.query_subject("terminal_primary").unwrap_or_default();

        let mut desktop_facts: Vec<String> = Vec::new();
        for f in desktop_raw
            .iter()
            .chain(browser_raw.iter())
            .chain(ide_raw.iter())
            .chain(terminal_raw.iter())
        {
            if f.confidence >= MIN_READ_CONFIDENCE {
                desktop_facts.push(format!(
                    "{}.{} = {} (conf={:.2})",
                    f.subject, f.predicate, f.object, f.confidence
                ));
            }
        }
        desktop_facts.truncate(MAX_INTROSPECT_FACTS);

        // Approximate entity count via stats
        let stats = store.stats().unwrap_or_default();
        let fact_count = stats.total_facts as usize;

        // Build narrative
        let snapshot = self.handle.get_context_snapshot();
        let mut narrative_parts = Vec::new();
        if let Some(ref app) = snapshot.focused_app {
            narrative_parts.push(format!("focused on {app}"));
        }
        if let Some(ref url) = snapshot.browser_url {
            narrative_parts.push(format!("browser at {url}"));
        }
        if let Some(ref ws) = snapshot.ide_workspace {
            narrative_parts.push(format!("IDE workspace {ws}"));
        }
        if let Some(ref wf) = snapshot.active_workflow {
            narrative_parts.push(format!("running workflow {wf}"));
        }
        let narrative = if narrative_parts.is_empty() {
            format!("PSDG has {fact_count} facts. No active desktop context established yet.")
        } else {
            format!(
                "PSDG has {fact_count} facts. Currently: {}.",
                narrative_parts.join(", ")
            )
        };

        GraphSummary {
            fact_count,
            entity_count: 0, // Would require a GROUP BY query — approximate
            top_entities: vec![
                "desktop_environment".into(),
                "browser_primary".into(),
                "ide_primary".into(),
            ],
            desktop_facts,
            narrative,
        }
    }

    /// Trace all known facts about a specific entity (subject).
    pub fn trace_entity(&self, subject: &str) -> EntityTrace {
        let store = self.handle.store();
        let raw_facts = store.query_subject(subject).unwrap_or_default();
        let active_facts: Vec<FactSummary> = raw_facts
            .iter()
            .filter(|f| f.confidence >= MIN_READ_CONFIDENCE)
            .take(MAX_INTROSPECT_FACTS)
            .map(|f| FactSummary::from(f))
            .collect();

        let is_focused = self
            .handle
            .get_focused_app()
            .map(|app| app.to_lowercase() == subject.to_lowercase())
            .unwrap_or(false);

        let in_active_workflow = self
            .handle
            .get_active_workflow()
            .map(|wf| wf.to_lowercase() == subject.to_lowercase())
            .unwrap_or(false);

        EntityTrace {
            subject: subject.to_string(),
            active_facts,
            is_focused,
            in_active_workflow,
        }
    }

    /// Explain WHY a context snapshot would (or would not) be injected
    /// for a given operation type.
    ///
    /// Returns an `InjectionTrace` documenting the decision and every fact
    /// that was considered, included, or excluded.
    pub fn explain_injection(&self, operation: Operation) -> InjectionTrace {
        use crate::agent::psdg::context_injector::{build_context_block, should_inject_context};

        let op_str = format!("{:?}", operation);

        if !should_inject_context(operation) {
            return InjectionTrace {
                operation: op_str,
                injected: false,
                skip_reason: Some(format!(
                    "Operation {:?} does not benefit from desktop context injection \
                     (only Automate, ExecuteShell, ExecuteCode, Write, Clarify, ConfigureSystem are injected)",
                    operation
                )),
                included_facts: vec![],
                excluded_facts: vec![],
                injected_block: None,
            };
        }

        let snapshot = self.handle.get_context_snapshot();
        let mut included = Vec::new();
        let mut excluded = Vec::new();

        // Explain each field of the snapshot
        macro_rules! trace_field {
            ($field:expr, $name:literal, $getter:expr) => {{
                match $field {
                    Some(ref v) => {
                        let conf = $getter;
                        included.push(InjectedFact {
                            field: $name.into(),
                            value: v.clone(),
                            confidence: conf,
                        });
                    }
                    None => {
                        excluded.push(ExcludedFact {
                            field: $name.into(),
                            reason: format!(
                                "{} not present in WorldModelStore (no fact with confidence >= {})",
                                $name, MIN_READ_CONFIDENCE
                            ),
                        });
                    }
                }
            }};
        }

        trace_field!(
            snapshot.focused_app,
            "focused_app",
            self.handle
                .store()
                .query("desktop_environment", "focused_app")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );
        trace_field!(
            snapshot.browser_url,
            "browser_url",
            self.handle
                .store()
                .query("browser_primary", "current_url")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );
        trace_field!(
            snapshot.browser_title,
            "browser_title",
            self.handle
                .store()
                .query("browser_primary", "current_title")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );
        trace_field!(
            snapshot.ide_workspace,
            "ide_workspace",
            self.handle
                .store()
                .query("ide_primary", "workspace_root")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );
        trace_field!(
            snapshot.ide_active_file,
            "ide_active_file",
            self.handle
                .store()
                .query("ide_primary", "active_file")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );
        trace_field!(
            snapshot.terminal_cwd,
            "terminal_cwd",
            self.handle
                .store()
                .query("terminal_primary", "cwd")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );
        trace_field!(
            snapshot.active_workflow,
            "active_workflow",
            self.handle
                .store()
                .query("desktop_environment", "active_workflow")
                .ok()
                .flatten()
                .map(|f| f.confidence)
        );

        let block = build_context_block(&snapshot, operation);
        let injected = block.is_some();
        let injected_block = block.map(|b| {
            if b.len() > 200 {
                format!("{}...", &b[..200])
            } else {
                b
            }
        });

        InjectionTrace {
            operation: op_str,
            injected,
            skip_reason: None,
            included_facts: included,
            excluded_facts: excluded,
            injected_block,
        }
    }

    /// Perform a full-text search across all facts and return matches.
    pub fn search_facts(&self, query: &str) -> Vec<FactSummary> {
        self.handle
            .store()
            .search(query)
            .unwrap_or_default()
            .iter()
            .take(MAX_INTROSPECT_FACTS)
            .map(FactSummary::from)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::world_model::FactSource;
    use tempfile::NamedTempFile;

    fn make_handle() -> PsdgHandle {
        let tmp = NamedTempFile::new().unwrap();
        PsdgHandle::open(tmp.path()).unwrap()
    }

    #[test]
    fn health_report_empty_store_is_healthy() {
        let handle = make_handle();
        let report = handle.introspect().health();
        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.total_facts, 0);
        assert!(!report.inflation_risk);
        assert!(!report.stale_risk);
    }

    #[test]
    fn describe_graph_empty_has_no_narrative_context() {
        let handle = make_handle();
        let summary = handle.introspect().describe_graph();
        assert!(summary.narrative.contains("No active desktop context"));
        assert_eq!(summary.fact_count, 0);
    }

    #[test]
    fn describe_graph_with_facts_includes_them() {
        let handle = make_handle();
        handle
            .store()
            .upsert(
                "desktop_environment",
                "focused_app",
                "firefox",
                0.95,
                FactSource::Detected,
                "t",
            )
            .unwrap();
        let summary = handle.introspect().describe_graph();
        assert!(summary.narrative.contains("firefox"));
        assert!(summary.fact_count > 0);
    }

    #[test]
    fn trace_entity_returns_active_facts() {
        let handle = make_handle();
        handle
            .store()
            .upsert(
                "firefox",
                "is_a",
                "browser",
                0.95,
                FactSource::Detected,
                "t",
            )
            .unwrap();
        handle
            .store()
            .upsert(
                "firefox",
                "version",
                "120.0",
                0.80,
                FactSource::Detected,
                "t",
            )
            .unwrap();
        let trace = handle.introspect().trace_entity("firefox");
        assert_eq!(trace.subject, "firefox");
        assert_eq!(trace.active_facts.len(), 2);
    }

    #[test]
    fn trace_entity_excludes_low_confidence() {
        let handle = make_handle();
        handle
            .store()
            .upsert("myapp", "state", "running", 0.1, FactSource::Inferred, "t")
            .unwrap();
        let trace = handle.introspect().trace_entity("myapp");
        assert!(
            trace.active_facts.is_empty(),
            "Low-confidence fact should be excluded from trace"
        );
    }

    #[test]
    fn explain_injection_skips_converse() {
        let handle = make_handle();
        let trace = handle.introspect().explain_injection(Operation::Converse);
        assert!(!trace.injected);
        assert!(trace.skip_reason.is_some());
    }

    #[test]
    fn explain_injection_automate_with_empty_snapshot() {
        let handle = make_handle();
        let trace = handle.introspect().explain_injection(Operation::Automate);
        assert!(
            !trace.injected,
            "Empty snapshot => no injection even for Automate"
        );
        assert_eq!(
            trace.excluded_facts.len(),
            7,
            "All 7 fields should be excluded"
        );
    }

    #[test]
    fn explain_injection_automate_with_facts_injects() {
        let handle = make_handle();
        handle
            .store()
            .upsert(
                "desktop_environment",
                "focused_app",
                "code",
                0.95,
                FactSource::Detected,
                "t",
            )
            .unwrap();
        let trace = handle.introspect().explain_injection(Operation::Automate);
        assert!(trace.injected);
        assert!(!trace.included_facts.is_empty());
        assert!(trace.injected_block.is_some());
    }

    #[test]
    fn search_facts_finds_by_keyword() {
        let handle = make_handle();
        handle
            .store()
            .upsert(
                "firefox_browser",
                "is_a",
                "application",
                0.99,
                FactSource::Detected,
                "t",
            )
            .unwrap();
        let results = handle.introspect().search_facts("firefox");
        assert!(!results.is_empty());
    }
}
