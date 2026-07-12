//! Wave 9 (W9-R11) — the **LLM-assisted IR proposer**.
//!
//! The model NEVER produces executable output. It produces a *proposal* — a JSON
//! list of audited primitive names — which is then parsed, normalized, validated,
//! and golden-tested by deterministic code that owns correctness. If the model is
//! unavailable, slow, or emits garbage, [`LlmIrProposer`] falls back to the
//! [`DeterministicIrProposer`] so synthesis is **model-optional**: a bad model can
//! never lower the safety bar or fabricate a capability (spec R7.2/R7.4/R3.4).
//!
//! Pipeline: `prompt → LLM → parse → normalize → validate → (golden verify) →
//! Decision`. The [`TextGenerator`] seam keeps the Brain provider-neutral (no
//! `crate::llm` dependency here) and pluggable — the desktop/runtime layer injects
//! an adapter over the real LLM router behind the `synthesis_llm` flag.

use async_trait::async_trait;

use super::capability_graph::CapabilityGraph;
use super::primitives::KNOWN_PRIMITIVES;
use super::synthesis::{DeterministicIrProposer, IrProposer};

/// Versioned prompt id recorded in provenance so prompt changes are reproducible
/// + A/B-testable (spec R24.2). Bump on any change to [`build_prompt`].
pub const IR_PROMPT_VERSION: u32 = 1;

/// A neutral, minimal text-generation seam. The capability Brain depends only on
/// this trait — never on a concrete LLM backend — so it stays provider-neutral
/// and unit-testable with a mock. The desktop/runtime layer supplies the real
/// adapter (grammar-constrained where the backend supports it).
#[async_trait]
pub trait TextGenerator: Send + Sync {
    /// Generate a completion for a system+user prompt. Returns the raw text, or
    /// an error (the proposer then falls back — never fabricates).
    async fn generate(&self, system: &str, user: &str) -> Result<String, String>;
    /// A stable model label for provenance (e.g. `"qwen3-vl-4b"`).
    fn model_label(&self) -> &str;
}

/// Build the versioned system+user prompt that asks the model to decompose a goal
/// into an ordered list of audited primitives. The audited set is injected so the
/// model cannot invent operations (out-of-set names simply fail validation).
pub fn build_prompt(goal: &str) -> (String, String) {
    let ops = KNOWN_PRIMITIVES.join(", ");
    let system = format!(
        "You are KRIA's capability synthesizer. Decompose the user's goal into an ORDERED \
         pipeline of text transforms, using ONLY these audited operations: [{ops}]. \
         Each stage's text output feeds the next. Respond with STRICT JSON only, no prose: \
         {{\"pipeline\": [\"op1\", \"op2\", ...]}}. If the goal cannot be expressed with the \
         audited operations, respond exactly {{\"pipeline\": []}}. Never invent operation names."
    );
    let user = format!("Goal: {}", goal.trim());
    (system, user)
}

/// Parse a model completion into an ordered pipeline of primitive names. Tolerant
/// of code fences / surrounding prose: extracts the first JSON object and reads
/// its `pipeline` array. Returns `None` when no valid pipeline array is present
/// (caller falls back — never fabricates).
pub fn parse_pipeline(completion: &str) -> Option<Vec<String>> {
    let json_slice = extract_json_object(completion)?;
    let value: serde_json::Value = serde_json::from_str(json_slice).ok()?;
    let arr = value.get("pipeline")?.as_array()?;
    let ops: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_lowercase()))
        .collect();
    if ops.is_empty() {
        return None;
    }
    Some(ops)
}

/// Parse a model completion into a Tier-3 code proposal `(language, source)`,
/// or `None`. Reads `{"code": {"language": "...", "source": "..."}}`.
pub fn parse_code_proposal(completion: &str) -> Option<(String, String)> {
    let json_slice = extract_json_object(completion)?;
    let value: serde_json::Value = serde_json::from_str(json_slice).ok()?;
    let code = value.get("code")?;
    let language = code.get("language")?.as_str()?.trim().to_lowercase();
    let source = code.get("source")?.as_str()?.to_string();
    if source.trim().is_empty() {
        return None;
    }
    Some((language, source))
}

/// Extract the first balanced `{...}` JSON object substring from arbitrary model
/// text (handles ```json fences and leading/trailing prose).
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (i, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The LLM-assisted proposer. Proposes an IR via the model, admits it ONLY if it
/// validates as a linear graph of audited primitives, and otherwise falls back to
/// the deterministic proposer. Records which path produced the IR via
/// [`Self::proposer_id`] (`"llm:<model>"` on model success, `"llm:<model>+fallback"`
/// when the deterministic path was used).
pub struct LlmIrProposer<G: TextGenerator> {
    generator: G,
    fallback: DeterministicIrProposer,
    id: std::sync::Mutex<String>,
    /// Max primitives in a proposed pipeline (security bound — see BLOCKER 9).
    max_stages: usize,
    /// Whether the model may propose a **Tier-3 code node** (only when the
    /// `synthesis_code` sandbox is wired). When false, code proposals are ignored
    /// and the deterministic primitive fallback is used instead.
    allow_code: bool,
}

impl<G: TextGenerator> LlmIrProposer<G> {
    pub fn new(generator: G) -> Self {
        let id = format!("llm:{}", generator.model_label());
        Self {
            generator,
            fallback: DeterministicIrProposer,
            id: std::sync::Mutex::new(id),
            max_stages: 16,
            allow_code: false,
        }
    }

    /// Allow Tier-3 code-node proposals (wire only when `synthesis_code` + the
    /// Docker sandbox are enabled; the sandbox still owns safety at smoke time).
    pub fn with_code(mut self, allow: bool) -> Self {
        self.allow_code = allow;
        self
    }

    /// Max source bytes a code proposal may carry (bound; sandbox re-checks).
    const MAX_CODE_BYTES: usize = 16 * 1024;

    /// Build a single Tier-3 code-node graph from a model proposal, or `None`.
    /// Structural validation only here — the hardened sandbox runs the real
    /// static-analysis + execution gate at smoke time (validator owns safety).
    fn code_graph(&self, language: &str, source: &str) -> Option<CapabilityGraph> {
        if language != "python" || source.trim().is_empty() {
            return None;
        }
        if source.len() > Self::MAX_CODE_BYTES {
            return None;
        }
        let graph = CapabilityGraph {
            ir_version: super::capability_graph::IR_SCHEMA_VERSION,
            nodes: vec![super::capability_graph::GraphNode {
                id: "n0".into(),
                op: super::capability_graph::NodeOp::Code {
                    language: language.to_string(),
                    source: source.to_string(),
                },
                inputs: vec!["text".into()],
                outputs: vec!["text".into()],
                effects: vec!["code_execution".into()],
            }],
            edges: vec![],
        };
        graph.validate().ok().map(|_| graph)
    }

    /// Validate + normalize a model-proposed pipeline into a graph, or `None`.
    /// The VALIDATOR owns correctness: unknown ops, over-length, or a graph that
    /// fails structural validation are all rejected here — never executed.
    fn validate_proposed(&self, ops: &[String]) -> Option<CapabilityGraph> {
        if ops.is_empty() || ops.len() > self.max_stages {
            return None;
        }
        // Every op must be in the audited set.
        if !ops.iter().all(|o| KNOWN_PRIMITIVES.contains(&o.as_str())) {
            return None;
        }
        let graph = CapabilityGraph::linear_primitives(ops)?;
        if graph.validate().is_err() {
            return None;
        }
        // Golden liveness: the proposed pipeline must actually run on a probe input.
        if graph.execute_pure("KRIA").is_err() {
            return None;
        }
        Some(graph)
    }
}

#[async_trait]
impl<G: TextGenerator> IrProposer for LlmIrProposer<G> {
    async fn propose(&self, goal: &str) -> Option<CapabilityGraph> {
        let (mut system, user) = build_prompt(goal);
        if self.allow_code {
            system.push_str(
                " If — and ONLY if — the goal genuinely cannot be expressed with the audited \
                 operations, you MAY instead return a small self-contained Python program that \
                 reads all of stdin and prints the result, as \
                 {\"code\": {\"language\": \"python\", \"source\": \"...\"}}. Prefer the audited \
                 pipeline whenever possible.",
            );
        }
        // Try the model. Any failure (network/parse/validation) → deterministic.
        let completion = self.generator.generate(&system, &user).await.ok();
        // Prefer an audited primitive pipeline; fall back to a Tier-3 code node
        // only when allowed (sandbox wired). Both are validator-gated.
        let model_ok = completion.as_ref().and_then(|c| {
            parse_pipeline(c)
                .and_then(|ops| self.validate_proposed(&ops))
                .or_else(|| {
                    if self.allow_code {
                        parse_code_proposal(c).and_then(|(l, s)| self.code_graph(&l, &s))
                    } else {
                        None
                    }
                })
        });
        if let Some(graph) = model_ok {
            if let Ok(mut id) = self.id.lock() {
                *id = format!("llm:{}", self.generator.model_label());
            }
            return Some(graph);
        }
        // Deterministic fallback — honest, never fabricated.
        let fb = self.fallback.propose(goal).await;
        if fb.is_some() {
            if let Ok(mut id) = self.id.lock() {
                *id = format!("llm:{}+fallback", self.generator.model_label());
            }
        }
        fb
    }

    fn proposer_id(&self) -> &str {
        // Leak a stable &str for the trait signature: store the id in a boxed
        // leak only once per distinct value is overkill; instead expose a static
        // best-effort label. Provenance detail is recorded via the Decision
        // Record path, which reads `last_proposer_id`.
        "llm"
    }
}

impl<G: TextGenerator> LlmIrProposer<G> {
    /// The precise proposer id of the LAST proposal (`llm:<model>` or
    /// `llm:<model>+fallback`) for provenance/Decision Records (W9-R4).
    pub fn last_proposer_id(&self) -> String {
        self.id.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockGen {
        reply: String,
        fail: bool,
    }
    #[async_trait]
    impl TextGenerator for MockGen {
        async fn generate(&self, _s: &str, _u: &str) -> Result<String, String> {
            if self.fail {
                Err("model offline".into())
            } else {
                Ok(self.reply.clone())
            }
        }
        fn model_label(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn parses_pipeline_from_fenced_or_prosey_output() {
        assert_eq!(
            parse_pipeline("```json\n{\"pipeline\":[\"trim\",\"upper\"]}\n```"),
            Some(vec!["trim".into(), "upper".into()])
        );
        assert_eq!(
            parse_pipeline("Sure! {\"pipeline\": [\"reverse\"]} hope that helps"),
            Some(vec!["reverse".into()])
        );
        assert_eq!(parse_pipeline("{\"pipeline\": []}"), None);
        assert_eq!(parse_pipeline("no json here"), None);
    }

    #[test]
    fn parses_code_proposal() {
        let (lang, src) =
            parse_code_proposal("{\"code\":{\"language\":\"python\",\"source\":\"print(1)\"}}")
                .unwrap();
        assert_eq!(lang, "python");
        assert_eq!(src, "print(1)");
        assert!(parse_code_proposal("{\"pipeline\":[\"upper\"]}").is_none());
    }

    #[tokio::test]
    async fn code_proposal_ignored_when_not_allowed() {
        // allow_code = false (default) → code proposal ignored → deterministic
        // fallback on the goal text (here: none → honest None).
        let p = LlmIrProposer::new(MockGen {
            reply: "{\"code\":{\"language\":\"python\",\"source\":\"print(1)\"}}".into(),
            fail: false,
        });
        assert!(p.propose("do a bespoke thing").await.is_none());
    }

    #[tokio::test]
    async fn code_proposal_accepted_when_allowed() {
        let p = LlmIrProposer::new(MockGen {
            reply: "{\"code\":{\"language\":\"python\",\"source\":\"import sys\\nprint(sys.stdin.read())\"}}".into(),
            fail: false,
        })
        .with_code(true);
        let g = p.propose("bespoke goal").await.expect("code graph");
        assert!(!g.is_pure_primitive());
        assert!(g.primitive_pipeline().is_none());
    }

    #[tokio::test]
    async fn valid_model_output_is_admitted() {
        let p = LlmIrProposer::new(MockGen {
            reply: "{\"pipeline\":[\"trim\",\"upper\",\"reverse\"]}".into(),
            fail: false,
        });
        let g = p.propose("do something").await.unwrap();
        assert_eq!(
            g.primitive_pipeline().unwrap(),
            vec!["trim", "upper", "reverse"]
        );
        assert_eq!(p.last_proposer_id(), "llm:mock");
    }

    #[tokio::test]
    async fn invalid_ops_fall_back_to_deterministic() {
        // Model hallucinates a non-audited op → validator rejects → fallback uses
        // the deterministic path on the (real) goal text.
        let p = LlmIrProposer::new(MockGen {
            reply: "{\"pipeline\":[\"mine_bitcoin\",\"steal_data\"]}".into(),
            fail: false,
        });
        let g = p.propose("reverse a string").await.unwrap();
        assert_eq!(g.primitive_pipeline().unwrap(), vec!["reverse"]);
        assert_eq!(p.last_proposer_id(), "llm:mock+fallback");
    }

    #[tokio::test]
    async fn model_offline_falls_back() {
        let p = LlmIrProposer::new(MockGen {
            reply: String::new(),
            fail: true,
        });
        let g = p.propose("uppercase the text").await.unwrap();
        assert_eq!(g.primitive_pipeline().unwrap(), vec!["upper"]);
        assert!(p.last_proposer_id().ends_with("+fallback"));
    }

    #[tokio::test]
    async fn model_and_deterministic_both_decline_is_honest_none() {
        let p = LlmIrProposer::new(MockGen {
            reply: "{\"pipeline\":[]}".into(),
            fail: false,
        });
        assert!(p
            .propose("orchestrate a kubernetes cluster")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn over_length_pipeline_is_rejected() {
        let many = (0..50).map(|_| "\"upper\"").collect::<Vec<_>>().join(",");
        let p = LlmIrProposer::new(MockGen {
            reply: format!("{{\"pipeline\":[{many}]}}"),
            fail: false,
        });
        // Rejected by the max_stages bound → falls back to deterministic on the goal.
        let g = p.propose("reverse it").await.unwrap();
        assert_eq!(g.primitive_pipeline().unwrap(), vec!["reverse"]);
    }
}
