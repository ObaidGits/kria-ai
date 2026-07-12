//! Wave 9 — Capability Synthesis specification + gap analysis (neutral).
//!
//! Before any generation, KRIA produces a **deterministic** [`CapabilitySpecification`]
//! from a goal (spec R7.2 scaffold stage): purpose, the audited primitive it maps
//! to, IO schema, declared effects (read-only, lowest trust), and a golden test
//! case. The actual generation + execution is done by the neutral synthesizing
//! `CapabilityProvider` (`acl::synthesis`), which reuses the identical
//! acquire→verify→smoke→benchmark→activate lifecycle (spec R7.1 — no special
//! Brain code).
//!
//! [`CapabilityGapAnalyzer`] classifies whether a goal needs synthesis at all
//! (spec R7.4 / Wave 9.1): native/installed/acquire suffice, synthesize, or
//! honestly decline. Synthesis is the LAST resort, never auto-preferred.

use serde::{Deserialize, Serialize};

use super::capability_graph::CapabilityGraph;
use super::primitives;

/// A deterministic, self-describing specification for a synthesizable capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilitySpecification {
    /// Stable synthesized capability id (derived from the primitive + goal hash).
    pub capability_id: String,
    pub name: String,
    pub purpose: String,
    /// The primary audited primitive (== `pipeline[0]`; kept for id/back-compat).
    pub primitive: String,
    /// The ordered pipeline of audited primitives this capability composes
    /// (length 1 = a single primitive; ≥2 = an engineered composition). The
    /// anti-fake boundary: every stage is an audited primitive, never generated
    /// code. Kept for back-compat + is the primitive projection of [`Self::graph`].
    #[serde(default)]
    pub pipeline: Vec<String>,
    /// The **Capability-Graph IR** (W9-R1/R2): the authoritative typed, hashable
    /// representation this capability *is*. For a pure-primitive composition it
    /// is a linear graph whose `primitive_pipeline()` == [`Self::pipeline`].
    /// `#[serde(default)]` so pre-IR on-disk records still deserialize (their
    /// graph is reconstructed from `pipeline` via [`Self::normalized_graph`]).
    #[serde(default)]
    pub graph: Option<CapabilityGraph>,
    /// Declared family (for portfolio awareness, R17).
    pub family: String,
    /// A golden test case: input → expected output (liveness proxy, R18/R30).
    /// For a multi-input capability this is the JSON-encoded args object.
    pub golden_input: String,
    pub golden_output: String,
    /// **Multi-input reducer** (Wave 9, W9-R9 / BLOCKER 4): when `Some`, this
    /// capability's first stage is an audited multi-input reducer over several
    /// named text inputs ([`Self::input_keys`]), producing the initial text that
    /// then flows through [`Self::pipeline`]. `None` ⇒ single `{text}` input.
    #[serde(default)]
    pub reducer: Option<String>,
    /// The declared named input keys (typed multi-input schema). Empty ⇒ `["text"]`.
    #[serde(default)]
    pub input_keys: Vec<String>,
}

impl CapabilitySpecification {
    /// Deterministically derive a spec from a goal, or `None` when the goal is
    /// not expressible from the audited primitive set (honest-decline, R7.4).
    pub fn from_goal(goal: &str) -> Option<Self> {
        // Multi-input first (W9-R9): a goal like "concatenate two strings" or
        // "merge two json objects" maps to an audited reducer over named inputs.
        if let Some(reducer) = primitives::infer_reducer_from_goal(goal) {
            return Self::from_reducer(goal, reducer);
        }
        // Capability engineering: a goal may compose several audited primitives
        // (e.g. "trim then uppercase then reverse"). Single-op goals yield a
        // pipeline of length 1 (back-compatible id).
        let pipeline: Vec<String> = primitives::infer_pipeline_from_goal(goal)?
            .into_iter()
            .map(String::from)
            .collect();
        let primary = pipeline.first()?.clone();
        // Deterministic golden case computed over the WHOLE pipeline.
        let (golden_in, golden_out) = golden_case(&pipeline)?;
        let hash = blake3::hash(goal.trim().to_lowercase().as_bytes()).to_hex();
        let capability_id = if pipeline.len() == 1 {
            format!("syn_{}_{}", primary, &hash.as_str()[..8])
        } else {
            format!("syn_pipeline_{}", &hash.as_str()[..8])
        };
        let name = if pipeline.len() == 1 {
            format!("Synthesized: {primary}")
        } else {
            format!("Synthesized pipeline: {}", pipeline.join(" → "))
        };
        let family = family_for(&primary);
        // Build the authoritative Capability-Graph IR from the primitive pipeline
        // (linear text→text chain). This is the artifact the capability *is*.
        let graph = CapabilityGraph::linear_primitives(&pipeline);
        Some(Self {
            capability_id,
            name,
            purpose: format!("Synthesized capability for goal: {}", goal.trim()),
            primitive: primary,
            pipeline,
            graph,
            family,
            golden_input: golden_in,
            golden_output: golden_out,
            reducer: None,
            input_keys: Vec::new(),
        })
    }

    /// Build a **multi-input** spec from an audited reducer (W9-R9 / BLOCKER 4):
    /// several named text inputs → one text output. Deterministic golden built
    /// from a canonical set of inputs valid for the reducer.
    fn from_reducer(goal: &str, reducer: &str) -> Option<Self> {
        let keys: Vec<String> = primitives::reducer_inputs(reducer)?
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Canonical golden args per reducer (valid, deterministic).
        let golden_args = match reducer {
            "concat" => serde_json::json!({ "a": "foo", "b": "bar" }),
            "json_merge" => serde_json::json!({ "a": "{\"x\":1}", "b": "{\"y\":2}" }),
            "join_lines" => serde_json::json!({ "items": ["a", "b", "c"], "separator": "," }),
            _ => return None,
        };
        let args_map = golden_args.as_object()?.clone();
        let golden_output = primitives::apply_reducer(reducer, &args_map).ok()??;
        let golden_input = serde_json::to_string(&golden_args).ok()?;
        let hash = blake3::hash(goal.trim().to_lowercase().as_bytes()).to_hex();
        Some(Self {
            capability_id: format!("syn_multi_{}_{}", reducer, &hash.as_str()[..8]),
            name: format!("Synthesized multi-input: {reducer}"),
            purpose: format!(
                "Synthesized multi-input capability for goal: {}",
                goal.trim()
            ),
            primitive: reducer.to_string(),
            pipeline: Vec::new(),
            graph: None, // reducer node executes at the provider boundary (multi-arg)
            family: "Data".to_string(),
            golden_input,
            golden_output,
            reducer: Some(reducer.to_string()),
            input_keys: keys,
        })
    }

    /// Build a spec from a **Brain-proposed** pure-primitive Capability-Graph IR
    /// (W9-R11): the graph is authoritative (it came from the deterministic or
    /// LLM-assisted proposer and already passed `propose_validated`). Returns
    /// `None` if the graph is not a pure-primitive linear chain (capability-node
    /// graphs are executed via the platform, not persisted as a text pipeline
    /// here) or fails validation. Ids/goldens use the identical scheme as
    /// [`Self::from_goal`], so a graph equal to the deterministic one yields the
    /// identical capability id (parity).
    pub fn from_graph(goal: &str, graph: CapabilityGraph) -> Option<Self> {
        graph.validate().ok()?;
        let hash = blake3::hash(goal.trim().to_lowercase().as_bytes()).to_hex();

        // Pure-primitive graph → deterministic in-process pipeline (parity with
        // `from_goal`: same id/golden scheme).
        if let Some(pipeline) = graph.primitive_pipeline() {
            if pipeline.is_empty() {
                return None;
            }
            let primary = pipeline.first()?.clone();
            let (golden_in, golden_out) = golden_case(&pipeline)?;
            let capability_id = if pipeline.len() == 1 {
                format!("syn_{}_{}", primary, &hash.as_str()[..8])
            } else {
                format!("syn_pipeline_{}", &hash.as_str()[..8])
            };
            let name = if pipeline.len() == 1 {
                format!("Synthesized: {primary}")
            } else {
                format!("Synthesized pipeline: {}", pipeline.join(" → "))
            };
            let family = family_for(&primary);
            return Some(Self {
                capability_id,
                name,
                purpose: format!("Synthesized capability for goal: {}", goal.trim()),
                primitive: primary,
                pipeline,
                graph: Some(graph),
                family,
                golden_input: golden_in,
                golden_output: golden_out,
                reducer: None,
                input_keys: Vec::new(),
            });
        }

        // Composed graph (capability and/or Tier-3 code nodes): executes via the
        // platform graph executor / sandbox, not in-process. Persisted with the
        // graph as the authoritative IR (W9-R8 / BLOCKER 2). The golden is a
        // liveness probe (spec R30.2 — smoke proves the node RUNS, not
        // correctness); the sandbox owns safety at execution time.
        let has_code = graph
            .nodes
            .iter()
            .any(|n| matches!(n.op, super::capability_graph::NodeOp::Code { .. }));
        let family = if has_code { "Code" } else { "Composed" }.to_string();
        let kind_tag = if has_code { "code" } else { "graph" };
        Some(Self {
            capability_id: format!("syn_{kind_tag}_{}", &hash.as_str()[..8]),
            name: format!("Synthesized {kind_tag}: {}", goal.trim()),
            purpose: format!(
                "Synthesized {kind_tag} capability for goal: {}",
                goal.trim()
            ),
            primitive: kind_tag.to_string(),
            pipeline: Vec::new(),
            graph: Some(graph),
            family,
            // Liveness probe input; the composed graph produces its own output.
            golden_input: "KRIA".to_string(),
            golden_output: String::new(),
            reducer: None,
            input_keys: Vec::new(),
        })
    }

    /// The authoritative Capability-Graph IR for this spec. Uses the stored
    /// [`Self::graph`] when present (IR-era records), else reconstructs a linear
    /// primitive graph from [`Self::pipeline`] (pre-IR records — migration on
    /// read, spec R22). Falls back to the single `primitive` for the oldest
    /// records that predate `pipeline`.
    pub fn normalized_graph(&self) -> Option<CapabilityGraph> {
        if let Some(g) = &self.graph {
            return Some(g.clone());
        }
        let ops = if !self.pipeline.is_empty() {
            self.pipeline.clone()
        } else {
            vec![self.primitive.clone()]
        };
        CapabilityGraph::linear_primitives(&ops)
    }

    /// Stable content hash of the capability's IR (provenance/reproducibility,
    /// spec R7/R16/R24). `None` only for a record whose graph cannot be rebuilt.
    pub fn ir_hash(&self) -> Option<String> {
        self.normalized_graph().map(|g| g.hash())
    }
}

fn golden_case(pipeline: &[String]) -> Option<(String, String)> {
    // Pick an input valid for the FIRST stage, then compute the expected output
    // deterministically by folding the whole pipeline (always self-consistent).
    let input = match pipeline.first().map(String::as_str) {
        Some("json_pretty") | Some("json_minify") => "{\"a\":1}".to_string(),
        Some("base64_decode") => "aGk=".to_string(),
        _ => "KRIA".to_string(),
    };
    let output = primitives::apply_pipeline(pipeline, &input).ok()?;
    Some((input, output))
}

/// Classify a primitive into a real capability family (spec R17). Derived from
/// the primitive's semantics — measurement ops are Analysis, everything else is
/// a Data transform. Not a single hardcoded value.
fn family_for(primitive: &str) -> String {
    match primitive {
        // Measurement / analysis over the input.
        "length" | "word_count" => "Analysis",
        // Structured-data (de)serialization / encoding / text transforms.
        "json_pretty" | "json_minify" | "base64_encode" | "base64_decode" | "hex_encode"
        | "reverse" | "upper" | "lower" | "trim" => "Data",
        // A new/unknown primitive is uncategorized (open vocabulary), never a
        // silent default.
        _ => "Other",
    }
    .to_string()
}

/// The recommended path for satisfying a goal (Wave 9.1 gap analysis). Mirrors
/// [`super::ExecutionPath`] but is the *gap* classification produced BEFORE a
/// concrete selection, so the Brain decides synthesis vs acquire vs ask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapResolution {
    /// An installed/native capability already suffices — no gap.
    UseExisting,
    /// No local capability; a marketplace install can satisfy it.
    Acquire,
    /// No local + not in any catalog, but expressible from the audited primitive
    /// set — synthesize it.
    Synthesize,
    /// Cannot be satisfied by existing/marketplace/synthesis — decline honestly.
    Decline,
}

/// Classifies the capability gap for a goal (spec R7.4 / Wave 9.1). Neutral: it
/// reasons over booleans the Brain already computes (does a confident local
/// candidate exist? a catalog candidate?), never over provider identity.
#[derive(Debug, Clone, Default)]
pub struct CapabilityGapAnalyzer;

impl CapabilityGapAnalyzer {
    /// `local_sufficient`: a confident installed/native candidate exists.
    /// `catalog_available`: a marketplace candidate exists. Synthesis is the LAST
    /// resort and only when the goal maps to the audited primitive set.
    pub fn classify(
        &self,
        goal: &str,
        local_sufficient: bool,
        catalog_available: bool,
    ) -> GapResolution {
        if local_sufficient {
            return GapResolution::UseExisting;
        }
        if catalog_available {
            return GapResolution::Acquire;
        }
        if CapabilitySpecification::from_goal(goal).is_some() {
            return GapResolution::Synthesize;
        }
        GapResolution::Decline
    }
}

/// Proposes a [`CapabilityGraph`] IR for a goal (W9-R11). This is the seam that
/// makes synthesis **model-optional**: the default [`DeterministicIrProposer`]
/// always works with no model, and a future `LlmIrProposer` (behind its own flag)
/// can propose richer graphs — but a proposal is only ever admitted after it
/// passes [`propose_validated`]'s validator + golden gate, so the *validator*,
/// not the model, decides what becomes a capability (honest-decline over
/// fabrication). A flaky/absent model can never lower the safety bar.
#[async_trait::async_trait]
pub trait IrProposer: Send + Sync {
    /// Propose an IR for the goal, or `None` to honestly decline.
    async fn propose(&self, goal: &str) -> Option<CapabilityGraph>;
    /// A stable label for provenance (`"deterministic"`, `"llm:<model>"`).
    fn proposer_id(&self) -> &str;
}

/// The always-available, no-model proposer: lowers the deterministic
/// keyword→primitive inference into a linear Capability-Graph IR.
#[derive(Debug, Clone, Default)]
pub struct DeterministicIrProposer;

#[async_trait::async_trait]
impl IrProposer for DeterministicIrProposer {
    async fn propose(&self, goal: &str) -> Option<CapabilityGraph> {
        CapabilitySpecification::from_goal(goal).and_then(|s| s.normalized_graph())
    }
    fn proposer_id(&self) -> &str {
        "deterministic"
    }
}

/// Propose an IR and admit it ONLY if it validates (typed edges, known
/// primitives) AND — for a pure-primitive graph — its golden case executes
/// (liveness). Returns the validated graph or `None` (honest-decline). This is
/// the mandatory gate every proposer (deterministic or LLM) passes through, so
/// no unverified artifact is ever produced (spec R7.2/R7.4/R21).
pub async fn propose_validated(proposer: &dyn IrProposer, goal: &str) -> Option<CapabilityGraph> {
    let graph = proposer.propose(goal).await?;
    if graph.validate().is_err() {
        return None;
    }
    // Golden liveness for pure-primitive graphs: it must actually run.
    if graph.is_pure_primitive() {
        let (input, _) = golden_case(&graph.primitive_pipeline()?)?;
        if graph.execute_pure(&input).is_err() {
            return None;
        }
    }
    Some(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_is_deterministic_and_self_consistent() {
        let a = CapabilitySpecification::from_goal("reverse a string").unwrap();
        let b = CapabilitySpecification::from_goal("reverse a string").unwrap();
        assert_eq!(a, b, "spec generation must be deterministic");
        assert_eq!(a.primitive, "reverse");
        // Golden case is self-consistent with the primitive.
        assert_eq!(
            primitives::apply_primitive(&a.primitive, &a.golden_input)
                .unwrap()
                .unwrap(),
            a.golden_output
        );
    }

    #[test]
    fn composed_goal_yields_a_pipeline_spec() {
        let s = CapabilitySpecification::from_goal("trim then uppercase then reverse").unwrap();
        assert_eq!(s.pipeline, vec!["trim", "upper", "reverse"]);
        assert!(s.capability_id.starts_with("syn_pipeline_"));
        // Golden output is the full pipeline applied to the golden input.
        assert_eq!(
            primitives::apply_pipeline(&s.pipeline, &s.golden_input).unwrap(),
            s.golden_output
        );
    }

    #[tokio::test]
    async fn deterministic_proposer_yields_a_validated_ir_or_declines() {
        let p = DeterministicIrProposer;
        let g = propose_validated(&p, "trim then uppercase then reverse")
            .await
            .expect("should propose a validated IR");
        assert_eq!(
            g.primitive_pipeline().unwrap(),
            vec!["trim", "upper", "reverse"]
        );
        // Un-expressible → honest-decline (validator/proposer returns None).
        assert!(propose_validated(&p, "orchestrate a kubernetes cluster")
            .await
            .is_none());
        assert_eq!(p.proposer_id(), "deterministic");
    }

    #[test]
    fn unsynthesizable_goal_declines() {
        assert!(CapabilitySpecification::from_goal("orchestrate a kubernetes cluster").is_none());
        // A composition where any stage is unsynthesizable also declines (whole).
        assert!(CapabilitySpecification::from_goal("uppercase then deploy to prod").is_none());
    }

    #[test]
    fn gap_analyzer_prefers_existing_then_acquire_then_synthesize_then_decline() {
        let a = CapabilityGapAnalyzer;
        assert_eq!(
            a.classify("reverse text", true, true),
            GapResolution::UseExisting
        );
        assert_eq!(
            a.classify("reverse text", false, true),
            GapResolution::Acquire
        );
        assert_eq!(
            a.classify("reverse text", false, false),
            GapResolution::Synthesize
        );
        assert_eq!(
            a.classify("fly to the moon", false, false),
            GapResolution::Decline
        );
    }
}
