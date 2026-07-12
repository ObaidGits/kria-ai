//! Plan-level permission (spec R11). A composed/generated [`SolutionPlan`] is
//! authorized against the **union of its step effects at max risk** — never
//! per-isolated-step — so a benign-looking chain cannot silently escalate
//! (R11.1). One approval covers the whole plan at its scope; monotonic grant
//! coverage means a re-run with a subset of effects reuses the grant and only an
//! effect *widening* re-prompts (R11.2).
//!
//! It reuses the existing [`DefaultPermissionEngine`] + [`GrantStore`] by building
//! a synthetic [`AuthorizeRequest`] for the plan — no second permission system.

use super::SolutionPlan;
use crate::capability::descriptor::{Effects, ResourceClass};
use crate::capability::grants::GrantStore;
use crate::capability::permission::{AuthorizeRequest, PermissionDecision, PermissionEngine};

/// Stable id for a plan (so the same plan shape reuses its grant across runs):
/// a hash of the ordered `provider:capability` step keys.
pub fn plan_key(plan: &SolutionPlan) -> String {
    let joined = plan
        .steps
        .iter()
        .map(|s| format!("{}:{}", s.provider_id, s.capability_id))
        .collect::<Vec<_>>()
        .join("|");
    format!("plan:{}", blake3::hash(joined.as_bytes()).to_hex())
}

/// Authorize a whole plan against its union effects at max risk (R11).
pub fn authorize_plan(
    engine: &dyn PermissionEngine,
    plan: &SolutionPlan,
    grants: &GrantStore,
    session_id: Option<String>,
    workspace_id: Option<String>,
) -> PermissionDecision {
    let effects = Effects {
        classes: plan.plan_effects.clone(),
        reversible: plan.plan_reversibility,
        idempotent: false,
        resource_class: ResourceClass::Medium,
    };
    let req = AuthorizeRequest {
        provider_id: "plan".to_string(),
        capability_id: plan_key(plan),
        effects,
        session_id,
        workspace_id,
    };
    engine.authorize(&req, grants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility};
    use crate::capability::intelligence::{planner::DefaultCapabilityPlanner, GoalClass};
    use crate::capability::permission::DefaultPermissionEngine;

    fn cap(id: &str, classes: &[&str], rev: Reversibility) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            "openclaw",
            id,
            id,
            "",
            serde_json::json!({"type": "object"}),
        );
        d.effects = Effects {
            classes: classes.iter().map(|s| s.to_string()).collect(),
            reversible: rev,
            idempotent: false,
            resource_class: Default::default(),
        };
        d
    }

    #[test]
    fn read_only_plan_is_allowed_without_prompt() {
        let planner = DefaultCapabilityPlanner::new();
        let plan = planner
            .compose_linear(
                GoalClass::Information,
                &[(
                    cap("read1", &["read"], Reversibility::Reversible),
                    serde_json::json!({}),
                )],
            )
            .unwrap();
        let grants = GrantStore::in_memory().unwrap();
        let decision = authorize_plan(
            &DefaultPermissionEngine,
            &plan,
            &grants,
            None,
            Some("default".into()),
        );
        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }

    #[test]
    fn plan_with_irreversible_step_prompts() {
        let planner = DefaultCapabilityPlanner::new();
        // A benign read chained with an irreversible write ⇒ union is elevated ⇒ prompt.
        let plan = planner
            .compose_linear(
                GoalClass::Automation,
                &[
                    (
                        cap("read1", &["read"], Reversibility::Reversible),
                        serde_json::json!({}),
                    ),
                    (
                        cap("send", &["network", "write"], Reversibility::Irreversible),
                        serde_json::json!({}),
                    ),
                ],
            )
            .unwrap();
        // Union effects include write+network; plan reversibility is Irreversible.
        assert!(plan.plan_effects.contains(&"write".to_string()));
        assert_eq!(plan.plan_reversibility, Reversibility::Irreversible);
        let grants = GrantStore::in_memory().unwrap();
        let decision = authorize_plan(
            &DefaultPermissionEngine,
            &plan,
            &grants,
            None,
            Some("default".into()),
        );
        assert!(
            matches!(decision, PermissionDecision::Prompt { .. }),
            "irreversible plan must prompt, got {decision:?}"
        );
    }
}
