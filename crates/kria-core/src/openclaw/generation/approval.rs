//! A9.0.3 Human Approval Layer — high-risk skills need approval BEFORE installation.
//!
//! Generation may complete; installation waits. This layer decides whether a design
//! requires approval and tracks pending/granted decisions. It never blocks generation.

use super::designer::capabilities_requiring_approval;
use super::designer::SkillDesign;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Outcome of an approval check (A9.0.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalRequirement {
    /// No approval needed — safe to auto-install.
    NotRequired,
    /// Approval required, listing the triggering capabilities.
    Required(Vec<String>),
}

impl ApprovalRequirement {
    pub fn is_required(&self) -> bool {
        matches!(self, ApprovalRequirement::Required(_))
    }
}

/// Decision recorded by a human (or enterprise auto-policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approved,
    Rejected,
    Pending,
}

/// The single approval layer (A9.0.3). Tracks per-skill decisions.
#[derive(Clone, Default)]
pub struct ApprovalLayer {
    decisions: Arc<RwLock<HashMap<String, ApprovalDecision>>>,
    /// If true, auto-approve everything (dev/enterprise-trusted mode).
    auto_approve: bool,
}

impl ApprovalLayer {
    pub fn new(auto_approve: bool) -> Self {
        Self {
            decisions: Arc::new(RwLock::new(HashMap::new())),
            auto_approve,
        }
    }

    /// Determine whether a design requires human approval before install.
    pub fn requirement(design: &SkillDesign) -> ApprovalRequirement {
        let caps = capabilities_requiring_approval(&design.capabilities);
        if caps.is_empty() {
            ApprovalRequirement::NotRequired
        } else {
            ApprovalRequirement::Required(caps)
        }
    }

    /// Whether installation may proceed for a design right now.
    pub fn may_install(&self, design: &SkillDesign) -> bool {
        match Self::requirement(design) {
            ApprovalRequirement::NotRequired => true,
            ApprovalRequirement::Required(_) => {
                if self.auto_approve {
                    return true;
                }
                matches!(
                    self.decisions.read().unwrap().get(&design.slug),
                    Some(ApprovalDecision::Approved)
                )
            }
        }
    }

    /// Record a human decision for a skill.
    pub fn record(&self, slug: &str, decision: ApprovalDecision) {
        self.decisions
            .write()
            .unwrap()
            .insert(slug.to_string(), decision);
    }

    pub fn decision(&self, slug: &str) -> ApprovalDecision {
        self.decisions
            .read()
            .unwrap()
            .get(slug)
            .copied()
            .unwrap_or(ApprovalDecision::Pending)
    }
}
