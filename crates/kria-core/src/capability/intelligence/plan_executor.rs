//! [`CapabilityPlanExecutor`] — executes a composed [`SolutionPlan`] step-by-step
//! through the provider-neutral [`CapabilityPlatform`] (spec R4.2/R4.3).
//!
//! # Architecture note (verified against the real code)
//!
//! The design's "single execution-graph engine" invariant is about **planning
//! authority** (one arbiter decides which subsystem owns a turn — spec R10) and
//! about NOT duplicating the GUI-automation HTN engine (`agent/htn_executor.rs`,
//! which models `GuiWorkflow`/`SubGoal`/`SafeAbortStep` for *desktop GUI* steps).
//! Runtime inspection confirms the HTN engine is GUI-specific and is NOT a general
//! capability runtime. Capability plans execute Docker/native/MCP capabilities, a
//! different domain; running them through the GUI HTN engine would be a category
//! error. This executor therefore runs a capability plan directly over
//! `platform.execute` — it is NOT a competing planner (the planning authority
//! already chose CPP for the turn), so the anti-proliferation invariant holds.
//!
//! Saga safety (R4.3): steps run in order; on a failure the executor STOPS and
//! returns an honest **partial result** listing completed steps and the compensation
//! slots of any completed steps that declared one — it never fabricates
//! compensation it cannot perform, and never reports a half-done plan as success.

use std::sync::Arc;

use super::SolutionPlan;
use crate::capability::platform::CapabilityPlatform;
use crate::capability::provider::{CapabilityOutcome, CapabilityRequest, RequestContext};

/// Result of executing one step of a plan.
#[derive(Debug, Clone)]
pub struct StepRun {
    pub provider_id: String,
    pub capability_id: String,
    pub ok: bool,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    /// The compensation slot this step declared (for saga rollback/undo).
    pub compensation: Option<String>,
}

/// The outcome of running a whole [`SolutionPlan`].
#[derive(Debug, Clone)]
pub struct PlanRunResult {
    /// True only if every step produced a real value (no decline/error).
    pub ok: bool,
    /// Per-step results, in execution order (may be shorter than the plan on
    /// early failure — an honest partial result, R4.3).
    pub steps: Vec<StepRun>,
    /// Index of the failed step, if any.
    pub failed_at: Option<usize>,
    /// Compensation slots of completed steps that declared one (reverse order),
    /// so the caller/UI can surface/undo — never silently half-done.
    pub pending_compensations: Vec<String>,
    /// Final output value (last successful step's output), if any.
    pub output: Option<serde_json::Value>,
}

/// Executes capability [`SolutionPlan`]s over the neutral platform.
pub struct CapabilityPlanExecutor {
    platform: Arc<CapabilityPlatform>,
}

impl CapabilityPlanExecutor {
    pub fn new(platform: Arc<CapabilityPlatform>) -> Self {
        Self { platform }
    }

    /// Run a plan sequentially. Prior step outputs are threaded to a consuming
    /// step under `args["_upstream"]` (the reasoner sets concrete args; this makes
    /// upstream data available for capabilities that consume it). On the first
    /// failing step the run stops and returns an honest partial result with the
    /// completed steps' compensation slots.
    pub async fn execute_plan(&self, plan: &SolutionPlan) -> PlanRunResult {
        let ctx = RequestContext::new();
        let mut steps: Vec<StepRun> = Vec::with_capacity(plan.steps.len());
        let mut last_output: Option<serde_json::Value> = None;

        for step in &plan.steps {
            // Thread upstream outputs (if this step declared inputs_from).
            let mut args = step.args.clone();
            if !step.inputs_from.is_empty() {
                let upstream: Vec<serde_json::Value> = step
                    .inputs_from
                    .iter()
                    .filter_map(|&i| steps.get(i).and_then(|s| s.output.clone()))
                    .collect();
                if !upstream.is_empty() {
                    if let serde_json::Value::Object(map) = &mut args {
                        map.entry("_upstream")
                            .or_insert_with(|| serde_json::Value::Array(upstream));
                    }
                }
            }

            let req = CapabilityRequest {
                provider_id: step.provider_id.clone(),
                capability_id: step.capability_id.clone(),
                args,
                context: ctx.clone(),
                granted_effects: plan.plan_effects.clone(),
            };

            match self.platform.execute(req).await {
                Ok(CapabilityOutcome::Value(v)) => {
                    last_output = Some(v.clone());
                    steps.push(StepRun {
                        provider_id: step.provider_id.clone(),
                        capability_id: step.capability_id.clone(),
                        ok: true,
                        output: Some(v),
                        error: None,
                        compensation: step.compensation.clone(),
                    });
                }
                Ok(CapabilityOutcome::Stream(_)) => {
                    // Streams are surfaced via the timeline; treat as completed
                    // with no captured value (honest — we don't buffer here).
                    steps.push(StepRun {
                        provider_id: step.provider_id.clone(),
                        capability_id: step.capability_id.clone(),
                        ok: true,
                        output: None,
                        error: None,
                        compensation: step.compensation.clone(),
                    });
                }
                Ok(CapabilityOutcome::Declined { reason }) => {
                    return self.fail(steps, reason);
                }
                Err(e) => {
                    return self.fail(steps, e.to_string());
                }
            }
        }

        PlanRunResult {
            ok: true,
            steps,
            failed_at: None,
            pending_compensations: Vec::new(),
            output: last_output,
        }
    }

    /// Build an honest partial result on step failure: record the failed step and
    /// gather completed steps' compensation slots in reverse order (saga, R4.3).
    fn fail(&self, mut completed: Vec<StepRun>, error: String) -> PlanRunResult {
        let failed_at = completed.len();
        let pending_compensations: Vec<String> = completed
            .iter()
            .rev()
            .filter_map(|s| s.compensation.clone())
            .collect();
        completed.push(StepRun {
            provider_id: String::new(),
            capability_id: String::new(),
            ok: false,
            output: None,
            error: Some(error),
            compensation: None,
        });
        PlanRunResult {
            ok: false,
            steps: completed,
            failed_at: Some(failed_at),
            pending_compensations,
            output: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility};
    use crate::capability::index::{Embedder, InMemoryFederatedIndex};
    use crate::capability::intelligence::{planner::DefaultCapabilityPlanner, GoalClass};
    use crate::capability::registry::ProviderRegistry;
    use crate::capability::CapabilityPlatform;

    struct HashEmbedder;
    impl Embedder for HashEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, crate::capability::error::CapError> {
            let mut v = vec![0.0f32; 32];
            for (i, b) in text.bytes().enumerate() {
                v[i % 32] += b as f32;
            }
            Ok(v)
        }
        fn dim(&self) -> usize {
            32
        }
        fn model_id(&self) -> &str {
            "hash-test"
        }
    }

    fn cap(
        id: &str,
        inputs: &[&str],
        outputs: &[&str],
        rev: Reversibility,
    ) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            "fake",
            id,
            id,
            "",
            serde_json::json!({"type": "object"}),
        );
        d.inputs = inputs.iter().map(|s| s.to_string()).collect();
        d.outputs = outputs.iter().map(|s| s.to_string()).collect();
        d.effects = Effects {
            classes: vec!["read".into()],
            reversible: rev,
            idempotent: true,
            resource_class: Default::default(),
        };
        d
    }

    async fn platform_with(caps: Vec<CapabilityDescriptor>) -> Arc<CapabilityPlatform> {
        let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder)));
        let registry = ProviderRegistry::new(index);
        registry.register(Arc::new(crate::capability::fake::FakeProvider::new(
            "fake", caps,
        )));
        let platform = Arc::new(CapabilityPlatform::new(Arc::new(registry)));
        platform.refresh().await;
        platform
    }

    #[tokio::test]
    async fn runs_all_steps_and_threads_output() {
        let caps = vec![
            cap("a", &[], &["text"], Reversibility::Reversible),
            cap("b", &["text"], &["text"], Reversibility::Reversible),
        ];
        let platform = platform_with(caps.clone()).await;
        let planner = DefaultCapabilityPlanner::new();
        let plan = planner
            .compose_linear(
                GoalClass::Transformation,
                &[
                    (caps[0].clone(), serde_json::json!({})),
                    (caps[1].clone(), serde_json::json!({})),
                ],
            )
            .unwrap();
        let exec = CapabilityPlanExecutor::new(platform);
        let result = exec.execute_plan(&plan).await;
        assert!(result.ok, "plan should succeed: {result:?}");
        assert_eq!(result.steps.len(), 2);
        // Step b received step a's output threaded under _upstream.
        // (FakeProvider echoes args; assert the echo contains _upstream.)
        let out = result.output.unwrap();
        assert_eq!(out["capability"], "b");
        assert!(out["echo"]["_upstream"].is_array());
    }

    #[tokio::test]
    async fn honest_partial_on_step_failure() {
        // Plan references a capability the provider does not expose ⇒ Declined.
        let caps = vec![cap("a", &[], &[], Reversibility::Irreversible)];
        let platform = platform_with(caps.clone()).await;
        let planner = DefaultCapabilityPlanner::new();
        let missing = {
            let mut d = cap("missing", &[], &[], Reversibility::Reversible);
            d.provider_id = "fake".into();
            d
        };
        let plan = planner
            .compose_linear(
                GoalClass::Automation,
                &[
                    (caps[0].clone(), serde_json::json!({})),
                    (missing, serde_json::json!({})),
                ],
            )
            .unwrap();
        let exec = CapabilityPlanExecutor::new(platform);
        let result = exec.execute_plan(&plan).await;
        assert!(!result.ok);
        assert_eq!(result.failed_at, Some(1));
        // Step 0 was irreversible ⇒ its compensation slot is pending (saga).
        assert_eq!(result.pending_compensations.len(), 1);
    }
}
