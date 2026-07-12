//! Wave 9 (W9-R1) — the **Capability-Graph IR**: the neutral, typed, hashable
//! intermediate representation a synthesized capability *is*.
//!
//! A synthesized capability is no longer a bag of primitive-name strings — it is
//! a validated graph of **nodes** connected by **typed edges**. A node is either
//! an audited pure [`NodeOp::Primitive`] or a reference to an already-installed
//! [`NodeOp::Capability`] (`provider_id` + `capability_id`). Edges declare that
//! one node's output type feeds another's input type; the graph is rejected at
//! synthesis time if a chain is not type-linkable (reusing the same
//! outputs∩inputs≠∅ rule as [`crate::capability::intelligence::planner`]).
//!
//! Why an IR (and not raw code):
//! - **Pure + verifiable without a model** — the graph validates, hashes, and
//!   golden-tests deterministically; no compiler, no code model, no host code.
//! - **Reuses the existing runtime** — a graph lowers to the planner's
//!   `SolutionPlan` and executes on the HTN runtime; no rival engine.
//! - **Safe by construction** — per-node declared effects union at max risk so a
//!   benign-looking chain cannot silently escalate (spec R11.1).
//! - **Survives the future** — model/provider/version changes touch only the
//!   *proposer*; the IR + validator + executor are stable + content-addressable.
//!
//! This module is pure (no I/O, no model). Capability-node *execution* is done by
//! an injected [`NodeExecutor`] so the IR itself stays free of provider runtime.

use serde::{Deserialize, Serialize};

use super::primitives;

/// Version of the audited primitive vocabulary the IR is validated against.
/// Stamped into a synthesized capability's provenance so reproducibility can be
/// proven (spec R7/R24): re-synthesizing a goal under the same primitive-set
/// version yields the same graph hash.
pub const PRIMITIVE_SET_VERSION: u32 = 1;

/// Version of the Capability-Graph IR schema itself. Bump on any change to the
/// node/edge shape so on-disk graphs can be migrated deterministically.
pub const IR_SCHEMA_VERSION: u32 = 1;

/// What a graph node does. Neutral: a `Capability` node references a capability
/// by its `(provider_id, capability_id)` coordinate — never a provider-native
/// type (Brain/Hands invariant, spec R23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum NodeOp {
    /// An audited pure text primitive (see [`primitives::KNOWN_PRIMITIVES`]).
    Primitive { name: String },
    /// A reference to an already-installed capability (composition/reuse).
    Capability {
        provider_id: String,
        capability_id: String,
    },
    /// A **Tier-3 generated-code** node (BLOCKER 2): `source` is executed in the
    /// hardened Docker sandbox (never on the host). Only reachable behind the
    /// `synthesis_code` flag + a [`NodeExecutor`] that owns a sandbox; a bad/absent
    /// executor makes execution fail closed (never runs unsandboxed).
    Code { language: String, source: String },
}

impl NodeOp {
    /// A stable label for the node (used in ids/rationale).
    pub fn label(&self) -> String {
        match self {
            NodeOp::Primitive { name } => name.clone(),
            NodeOp::Capability {
                provider_id,
                capability_id,
            } => format!("{provider_id}:{capability_id}"),
            NodeOp::Code { language, .. } => format!("code:{language}"),
        }
    }
}

/// One node of a [`CapabilityGraph`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Stable node id, unique within the graph (e.g. `n0`, `n1`).
    pub id: String,
    pub op: NodeOp,
    /// Declared input type names (IO typing, spec R4.4). Primitives are `["text"]`.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Declared output type names. Primitives are `["text"]`.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Declared effect classes of this node (spec R11.1). Pure primitives declare
    /// none (read-only, no side effects); capability nodes carry their descriptor
    /// effects so the graph's effect union is honest.
    #[serde(default)]
    pub effects: Vec<String>,
}

/// A directed edge: `from` node's output feeds `to` node's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
}

/// The Capability-Graph IR. Nodes are stored in a valid topological (execution)
/// order; edges make the data flow explicit (DAG-ready — today's synthesizer
/// emits a linear chain, but the type + validator already support a DAG).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGraph {
    #[serde(default = "default_ir_version")]
    pub ir_version: u32,
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

fn default_ir_version() -> u32 {
    IR_SCHEMA_VERSION
}

impl CapabilityGraph {
    /// Build a linear graph from an ordered list of audited primitives (the
    /// common synthesis case). Every stage is `text → text`; consecutive stages
    /// are linked. Returns `None` if any op is not a known primitive.
    pub fn linear_primitives(ops: &[String]) -> Option<Self> {
        if ops.is_empty() {
            return None;
        }
        let mut nodes = Vec::with_capacity(ops.len());
        let mut edges = Vec::new();
        for (i, op) in ops.iter().enumerate() {
            if !primitives::KNOWN_PRIMITIVES.contains(&op.as_str()) {
                return None;
            }
            nodes.push(GraphNode {
                id: format!("n{i}"),
                op: NodeOp::Primitive { name: op.clone() },
                inputs: vec!["text".into()],
                outputs: vec!["text".into()],
                effects: Vec::new(),
            });
            if i > 0 {
                edges.push(GraphEdge { from: i - 1, to: i });
            }
        }
        Some(Self {
            ir_version: IR_SCHEMA_VERSION,
            nodes,
            edges,
        })
    }

    /// Validate the graph (spec R4.4/R7.2): non-empty, node ids unique, every
    /// primitive node references a known primitive, edge indices in range, and
    /// every edge is **type-linkable** (`from.outputs ∩ to.inputs ≠ ∅`) when both
    /// sides declare IO types (unknown IO is not a proof of incompatibility).
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("empty capability graph".into());
        }
        let mut seen = std::collections::HashSet::new();
        for n in &self.nodes {
            if !seen.insert(&n.id) {
                return Err(format!("duplicate node id '{}'", n.id));
            }
            match &n.op {
                NodeOp::Primitive { name } => {
                    if !primitives::KNOWN_PRIMITIVES.contains(&name.as_str()) {
                        return Err(format!("unknown primitive '{name}'"));
                    }
                }
                NodeOp::Code { language, source } => {
                    // Structural validation only (full static analysis + sandbox
                    // execution are the sandbox's job at run time). A code node
                    // must declare a supported language + non-empty source.
                    if language != "python" {
                        return Err(format!("unsupported code language '{language}'"));
                    }
                    if source.trim().is_empty() {
                        return Err("code node has empty source".into());
                    }
                }
                NodeOp::Capability { .. } => {}
            }
        }
        for e in &self.edges {
            let (from, to) = (
                self.nodes
                    .get(e.from)
                    .ok_or_else(|| format!("edge from-index {} out of range", e.from))?,
                self.nodes
                    .get(e.to)
                    .ok_or_else(|| format!("edge to-index {} out of range", e.to))?,
            );
            let both_typed = !from.outputs.is_empty() && !to.inputs.is_empty();
            if both_typed && !io_links(&from.outputs, &to.inputs) {
                return Err(format!(
                    "edge {}→{} is not type-linkable (outputs {:?} ∩ inputs {:?} = ∅)",
                    from.id, to.id, from.outputs, to.inputs
                ));
            }
        }
        Ok(())
    }

    /// The ordered primitive pipeline IFF the graph is a pure linear chain of
    /// primitives (back-compat with [`primitives::apply_pipeline`] + existing
    /// synthesized capability ids). Returns `None` when any node is a capability
    /// node (a richer graph that must execute via a [`NodeExecutor`]).
    pub fn primitive_pipeline(&self) -> Option<Vec<String>> {
        let mut out = Vec::with_capacity(self.nodes.len());
        for n in &self.nodes {
            match &n.op {
                NodeOp::Primitive { name } => out.push(name.clone()),
                NodeOp::Capability { .. } | NodeOp::Code { .. } => return None,
            }
        }
        Some(out)
    }

    /// True when every node is an audited pure primitive (no capability nodes) —
    /// so the graph is safe to run in-process with no injected executor.
    pub fn is_pure_primitive(&self) -> bool {
        self.nodes
            .iter()
            .all(|n| matches!(n.op, NodeOp::Primitive { .. }))
    }

    /// The union of all node effect classes at max risk (deduped, sorted). This
    /// is the effect set permission is evaluated against for the whole graph, so
    /// a benign chain that includes one escalating capability node cannot slip
    /// through un-permissioned (spec R11.1 — anti trust-laundering).
    pub fn effects_union(&self) -> Vec<String> {
        let mut set: Vec<String> = Vec::new();
        for n in &self.nodes {
            for c in &n.effects {
                if !set.contains(c) {
                    set.push(c.clone());
                }
            }
        }
        set.sort();
        set
    }

    /// The distinct input type names the graph consumes at its boundary (nodes
    /// with no incoming edge). Used to derive the synthesized capability's public
    /// `input_schema`/`io_modality` (multi-input, spec R7 Phase 7).
    pub fn boundary_inputs(&self) -> Vec<String> {
        let has_incoming: std::collections::HashSet<usize> =
            self.edges.iter().map(|e| e.to).collect();
        let mut out: Vec<String> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if !has_incoming.contains(&i) {
                for t in &n.inputs {
                    if !out.contains(t) {
                        out.push(t.clone());
                    }
                }
            }
        }
        if out.is_empty() {
            out.push("text".into());
        }
        out
    }

    /// The distinct output type names the graph produces at its boundary (nodes
    /// with no outgoing edge).
    pub fn boundary_outputs(&self) -> Vec<String> {
        let has_outgoing: std::collections::HashSet<usize> =
            self.edges.iter().map(|e| e.from).collect();
        let mut out: Vec<String> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if !has_outgoing.contains(&i) {
                for t in &n.outputs {
                    if !out.contains(t) {
                        out.push(t.clone());
                    }
                }
            }
        }
        if out.is_empty() {
            out.push("text".into());
        }
        out
    }

    /// A stable content hash of the graph (canonical JSON → blake3 hex). Recorded
    /// in provenance so a synthesized capability is reproducible + auditable
    /// (spec R7/R16/R24). Stable across runs for the same graph.
    pub fn hash(&self) -> String {
        // serde_json object key order is preserved by field order for structs and
        // is deterministic for our types (no maps), so this is canonical enough
        // for a reproducibility fingerprint.
        let json = serde_json::to_string(self).unwrap_or_default();
        blake3::hash(json.as_bytes()).to_hex().to_string()
    }
}

/// Whether `outputs ∩ inputs ≠ ∅` (typed IO chaining, spec R4.4). Mirrors
/// [`crate::capability::intelligence::planner::DefaultCapabilityPlanner::io_links`]
/// but over raw type-name lists (the IR's node-level typing).
pub fn io_links(outputs: &[String], inputs: &[String]) -> bool {
    outputs.iter().any(|o| inputs.iter().any(|i| i == o))
}

/// Runs generated code in a hardened sandbox (BLOCKER 2/3). Neutral seam so the
/// Brain never depends on the Docker/ACL sandbox type; the concrete
/// `acl::code_sandbox::CodeSandbox` implements this, wired behind `synthesis_code`.
#[async_trait::async_trait]
pub trait CodeRunner: Send + Sync {
    async fn run(&self, language: &str, source: &str, input: &str) -> Result<String, String>;
}

/// Executes a single capability node — injected so the pure IR never depends on
/// a provider runtime (the Brain owns composition; the Hands execute).
#[async_trait::async_trait]
pub trait NodeExecutor: Send + Sync {
    /// Run the capability `(provider_id, capability_id)` with `input` text; return
    /// the text output (capability nodes are text→text in this stage).
    async fn run_capability(
        &self,
        provider_id: &str,
        capability_id: &str,
        input: &str,
    ) -> Result<String, String>;

    /// Run a **Tier-3 code node** in a hardened sandbox (BLOCKER 2/3). Default
    /// FAILS CLOSED — an executor that has no sandbox must never run code, so a
    /// code node cannot execute unless a sandbox-backed executor is wired.
    async fn run_code(
        &self,
        _language: &str,
        _source: &str,
        _input: &str,
    ) -> Result<String, String> {
        Err("code execution requires a sandbox-backed executor (synthesis_code)".into())
    }
}

impl CapabilityGraph {
    /// Execute a **pure-primitive** graph in-process (no executor needed). Errors
    /// if the graph contains a capability node — the caller must use
    /// [`Self::execute`] with a [`NodeExecutor`] for those.
    pub fn execute_pure(&self, input: &str) -> Result<String, String> {
        let pipeline = self
            .primitive_pipeline()
            .ok_or_else(|| "graph contains capability nodes; needs a NodeExecutor".to_string())?;
        primitives::apply_pipeline(&pipeline, input)
    }

    /// Execute a linear graph, running primitive nodes in-process and capability
    /// nodes via the injected [`NodeExecutor`]. Nodes run in stored order; each
    /// node's output feeds the next (linear data flow). A DAG executor is a later
    /// stage — today's synthesizer emits linear graphs.
    pub async fn execute(
        &self,
        input: &str,
        executor: &dyn NodeExecutor,
    ) -> Result<String, String> {
        if self.nodes.is_empty() {
            return Err("empty capability graph".into());
        }
        let mut cur = input.to_string();
        for n in &self.nodes {
            cur = match &n.op {
                NodeOp::Primitive { name } => primitives::apply_primitive(name, &cur)?
                    .ok_or_else(|| format!("node '{}' is not a known primitive", name))?,
                NodeOp::Capability {
                    provider_id,
                    capability_id,
                } => {
                    executor
                        .run_capability(provider_id, capability_id, &cur)
                        .await?
                }
                NodeOp::Code { language, source } => {
                    executor.run_code(language, source, &cur).await?
                }
            };
        }
        Ok(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_primitive_graph_builds_validates_and_executes() {
        let g =
            CapabilityGraph::linear_primitives(&["trim".into(), "upper".into(), "reverse".into()])
                .unwrap();
        g.validate().unwrap();
        assert_eq!(
            g.primitive_pipeline().unwrap(),
            vec!["trim", "upper", "reverse"]
        );
        assert!(g.is_pure_primitive());
        assert_eq!(g.execute_pure("  hi  ").unwrap(), "IH");
        // Pure primitives declare no effects.
        assert!(g.effects_union().is_empty());
    }

    #[test]
    fn unknown_primitive_is_rejected() {
        assert!(CapabilityGraph::linear_primitives(&["bogus".into()]).is_none());
        let g = CapabilityGraph {
            ir_version: IR_SCHEMA_VERSION,
            nodes: vec![GraphNode {
                id: "n0".into(),
                op: NodeOp::Primitive {
                    name: "bogus".into(),
                },
                inputs: vec!["text".into()],
                outputs: vec!["text".into()],
                effects: vec![],
            }],
            edges: vec![],
        };
        assert!(g.validate().is_err());
    }

    #[test]
    fn untyped_edge_links_but_mismatched_types_reject() {
        // image → text mismatch is rejected.
        let g = CapabilityGraph {
            ir_version: IR_SCHEMA_VERSION,
            nodes: vec![
                GraphNode {
                    id: "n0".into(),
                    op: NodeOp::Capability {
                        provider_id: "p".into(),
                        capability_id: "shot".into(),
                    },
                    inputs: vec![],
                    outputs: vec!["image".into()],
                    effects: vec!["read".into()],
                },
                GraphNode {
                    id: "n1".into(),
                    op: NodeOp::Primitive {
                        name: "upper".into(),
                    },
                    inputs: vec!["text".into()],
                    outputs: vec!["text".into()],
                    effects: vec![],
                },
            ],
            edges: vec![GraphEdge { from: 0, to: 1 }],
        };
        assert!(g.validate().is_err());
    }

    #[test]
    fn effects_union_widens_with_a_capability_node() {
        let g = CapabilityGraph {
            ir_version: IR_SCHEMA_VERSION,
            nodes: vec![
                GraphNode {
                    id: "n0".into(),
                    op: NodeOp::Primitive {
                        name: "trim".into(),
                    },
                    inputs: vec!["text".into()],
                    outputs: vec!["text".into()],
                    effects: vec![],
                },
                GraphNode {
                    id: "n1".into(),
                    op: NodeOp::Capability {
                        provider_id: "p".into(),
                        capability_id: "post".into(),
                    },
                    inputs: vec!["text".into()],
                    outputs: vec!["text".into()],
                    effects: vec!["network".into()],
                },
            ],
            edges: vec![GraphEdge { from: 0, to: 1 }],
        };
        g.validate().unwrap();
        // The whole graph's effect union includes the escalating node's effect —
        // permission is evaluated against this, never per-isolated-step (R11.1).
        assert_eq!(g.effects_union(), vec!["network".to_string()]);
        assert!(!g.is_pure_primitive());
        assert!(g.primitive_pipeline().is_none());
    }

    #[test]
    fn hash_is_stable_and_content_addressed() {
        let a = CapabilityGraph::linear_primitives(&["reverse".into()]).unwrap();
        let b = CapabilityGraph::linear_primitives(&["reverse".into()]).unwrap();
        let c = CapabilityGraph::linear_primitives(&["upper".into()]).unwrap();
        assert_eq!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn boundary_io_reflects_graph_ends() {
        let g = CapabilityGraph::linear_primitives(&["trim".into(), "upper".into()]).unwrap();
        assert_eq!(g.boundary_inputs(), vec!["text".to_string()]);
        assert_eq!(g.boundary_outputs(), vec!["text".to_string()]);
    }

    #[tokio::test]
    async fn execute_runs_capability_nodes_via_injected_executor() {
        struct Exec;
        #[async_trait::async_trait]
        impl NodeExecutor for Exec {
            async fn run_capability(
                &self,
                _p: &str,
                _c: &str,
                input: &str,
            ) -> Result<String, String> {
                Ok(format!("[{input}]"))
            }
        }
        let g = CapabilityGraph {
            ir_version: IR_SCHEMA_VERSION,
            nodes: vec![
                GraphNode {
                    id: "n0".into(),
                    op: NodeOp::Primitive {
                        name: "upper".into(),
                    },
                    inputs: vec!["text".into()],
                    outputs: vec!["text".into()],
                    effects: vec![],
                },
                GraphNode {
                    id: "n1".into(),
                    op: NodeOp::Capability {
                        provider_id: "p".into(),
                        capability_id: "wrap".into(),
                    },
                    inputs: vec!["text".into()],
                    outputs: vec!["text".into()],
                    effects: vec!["network".into()],
                },
            ],
            edges: vec![GraphEdge { from: 0, to: 1 }],
        };
        assert_eq!(g.execute("hi", &Exec).await.unwrap(), "[HI]");
    }
}
