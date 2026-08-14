//! The closed, in-tree workflow registry (Task 4.5, OSC-027/OSC-028).
//!
//! A workflow is **code**, not data. There is no API, no file and no parameter
//! through which a caller can supply a workflow body: `run_workflow` can only
//! name an id that already exists in [`IN_TREE_WORKFLOWS`], and every step is a
//! closed [`WorkflowStepAction`] variant. That is what keeps a workflow from
//! becoming a persistent way to run something the safety layer never saw.
//!
//! The registry is intentionally **empty** in this build. That is a fact, not a
//! placeholder: no workflow has been through review yet, so `list_workflows`
//! truthfully reports an empty page and `run_workflow` truthfully reports that
//! no such workflow exists. Adding one is a reviewed code change, and the
//! machinery around it (paging, id lookup, revision compare, per-step
//! execution and verification) is exercised against fixture registries in the
//! tests below and in [`super::fake`].

use crate::os_control::automation::typed::{Revision, WorkflowId};
use crate::os_control::contract::{Digest, SafeField, SafeText};
use crate::os_control::error::OsControlError;
use crate::safety::RiskLevel;

/// Maximum items a single workflow page may carry (frozen `WorkflowPage.items`
/// `maxItems`, `x-configBound: page_size`).
pub const WORKFLOW_PAGE_MAX_ITEMS: usize = 256;

/// Default page size when the caller omits `limit`.
pub const WORKFLOW_PAGE_DEFAULT_ITEMS: usize = 50;

/// Maximum steps a single in-tree workflow may declare.
pub const WORKFLOW_MAX_STEPS: usize = 32;

/// A single closed step action.
///
/// Every variant names a *typed* operation with fixed, in-tree arguments. There
/// is deliberately no variant carrying a command, a script, a URL or a
/// caller-supplied argument vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStepAction {
    /// Enable or disable a named KRIA-owned systemd user timer.
    SetTimerEnabled {
        /// The fully-qualified `.timer` unit, fixed in tree.
        unit: &'static str,
        /// The desired enablement.
        enabled: bool,
    },
}

impl WorkflowStepAction {
    /// The risk this step carries, from the operation it performs.
    #[must_use]
    pub const fn risk(&self) -> RiskLevel {
        match self {
            // Toggling a timer's enablement is the YELLOW
            // `modify_scheduled_task` operation.
            WorkflowStepAction::SetTimerEnabled { .. } => RiskLevel::Yellow,
        }
    }

    /// Whether this step can be undone by its own inverse.
    #[must_use]
    pub const fn reversible(&self) -> bool {
        match self {
            // Re-enabling / re-disabling a timer restores the prior state
            // exactly, so a compensation is real.
            WorkflowStepAction::SetTimerEnabled { .. } => true,
        }
    }

    /// The stable token used in the workflow digest.
    #[must_use]
    pub fn canonical(&self) -> String {
        match self {
            WorkflowStepAction::SetTimerEnabled { unit, enabled } => {
                format!("set_timer_enabled:{unit}:{enabled}")
            }
        }
    }
}

/// One reviewed step of a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowStep {
    /// The step's stable id (frozen `SafeStepId`), unique within the workflow.
    pub step_id: &'static str,
    /// The typed action.
    pub action: WorkflowStepAction,
}

/// A reviewed workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowDescriptor {
    /// The workflow's stable id. `run_workflow` matches on this and never on a
    /// display name.
    pub id: &'static str,
    /// The definition revision, bumped whenever the steps change in tree.
    pub revision: Revision,
    /// The ordered steps.
    pub steps: &'static [WorkflowStep],
}

impl WorkflowDescriptor {
    /// The strongest risk any step carries — the frozen
    /// `maximum_step_risk(workflow_id, expected_revision)`.
    #[must_use]
    pub fn max_step_risk(&self) -> RiskLevel {
        self.steps
            .iter()
            .map(|step| step.action.risk())
            .max()
            .unwrap_or(RiskLevel::Green)
    }

    /// Whether every step can be compensated. A workflow with one irreversible
    /// step must never advertise a rollback for the whole run.
    #[must_use]
    pub fn fully_reversible(&self) -> bool {
        self.steps.iter().all(|step| step.action.reversible())
    }

    /// A digest binding the id, revision and every step.
    #[must_use]
    pub fn digest(&self) -> Digest {
        let steps = self
            .steps
            .iter()
            .map(|step| format!("{}={}", step.step_id, step.action.canonical()))
            .collect::<Vec<_>>()
            .join(";");
        Digest::of_str(&format!("workflow:{}:{}:{}", self.id, self.revision, steps))
    }

    /// Structural self-check: bounded step count, non-empty unique step ids, no
    /// BLACK step. Enforced in a test so a bad registry entry cannot ship.
    pub fn check(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("workflow id must not be empty".to_string());
        }
        if self.steps.is_empty() {
            return Err(format!("workflow `{}` has no steps", self.id));
        }
        if self.steps.len() > WORKFLOW_MAX_STEPS {
            return Err(format!("workflow `{}` exceeds the step bound", self.id));
        }
        let mut seen = std::collections::BTreeSet::new();
        for step in self.steps {
            if step.step_id.is_empty() {
                return Err(format!("workflow `{}` has an unnamed step", self.id));
            }
            if !seen.insert(step.step_id) {
                return Err(format!(
                    "workflow `{}` repeats step id `{}`",
                    self.id, step.step_id
                ));
            }
            if step.action.risk() == RiskLevel::Black {
                return Err(format!(
                    "workflow `{}` step `{}` is BLACK and may never be registered",
                    self.id, step.step_id
                ));
            }
        }
        Ok(())
    }
}

/// The closed set of reviewed workflows. Empty by design in this build.
pub const IN_TREE_WORKFLOWS: &[WorkflowDescriptor] = &[];

/// Look up a workflow by its stable id in a registry.
///
/// Returns `None` for an id the registry does not contain — an unknown id can
/// never execute.
#[must_use]
pub fn descriptor<'a>(
    registry: &'a [WorkflowDescriptor],
    id: &WorkflowId,
) -> Option<&'a WorkflowDescriptor> {
    registry.iter().find(|w| w.id == id.as_str())
}

// ─────────────────────────────────────────────────────────────────────────────
// Paging
// ─────────────────────────────────────────────────────────────────────────────

/// One page of workflow descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPage {
    /// The descriptors in this page.
    pub items: Vec<WorkflowDescriptor>,
    /// The opaque cursor for the next page, when more remain.
    pub next_cursor: Option<String>,
    /// Whether more items exist beyond this page.
    pub truncated: bool,
}

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

/// Mint an integrity-checked opaque cursor for a page offset.
///
/// The cursor carries only an offset into a listing of public-local workflow
/// metadata, so it needs integrity (a tampered cursor is rejected instead of
/// silently skipping entries) rather than secrecy. It is deliberately not
/// presented as a MAC.
#[must_use]
pub fn encode_cursor(offset: usize) -> String {
    let check = Digest::of_str(&format!("workflow-cursor:{offset}"));
    format!("w1.{offset}.{}", &check.as_hex()[..16])
}

/// Decode and integrity-check a cursor minted by [`encode_cursor`].
pub fn decode_cursor(cursor: &str) -> Result<usize, OsControlError> {
    let field = "cursor";
    if cursor.len() > 512 {
        return Err(invalid(field, "cursor exceeds the maximum length"));
    }
    let mut parts = cursor.split('.');
    match parts.next() {
        Some("w1") => {}
        _ => return Err(invalid(field, "cursor was not minted by this build")),
    }
    let offset: usize = parts
        .next()
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| invalid(field, "cursor offset is not a number"))?;
    let check = parts
        .next()
        .ok_or_else(|| invalid(field, "cursor is missing its integrity check"))?;
    if parts.next().is_some() {
        return Err(invalid(field, "cursor has trailing content"));
    }
    let expected = Digest::of_str(&format!("workflow-cursor:{offset}"));
    if check != &expected.as_hex()[..16] {
        return Err(invalid(field, "cursor failed its integrity check"));
    }
    Ok(offset)
}

/// Page a registry deterministically.
///
/// Ordering is lexicographic by id (the frozen canonical ordering), so a cursor
/// stays meaningful across calls.
pub fn page(
    registry: &[WorkflowDescriptor],
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<WorkflowPage, OsControlError> {
    let limit = match limit {
        None => WORKFLOW_PAGE_DEFAULT_ITEMS,
        Some(0) => return Err(invalid("limit", "limit must be at least 1")),
        Some(n) if n > WORKFLOW_PAGE_MAX_ITEMS => {
            return Err(invalid("limit", "limit exceeds the maximum page size"))
        }
        Some(n) => n,
    };
    let offset = match cursor {
        None => 0,
        Some(raw) => decode_cursor(raw)?,
    };

    let mut sorted: Vec<WorkflowDescriptor> = registry.to_vec();
    sorted.sort_by(|a, b| a.id.cmp(b.id));

    if offset > sorted.len() {
        return Err(invalid("cursor", "cursor points past the end of the listing"));
    }
    let end = (offset + limit).min(sorted.len());
    let items = sorted[offset..end].to_vec();
    let truncated = end < sorted.len();
    Ok(WorkflowPage {
        items,
        next_cursor: truncated.then(|| encode_cursor(end)),
        truncated,
    })
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    const FIXTURE_STEPS_A: &[WorkflowStep] = &[WorkflowStep {
        step_id: "disable-backup-timer",
        action: WorkflowStepAction::SetTimerEnabled {
            unit: "kria-backup.timer",
            enabled: false,
        },
    }];

    const FIXTURE_STEPS_B: &[WorkflowStep] = &[
        WorkflowStep {
            step_id: "enable-backup-timer",
            action: WorkflowStepAction::SetTimerEnabled {
                unit: "kria-backup.timer",
                enabled: true,
            },
        },
        WorkflowStep {
            step_id: "enable-index-timer",
            action: WorkflowStepAction::SetTimerEnabled {
                unit: "kria-index.timer",
                enabled: true,
            },
        },
    ];

    const FIXTURE: &[WorkflowDescriptor] = &[
        WorkflowDescriptor {
            id: "fixture.pause-backups",
            revision: 1,
            steps: FIXTURE_STEPS_A,
        },
        WorkflowDescriptor {
            id: "fixture.resume-all",
            revision: 3,
            steps: FIXTURE_STEPS_B,
        },
    ];

    #[test]
    fn the_shipped_registry_is_closed_and_self_consistent() {
        // An empty registry is a fact: no workflow has been reviewed yet.
        for workflow in IN_TREE_WORKFLOWS {
            workflow
                .check()
                .unwrap_or_else(|reason| panic!("in-tree workflow is invalid: {reason}"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for workflow in IN_TREE_WORKFLOWS {
            assert!(ids.insert(workflow.id), "duplicate workflow id");
        }
    }

    #[test]
    fn an_unknown_id_can_never_resolve_to_a_workflow() {
        let id = WorkflowId::parse("fixture.does-not-exist").expect("valid id");
        assert!(descriptor(FIXTURE, &id).is_none());
        // And nothing resolves against the shipped, empty registry.
        let real = WorkflowId::parse("fixture.pause-backups").expect("valid id");
        assert!(descriptor(IN_TREE_WORKFLOWS, &real).is_none());
    }

    #[test]
    fn a_workflow_is_addressed_by_id_not_by_step_content() {
        let id = WorkflowId::parse("fixture.pause-backups").expect("valid id");
        let found = descriptor(FIXTURE, &id).expect("present");
        assert_eq!(found.revision, 1);
        assert_eq!(found.max_step_risk(), RiskLevel::Yellow);
        assert!(found.fully_reversible());
    }

    #[test]
    fn paging_is_deterministic_and_bounded() {
        let first = page(FIXTURE, None, Some(1)).expect("page");
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].id, "fixture.pause-backups");
        assert!(first.truncated);

        let cursor = first.next_cursor.clone().expect("cursor when truncated");
        let second = page(FIXTURE, Some(&cursor), Some(1)).expect("page");
        assert_eq!(second.items[0].id, "fixture.resume-all");
        assert!(!second.truncated);
        assert!(second.next_cursor.is_none());

        assert!(page(FIXTURE, None, Some(0)).is_err());
        assert!(page(FIXTURE, None, Some(WORKFLOW_PAGE_MAX_ITEMS + 1)).is_err());
    }

    #[test]
    fn a_tampered_cursor_is_rejected_rather_than_reinterpreted() {
        let valid = encode_cursor(1);
        assert_eq!(decode_cursor(&valid).expect("round trip"), 1);
        assert!(decode_cursor("w1.9.0000000000000000").is_err());
        assert!(decode_cursor("garbage").is_err());
        assert!(decode_cursor(&format!("{valid}.extra")).is_err());
        assert!(page(FIXTURE, Some("w1.9.0000000000000000"), None).is_err());
    }

    #[test]
    fn an_empty_registry_pages_as_an_empty_untruncated_page() {
        let empty = page(IN_TREE_WORKFLOWS, None, None).expect("page");
        assert!(empty.items.is_empty());
        assert!(!empty.truncated);
        assert!(empty.next_cursor.is_none());
    }

    #[test]
    fn a_descriptor_digest_binds_revision_and_steps() {
        let a = FIXTURE[0];
        let mut bumped = a;
        bumped.revision = 2;
        assert_ne!(a.digest(), bumped.digest());
    }
}
