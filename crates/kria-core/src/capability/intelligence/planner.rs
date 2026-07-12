//! [`DefaultCapabilityPlanner`] — composes capabilities into a saga-structured
//! [`SolutionPlan`] (spec R4). It does NOT execute; a Solution Plan is handed to
//! the existing HTN execution-graph runtime (spec R4.2) — there is exactly one
//! executor. This module owns only the *composition* logic:
//!
//! - **typed IO chaining** (R4.4): a step consumes a prior step's outputs only
//!   when `prior.outputs ∩ step.inputs ≠ ∅`; an unlinkable chain is rejected at
//!   plan time, not discovered at execution;
//! - **plan-level effects** (R11.1): the union of all step effect classes (at
//!   max risk) so permission is evaluated on the whole plan, never per-isolated
//!   step;
//! - **saga structure** (R4.3): every step carries a compensation slot;
//! - **per-step confidence** (R2.10).

use async_trait::async_trait;

use super::{
    CapabilityPlanner, ExecutionPath, GoalClass, PlanStep, ScoredCandidate, SolutionPlan,
    REASONING_POLICY_VERSION,
};
use crate::capability::descriptor::{CapabilityDescriptor, Effect};
use crate::capability::error::CapError;

/// Default, provider-neutral capability planner.
pub struct DefaultCapabilityPlanner;

impl DefaultCapabilityPlanner {
    pub fn new() -> Self {
        Self
    }

    /// Union of effect classes across a set of descriptors (deduped, sorted).
    /// This is the plan-level effect set permission is evaluated against (R11.1).
    pub fn union_effects(descriptors: &[&CapabilityDescriptor]) -> Vec<Effect> {
        let mut set: Vec<Effect> = Vec::new();
        for d in descriptors {
            for c in &d.effects.classes {
                if !set.contains(c) {
                    set.push(c.clone());
                }
            }
        }
        set.sort();
        set
    }

    /// Whether `producer.outputs ∩ consumer.inputs ≠ ∅` (typed IO chaining, R4.4).
    pub fn io_links(producer: &CapabilityDescriptor, consumer: &CapabilityDescriptor) -> bool {
        producer
            .outputs
            .iter()
            .any(|o| consumer.inputs.iter().any(|i| i == o))
    }

    /// Compose an ordered pipeline of capabilities into a validated linear plan.
    /// Each step (after the first) MUST consume the prior step's output type
    /// (typed IO chaining); otherwise the chain is rejected (R4.4). Returns the
    /// saga-structured [`SolutionPlan`] with a plan-level effect union (R11.1).
    ///
    /// `steps` are `(descriptor, args)` in execution order. The goal→pipeline
    /// decomposition itself is produced by the LLM-backed reasoner and fed here;
    /// this planner is the deterministic, verifiable composition + validation core.
    pub fn compose_linear(
        &self,
        goal_class: GoalClass,
        steps: &[(CapabilityDescriptor, serde_json::Value)],
    ) -> Result<SolutionPlan, CapError> {
        if steps.is_empty() {
            return Err(CapError::Discovery("empty plan".into()));
        }

        let mut plan_steps: Vec<PlanStep> = Vec::with_capacity(steps.len());
        for (idx, (descriptor, args)) in steps.iter().enumerate() {
            let mut inputs_from = Vec::new();
            if idx > 0 {
                let prior = &steps[idx - 1].0;
                // A multi-step plan must be typed-linkable; else reject (R4.4).
                // If EITHER side declares no IO types we allow the link (unknown
                // IO is not a proof of incompatibility — honest, not brittle).
                let both_typed = !prior.outputs.is_empty() && !descriptor.inputs.is_empty();
                if both_typed && !Self::io_links(prior, descriptor) {
                    return Err(CapError::Discovery(format!(
                        "plan step {idx} ('{}') cannot consume the output of '{}' \
                         (outputs {:?} ∩ inputs {:?} = ∅)",
                        descriptor.capability_id,
                        prior.capability_id,
                        prior.outputs,
                        descriptor.inputs
                    )));
                }
                inputs_from.push(idx - 1);
            }
            let irreversible = matches!(
                descriptor.effects.reversible,
                crate::capability::descriptor::Reversibility::Irreversible
            );
            plan_steps.push(PlanStep {
                provider_id: descriptor.provider_id.clone(),
                capability_id: descriptor.capability_id.clone(),
                args: args.clone(),
                inputs_from,
                // Saga default: irreversible steps get an explicit compensation
                // slot the executor must honor; reversible ones need none (R4.3).
                compensation: if irreversible {
                    Some(format!("compensate:{}", descriptor.capability_id))
                } else {
                    None
                },
                timeout_ms: None,
                confidence: 1.0,
            });
        }

        let descriptors: Vec<&CapabilityDescriptor> = steps.iter().map(|(d, _)| d).collect();
        let plan_effects = Self::union_effects(&descriptors);
        // Plan-level reversibility at max risk: irreversible if ANY step is (R11.1).
        use crate::capability::descriptor::Reversibility;
        let plan_reversibility = if descriptors
            .iter()
            .any(|d| matches!(d.effects.reversible, Reversibility::Irreversible))
        {
            Reversibility::Irreversible
        } else if descriptors
            .iter()
            .any(|d| matches!(d.effects.reversible, Reversibility::Unknown))
        {
            Reversibility::Unknown
        } else {
            Reversibility::Reversible
        };
        let path = if plan_steps.len() > 1 {
            ExecutionPath::Compose
        } else {
            ExecutionPath::Reuse
        };

        Ok(SolutionPlan {
            goal_class,
            path,
            steps: plan_steps,
            plan_effects,
            plan_reversibility,
            confidence: 1.0,
            rationale: format!("Composed {}-step plan.", steps.len()),
            budget_used_ms: 0,
            policy_version: REASONING_POLICY_VERSION,
        })
    }
}

impl Default for DefaultCapabilityPlanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityPlanner for DefaultCapabilityPlanner {
    /// Trait entry: compose the best candidate into a single-step plan. Multi-step
    /// composition uses [`Self::compose_linear`] with a reasoner-produced pipeline.
    async fn compose(
        &self,
        _goal: &str,
        class: &GoalClass,
        candidates: &[ScoredCandidate],
    ) -> Result<SolutionPlan, CapError> {
        let best = candidates
            .first()
            .ok_or_else(|| CapError::Discovery("no candidates to compose".into()))?;
        self.compose_linear(
            class.clone(),
            &[(best.descriptor.clone(), serde_json::json!({}))],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility};

    fn cap(
        id: &str,
        inputs: &[&str],
        outputs: &[&str],
        classes: &[&str],
        rev: Reversibility,
    ) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            "openclaw",
            id,
            id,
            "",
            serde_json::json!({"type":"object"}),
        );
        d.inputs = inputs.iter().map(|s| s.to_string()).collect();
        d.outputs = outputs.iter().map(|s| s.to_string()).collect();
        d.effects = Effects {
            classes: classes.iter().map(|s| s.to_string()).collect(),
            reversible: rev,
            idempotent: false,
            resource_class: Default::default(),
        };
        d
    }

    #[test]
    fn linear_chain_links_and_unions_effects() {
        let planner = DefaultCapabilityPlanner::new();
        // screenshot(image) -> ocr(image->text) -> summarize(text->text)
        let steps = vec![
            (
                cap(
                    "screenshot",
                    &[],
                    &["image"],
                    &["read"],
                    Reversibility::Reversible,
                ),
                serde_json::json!({}),
            ),
            (
                cap(
                    "ocr",
                    &["image"],
                    &["text"],
                    &["read"],
                    Reversibility::Reversible,
                ),
                serde_json::json!({}),
            ),
            (
                cap(
                    "summarize",
                    &["text"],
                    &["text"],
                    &["network"],
                    Reversibility::Reversible,
                ),
                serde_json::json!({}),
            ),
        ];
        let plan = planner
            .compose_linear(GoalClass::Transformation, &steps)
            .unwrap();
        assert_eq!(plan.path, ExecutionPath::Compose);
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[1].inputs_from, vec![0]);
        assert_eq!(plan.steps[2].inputs_from, vec![1]);
        // Effect union across all steps.
        assert_eq!(
            plan.plan_effects,
            vec!["network".to_string(), "read".to_string()]
        );
    }

    #[test]
    fn incompatible_chain_is_rejected_at_plan_time() {
        let planner = DefaultCapabilityPlanner::new();
        // ocr outputs text; next step expects image -> no link -> reject.
        let steps = vec![
            (
                cap("ocr", &[], &["text"], &["read"], Reversibility::Reversible),
                serde_json::json!({}),
            ),
            (
                cap(
                    "edit_image",
                    &["image"],
                    &["image"],
                    &["write"],
                    Reversibility::Reversible,
                ),
                serde_json::json!({}),
            ),
        ];
        let err = planner
            .compose_linear(GoalClass::Transformation, &steps)
            .unwrap_err();
        assert!(matches!(err, CapError::Discovery(_)));
    }

    #[test]
    fn irreversible_step_gets_compensation() {
        let planner = DefaultCapabilityPlanner::new();
        let steps = vec![(
            cap(
                "send_email",
                &[],
                &[],
                &["network"],
                Reversibility::Irreversible,
            ),
            serde_json::json!({}),
        )];
        let plan = planner
            .compose_linear(GoalClass::Automation, &steps)
            .unwrap();
        assert!(plan.steps[0].compensation.is_some());
    }

    #[test]
    fn unknown_io_types_do_not_block_linking() {
        let planner = DefaultCapabilityPlanner::new();
        // Neither declares IO types -> link allowed (unknown != incompatible).
        let steps = vec![
            (
                cap("a", &[], &[], &["read"], Reversibility::Reversible),
                serde_json::json!({}),
            ),
            (
                cap("b", &[], &[], &["read"], Reversibility::Reversible),
                serde_json::json!({}),
            ),
        ];
        let plan = planner
            .compose_linear(GoalClass::Automation, &steps)
            .unwrap();
        assert_eq!(plan.steps.len(), 2);
    }
}
