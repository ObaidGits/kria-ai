//! `CapabilityPlanner` — type-directed composition of selected capabilities into
//! the frozen [`execution::ExecutionGraph`](crate::execution::ExecutionGraph)
//! (design §8.6, R3.2/R3.3/R3.4/R12.3, task 10.1).
//!
//! # What this is
//!
//! The planner turns a [`GoalIntent`] + a set of ranked
//! [`CapabilityCandidate`]s into an [`ExecutionGraph`] expressed **entirely** as
//! the frozen graph type: one [`NodeKind::Skill`] node per selected candidate
//! plus frozen [`NodeKind::Merge`] / [`NodeKind::Parallel`] structural nodes for
//! fan-in / fan-out. It introduces **no new plan format** and does **not** touch
//! the `ExecutionEngine` or any executor — the frozen engine executes this graph
//! unchanged (design D5, R3.3).
//!
//! # Type-directed composition (R3.2, R12.3 — no-hardcoding)
//!
//! A composition edge `a → b` (i.e. `b depends_on a`) is added **only** where the
//! intersection of `a`'s open-vocabulary output type tags and `b`'s input type
//! tags is non-empty:
//!
//! ```text
//! edge a → b  ⇔  a.profile.outputs ∩ b.profile.inputs ≠ ∅
//! ```
//!
//! Matching is purely over the I/O **type-tag strings** (open-vocabulary MIME /
//! type ids from [`CapabilityProfile::inputs`]/[`outputs`]). There is **no**
//! branch on skill name, skill id, category, or any per-domain rule anywhere in
//! this module — a never-before-seen capability composes with zero code change
//! (design §7.1 anti-hardcoding, R12.3).
//!
//! # Structural nodes for fan-in / fan-out (R3.4)
//!
//! Composition is generic, never example-specific. When the type-match relation
//! produces a many-to-one or one-to-many shape, frozen structural nodes are
//! inserted:
//!
//! - **fan-in** — a consumer `b` fed by ≥2 producers routes those producers
//!   through a frozen [`NodeKind::Merge`] coordinator (`merge::<b>`); `b` then
//!   depends on the single merge node.
//! - **fan-out** — a producer `a` feeding ≥2 consumers routes its consumers
//!   through a frozen [`NodeKind::Parallel`] coordinator (`parallel::<a>`) that
//!   depends on `a`; each consumer then depends on the coordinator.
//!
//! When a node is both a multi-consumer producer and feeds a multi-producer
//! consumer the coordinators simply chain (`a → parallel::a → merge::b → b`),
//! which stays acyclic. Only the frozen `NodeKind` variants are used — no node
//! kind is invented.
//!
//! # Determinism
//!
//! Skill nodes are emitted sorted by `skill_id`; the type-match relation, the
//! dependency sets, and the structural-node ids are all gathered in `BTree*`
//! collections, so planning the same inputs yields a byte-identical graph.
//!
//! # Honesty
//!
//! If no selected candidate can contribute a skill node (none has both a
//! `skill_ref` and a [`CapabilityProfile`]), the planner returns
//! [`CilError::Plan`] rather than emitting an empty / broken graph.
//!
//! # Bounded plans + validation (task 10.2, R3.1 / R3.5 / R11.5)
//!
//! Before any graph is returned, the planner enforces two **config-driven** caps
//! and one validation gate — it never emits an unbounded or invalid plan:
//!
//! - **Breadth cap** (`planner_max_breadth`): the number of composable skill
//!   nodes (the plan's real fan-out of work; structural coordinators are
//!   plumbing and not counted). If it exceeds the cap the plan is **rejected**
//!   with [`CilError::Plan`] rather than silently truncated — rejecting is the
//!   honest choice (the caller can re-scope), whereas truncation would drop
//!   requested capabilities without saying so.
//! - **Depth cap** (`planner_max_depth`): the longest dependency chain
//!   (composition length, counting the structural coordinators the frozen engine
//!   actually executes). Over the cap ⇒ [`CilError::Plan`].
//! - **Validation** (R3.1): every emitted graph is passed through the frozen
//!   [`DependencyResolver::validate`] (acyclic, no missing dependency, no
//!   deadlock, and every `Skill` node's executor kind registered). Any issue ⇒
//!   [`CilError::Plan`]. The registry used for validation contains exactly the
//!   [`ExecutorKind::OpenClaw`] executor the planner emits, so it mirrors the
//!   planner's own contract; real *runtime* executor availability is re-checked
//!   at the engine boundary (task 10.3).
//!
//! The caps are **data, not code**: they come from [`CilConfig`] via
//! [`DefaultCapabilityPlanner::from_config`]. [`DefaultCapabilityPlanner::new`]
//! defaults to the same bounds as [`CilConfig`] (breadth 8, depth 5) so a plan
//! is always bounded.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;

use super::config::CilConfig;
use super::graph::CapabilityGraph;
use super::index::CapabilityCandidate;
use super::intent::GoalIntent;
use super::CilError;
use crate::execution::{
    DependencyResolver, ExecutionContext, ExecutionGraph, ExecutionRequest, Executor,
    ExecutorRegistry, GraphNode, NodeKind,
};
use crate::infra::isolation::ToolResult;

/// Turns a goal + selected capabilities into a capability graph, expressed
/// entirely as the frozen [`execution::ExecutionGraph`](crate::execution::ExecutionGraph)
/// (`Skill` nodes + `Barrier`/`Merge`/`Wait`/`Parallel` structural nodes). The
/// frozen `ExecutionEngine` executes it unchanged (design §8.6, D5).
///
/// # `graph_view` parameter (task 12.3 — graph-backed satisfiability)
///
/// Design §8.6 types this as `graph_view: &CapabilityGraph`. Constructing a
/// [`CapabilityGraph`] requires a live `skills.db` connection, which is an
/// awkward dependency for the pure composition performed here (and for its unit
/// tests), so this trait takes `Option<&CapabilityGraph>` instead.
///
/// **Composition itself remains strictly type-directed** — an edge `a → b` is
/// emitted **only** where `a.outputs ∩ b.inputs ≠ ∅` (R3.2 / R12.3), never from
/// a skill name/category and never from a graph edge that lacks I/O overlap. The
/// graph is therefore **never** used to *add* a composition edge (doing so could
/// violate the type-safety invariant); it is consulted **read-only** for a
/// **dependency-satisfiability check** (task 12.3): if two selected skills are
/// related by a hard `depends` edge in the graph, the emitted plan MUST order the
/// dependency before its dependent, else the plan is declined honestly with
/// [`CilError::Plan`]. See [`DefaultCapabilityPlanner`] for the exact rule.
///
/// A `None` `graph_view` (and no builder-wired graph) composes purely from I/O
/// types and runs no graph check — byte-for-byte the pre-12.3 behavior
/// (flag-off / graph-off parity).
pub trait CapabilityPlanner: Send + Sync {
    /// Compose `selected` into a frozen [`ExecutionGraph`] for `intent`.
    fn plan(
        &self,
        intent: &GoalIntent,
        selected: &[CapabilityCandidate],
        graph_view: Option<&CapabilityGraph>,
    ) -> Result<ExecutionGraph, CilError>;
}

/// The default type-directed [`CapabilityPlanner`] (design §8.6).
///
/// Composes selected capabilities by matching each capability's output type tags
/// to the next capability's input type tags, emitting a DAG of
/// [`NodeKind::Skill`] nodes with frozen structural nodes for fan-in/fan-out. It
/// is generic: it never encodes any specific skill or example — it composes
/// whatever capability I/O types connect (R3.4, R12.3).
///
/// The planner also enforces config-driven breadth/depth caps and full
/// [`DependencyResolver::validate`] before returning any graph (task 10.2). The
/// caps are carried as fields so the caps are configurable without changing the
/// [`CapabilityPlanner::plan`] trait signature.
#[derive(Clone)]
pub struct DefaultCapabilityPlanner {
    /// Maximum number of composable skill nodes (plan breadth). A plan whose
    /// skill-node count exceeds this is rejected with [`CilError::Plan`] (R3.5).
    max_breadth: usize,
    /// Maximum dependency-chain length (plan depth). A plan whose longest chain
    /// exceeds this is rejected with [`CilError::Plan`] (R3.5, R11.5).
    max_depth: usize,
    /// **Task 12.3 — optional derived [`CapabilityGraph`] for the
    /// dependency-satisfiability check.** Additive so the task-10.1/10.2
    /// constructors stay stable. When wired (or when a graph is passed to
    /// [`CapabilityPlanner::plan`] directly), the planner consults `depends`
    /// edges to confirm every in-plan hard dependency is ordered before its
    /// dependent; the graph is **never** used to add a composition edge (R3.2 —
    /// composition stays purely I/O-type-directed). `None` → no graph check,
    /// preserving graph-off parity.
    graph: Option<Arc<CapabilityGraph>>,
}

impl Default for DefaultCapabilityPlanner {
    fn default() -> Self {
        Self::new()
    }
}

// Manual `Debug` because [`CapabilityGraph`] holds a live DB connection and is
// not `Debug`; report only whether a graph is wired (not its contents).
impl std::fmt::Debug for DefaultCapabilityPlanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultCapabilityPlanner")
            .field("max_breadth", &self.max_breadth)
            .field("max_depth", &self.max_depth)
            .field("graph_wired", &self.graph.is_some())
            .finish()
    }
}

impl DefaultCapabilityPlanner {
    /// Construct with default caps that mirror [`CilConfig`] defaults
    /// (`max_breadth = 8`, `max_depth = 5`). Permissive enough for ordinary
    /// composition while still guaranteeing a **bounded** plan (R3.5). Prefer
    /// [`from_config`](Self::from_config) in production so the caps track config.
    pub fn new() -> Self {
        Self {
            max_breadth: 8,
            max_depth: 5,
            graph: None,
        }
    }

    /// Construct with explicit breadth/depth caps.
    pub fn with_caps(max_breadth: usize, max_depth: usize) -> Self {
        Self {
            max_breadth,
            max_depth,
            graph: None,
        }
    }

    /// Construct from [`CilConfig`], taking the configured
    /// `planner_max_breadth` / `planner_max_depth` (data, not hardcoded).
    pub fn from_config(config: &CilConfig) -> Self {
        Self::with_caps(config.planner_max_breadth, config.planner_max_depth)
    }

    /// Wire the derived [`CapabilityGraph`] used for the
    /// dependency-satisfiability check (task 12.3, R3.2 / R12.3).
    ///
    /// Additive builder so the task-10.1/10.2 constructors stay stable. The
    /// graph is a rebuildable view over the registry; the planner uses it
    /// **read-only** to confirm in-plan `depends` relationships are ordered, and
    /// **never** to add a composition edge (composition remains strictly
    /// I/O-type-directed — R3.2). A graph passed directly to
    /// [`CapabilityPlanner::plan`] takes precedence over this wired one; with
    /// neither wired nor passed, the planner runs no graph check (graph-off
    /// parity).
    #[must_use]
    pub fn with_capability_graph(mut self, graph: Arc<CapabilityGraph>) -> Self {
        self.graph = Some(graph);
        self
    }
}

/// A no-op [`Executor`] used **only** to populate the [`ExecutorRegistry`] that
/// [`DependencyResolver::validate`] checks executor-registration against.
///
/// The planner emits exclusively [`ExecutorKindTag::OpenClaw`] skill nodes, so a
/// registry that knows the `OpenClaw` kind exactly mirrors the planner's own
/// contract: `validate` would flag a skill node only if the planner ever emitted
/// an executor kind it does not itself produce (i.e. a planner bug). This stub is
/// never executed — `validate` is a pure, side-effect-free graph analysis and
/// never calls [`Executor::execute`]. Whether the real Docker-backed OpenClaw
/// executor is actually registered at run time is re-validated at the engine
/// boundary (task 10.3), where the live [`ExecutorRegistry`] is available.
struct PlannerValidationExecutor;

#[async_trait]
impl Executor for PlannerValidationExecutor {
    fn provider_id(&self) -> String {
        crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID.to_string()
    }

    async fn execute(&self, _req: &ExecutionRequest, _ctx: &ExecutionContext) -> ToolResult {
        // Unreachable in practice: exists solely so DependencyResolver::validate
        // can confirm the OpenClaw executor kind is registered.
        ToolResult::ok(serde_json::json!({}))
    }
}

/// Build the registry `DependencyResolver::validate` checks against. Contains
/// exactly the [`ExecutorKind::OpenClaw`] executor the planner emits.
fn validation_registry() -> ExecutorRegistry {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(PlannerValidationExecutor));
    registry
}

/// Longest dependency-chain length (in nodes) through `graph`. An isolated node
/// has depth 1; `a → b` has depth 2. Structural coordinator nodes count because
/// the frozen engine executes them. Returns `None` if the graph is cyclic (no
/// topological order), which the caller treats as an invalid plan.
fn longest_chain(graph: &ExecutionGraph) -> Option<usize> {
    // Roots-first topological order guarantees a node's dependencies are scored
    // before the node itself, so the DP below is single-pass.
    let order = DependencyResolver::topological_order(graph)?;
    let mut depth: HashMap<String, usize> = HashMap::new();
    for id in &order {
        let node_deps = graph
            .get(id)
            .map(|n| n.dependencies.clone())
            .unwrap_or_default();
        let longest_dep = node_deps
            .iter()
            .filter_map(|dep| depth.get(dep))
            .copied()
            .max()
            .unwrap_or(0);
        depth.insert(id.clone(), longest_dep + 1);
    }
    Some(depth.values().copied().max().unwrap_or(0))
}

/// A skill node that will be emitted: its unique node id (the `skill_id`) and the
/// I/O type tags used for composition. Derived from a candidate's profile.
struct SkillEntry {
    /// The skill id — used both as the node id and the `Skill.action_id`.
    skill_id: String,
    /// Open-vocabulary input type tags (from `CapabilityProfile::inputs`).
    inputs: Vec<String>,
    /// Open-vocabulary output type tags (from `CapabilityProfile::outputs`).
    outputs: Vec<String>,
}

/// `true` iff `outputs ∩ inputs ≠ ∅` — the sole composition predicate. Matching
/// is purely over the open-vocabulary type-tag strings; there is no name or
/// category branch anywhere (R3.2, R12.3).
fn types_connect(outputs: &[String], inputs: &[String]) -> bool {
    let out: BTreeSet<&str> = outputs.iter().map(String::as_str).collect();
    inputs.iter().any(|i| out.contains(i.as_str()))
}

impl CapabilityPlanner for DefaultCapabilityPlanner {
    fn plan(
        &self,
        intent: &GoalIntent,
        selected: &[CapabilityCandidate],
        graph_view: Option<&CapabilityGraph>,
    ) -> Result<ExecutionGraph, CilError> {
        // 1. Gather composable skill entries: a candidate contributes a Skill
        //    node only when it has BOTH a concrete skill_ref and a profile
        //    (the profile carries the I/O type contract). Dedup by skill_id and
        //    sort for determinism.
        let mut by_id: BTreeMap<String, SkillEntry> = BTreeMap::new();
        for cand in selected {
            let (Some(skill_ref), Some(profile)) = (&cand.skill_ref, &cand.profile) else {
                continue;
            };
            by_id
                .entry(skill_ref.clone())
                .or_insert_with(|| SkillEntry {
                    skill_id: skill_ref.clone(),
                    inputs: profile.inputs.clone(),
                    outputs: profile.outputs.clone(),
                });
        }
        if by_id.is_empty() {
            return Err(CilError::Plan(
                "no selected candidate has a skill_ref and capability profile to compose into an execution graph"
                    .to_string(),
            ));
        }
        let entries: Vec<SkillEntry> = by_id.into_values().collect();

        // 2. Type-directed edges: producer → consumer where
        //    producer.outputs ∩ consumer.inputs ≠ ∅. TYPE matching only — never
        //    skill name/category (R3.2, R12.3). A node never composes with
        //    itself.
        let mut producers_of: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut consumers_of: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for producer in &entries {
            for consumer in &entries {
                if producer.skill_id == consumer.skill_id {
                    continue;
                }
                if types_connect(&producer.outputs, &consumer.inputs) {
                    producers_of
                        .entry(consumer.skill_id.as_str())
                        .or_default()
                        .insert(producer.skill_id.as_str());
                    consumers_of
                        .entry(producer.skill_id.as_str())
                        .or_default()
                        .insert(consumer.skill_id.as_str());
                }
            }
        }

        let fan_in = |consumer: &str| {
            producers_of
                .get(consumer)
                .map(|p| p.len() >= 2)
                .unwrap_or(false)
        };
        let fan_out = |producer: &str| {
            consumers_of
                .get(producer)
                .map(|c| c.len() >= 2)
                .unwrap_or(false)
        };
        let merge_id = |consumer: &str| format!("merge::{consumer}");
        let parallel_id = |producer: &str| format!("parallel::{producer}");

        // 3. Build the dependency map (node id → sorted set of dependency ids),
        //    inserting frozen structural coordinators for fan-in / fan-out.
        //    Edge a → b means "b depends_on a" (a completes first).
        let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for e in &entries {
            deps.entry(e.skill_id.clone()).or_default();
        }
        // Structural coordinators discovered while walking edges.
        let mut merges: BTreeSet<String> = BTreeSet::new(); // consumer skill_ids needing a Merge
        let mut parallels: BTreeSet<String> = BTreeSet::new(); // producer skill_ids needing a Parallel

        for (&consumer, producers) in &producers_of {
            let consumer_out_fanin = fan_in(consumer);
            for &producer in producers {
                // Producer side: a fan-out producer feeds a Parallel coordinator.
                let from = if fan_out(producer) {
                    parallels.insert(producer.to_string());
                    parallel_id(producer)
                } else {
                    producer.to_string()
                };
                // Consumer side: a fan-in consumer is fed via a Merge coordinator.
                let to = if consumer_out_fanin {
                    merges.insert(consumer.to_string());
                    merge_id(consumer)
                } else {
                    consumer.to_string()
                };
                deps.entry(to).or_default().insert(from);
            }
        }
        // Wire the structural coordinators to their skill endpoints.
        for producer in &parallels {
            // parallel::<producer> runs after the producer.
            deps.entry(parallel_id(producer))
                .or_default()
                .insert(producer.clone());
        }
        for consumer in &merges {
            // consumer runs after its merge join.
            deps.entry(consumer.clone())
                .or_default()
                .insert(merge_id(consumer));
        }

        // 4. Emit the frozen ExecutionGraph. Skill nodes first (sorted by
        //    skill_id), then structural nodes (sorted by id) — stable order.
        let mut graph = ExecutionGraph::new("cil-plan", intent.raw.clone());
        for e in &entries {
            let node = GraphNode::new(
                e.skill_id.clone(),
                NodeKind::Skill {
                    provider_id: crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID.to_string(),
                    action_id: e.skill_id.clone(),
                    params: serde_json::json!({}),
                },
            )
            .with_label(e.skill_id.clone());
            graph.add_node(with_deps(node, deps.get(&e.skill_id)));
        }
        // Structural nodes, deterministic id order.
        let mut structural: BTreeMap<String, NodeKind> = BTreeMap::new();
        for consumer in &merges {
            structural.insert(merge_id(consumer), NodeKind::Merge);
        }
        for producer in &parallels {
            structural.insert(parallel_id(producer), NodeKind::Parallel);
        }
        for (id, kind) in structural {
            let node = GraphNode::new(id.clone(), kind);
            graph.add_node(with_deps(node, deps.get(&id)));
        }

        // 5. Task 10.2 — bounded plans + validation (R3.1, R3.5, R11.5). These
        //    gates run BEFORE returning: never emit an unbounded or invalid
        //    graph. Reject honestly (CilError::Plan) rather than truncate.

        // Breadth = number of composable skill nodes (real work fan-out).
        // Structural coordinators are plumbing and not counted.
        let breadth = entries.len();
        if breadth > self.max_breadth {
            return Err(CilError::Plan(format!(
                "plan breadth {breadth} exceeds configured planner_max_breadth {} \
                 (reduce the goal or raise [openclaw.cil].planner_max_breadth)",
                self.max_breadth
            )));
        }

        // Depth = longest dependency chain. `None` ⇒ cyclic (invalid plan).
        let depth = longest_chain(&graph).ok_or_else(|| {
            CilError::Plan("composed graph is cyclic (no topological order)".to_string())
        })?;
        if depth > self.max_depth {
            return Err(CilError::Plan(format!(
                "plan depth {depth} exceeds configured planner_max_depth {} \
                 (reduce composition length or raise [openclaw.cil].planner_max_depth)",
                self.max_depth
            )));
        }

        // Belt-and-suspenders: every emitted graph MUST pass the frozen
        // DependencyResolver::validate (acyclic, deps present, no deadlock, every
        // Skill node's executor kind registered) before it is returned (R3.1).
        let issues = DependencyResolver::validate(&graph, &validation_registry());
        if !issues.is_empty() {
            return Err(CilError::Plan(format!(
                "composed graph failed dependency validation: {issues:?}"
            )));
        }

        // 6. Task 12.3 — graph-backed dependency-satisfiability check (R3.2 /
        //    R12.3). Consult the derived CapabilityGraph (either passed to this
        //    call or wired via `with_capability_graph`; the call argument wins)
        //    read-only to confirm every in-plan hard `depends` relationship is
        //    ordered before its dependent. This NEVER adds a composition edge —
        //    composition stays purely I/O-type-directed above — so the plan's
        //    type-safety (Property 7) is untouched; the graph can only cause an
        //    honest decline of a plan whose selected skills carry a hard
        //    dependency the I/O composition does not order. With no graph
        //    available the check is skipped (graph-off parity).
        if let Some(graph_view) = graph_view.or(self.graph.as_deref()) {
            check_dependency_satisfiability(&graph, &entries, graph_view)?;
        }

        Ok(graph)
    }
}

/// **Task 12.3 — dependency-satisfiability check (R3.2 / R12.3).**
///
/// A read-only consultation of the derived [`CapabilityGraph`]'s hard `depends`
/// edges. For every selected (composable) skill `s`, each of its graph
/// dependencies `d` that is **also** in the selected set MUST be ordered before
/// `s` in the emitted plan (i.e. `s` transitively depends on `d`). If it is not,
/// the type-directed I/O composition left a declared hard dependency unordered —
/// the plan would run `d` and `s` without the required ordering — so the plan is
/// **dependency-unsatisfiable** and is declined honestly with [`CilError::Plan`]
/// (never a fake success, never a silently mis-ordered plan).
///
/// # Why this never violates type-directed composition (R3.2)
///
/// The check only ever **rejects** a plan; it does not add or rewrite any edge.
/// Composition edges remain exactly the `a.outputs ∩ b.inputs ≠ ∅` set built
/// above, so every emitted edge is still type-safe. A dependency `d ∉` the
/// selected set is treated as satisfied externally (already installed / resolved
/// by the [`AcquisitionOrchestrator`](crate::openclaw::cil::acquire) — R2.4), so
/// the planner does not fabricate nodes for out-of-plan deps.
///
/// # Honesty on a degraded graph
///
/// The graph is a rebuildable derived view, not a source of truth. A read
/// failure for a given skill is logged and treated as "no additional graph
/// dependencies" for that skill — it never turns a valid type-directed plan into
/// a fake failure (the plan already passed full validation).
fn check_dependency_satisfiability(
    graph: &ExecutionGraph,
    entries: &[SkillEntry],
    graph_view: &CapabilityGraph,
) -> Result<(), CilError> {
    let selected: BTreeSet<&str> = entries.iter().map(|e| e.skill_id.as_str()).collect();
    for e in entries {
        let deps = match graph_view.dependencies_of(&e.skill_id) {
            Ok(deps) => deps,
            Err(err) => {
                tracing::warn!(
                    skill_id = %e.skill_id,
                    error = %err,
                    "[plan] capability-graph dependency lookup failed; skipping graph \
                     satisfiability check for this skill (derived view is degraded, plan \
                     already passed type-directed validation)"
                );
                continue;
            }
        };
        for dep in deps {
            // Only in-plan hard dependencies are the planner's concern; an
            // out-of-plan dep is satisfied externally (installed / acquired).
            if !selected.contains(dep.as_str()) {
                continue;
            }
            if !transitively_depends(graph, &e.skill_id, &dep) {
                return Err(CilError::Plan(format!(
                    "plan is dependency-unsatisfiable: selected skill '{}' declares a hard \
                     dependency on selected skill '{}' (capability-graph `depends` edge) but \
                     their I/O types do not compose, so the plan does not order '{}' before \
                     '{}'; declining rather than emitting a mis-ordered plan",
                    e.skill_id, dep, dep, e.skill_id
                )));
            }
        }
    }
    Ok(())
}

/// `true` iff `from` transitively depends on `to` in the emitted execution
/// graph, following `GraphNode::dependencies` edges (through frozen structural
/// coordinator nodes transparently). A breadth-first walk over the dependency
/// relation; `from == to` is trivially `true`.
fn transitively_depends(graph: &ExecutionGraph, from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(from.to_string());
    seen.insert(from.to_string());
    while let Some(id) = queue.pop_front() {
        let Some(node) = graph.get(&id) else { continue };
        for dep in &node.dependencies {
            if dep == to {
                return true;
            }
            if seen.insert(dep.clone()) {
                queue.push_back(dep.clone());
            }
        }
    }
    false
}

/// Attach the sorted dependency set (if any) to a node. Kept separate so node
/// construction stays declarative and the dependency ordering is deterministic.
fn with_deps(mut node: GraphNode, deps: Option<&BTreeSet<String>>) -> GraphNode {
    if let Some(set) = deps {
        for dep in set {
            node = node.depends_on(dep.clone());
        }
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{DependencyResolver, NodeKind};
    use crate::openclaw::cil::index::{CandidateSource, CapabilityCandidate};
    use crate::openclaw::cil::profile::{CapabilityProfile, CapabilityTag};
    use crate::safety::RiskLevel;

    /// Build a candidate for `skill_id` with the given I/O type tags. `inputs`
    /// and `outputs` are the open-vocabulary type strings the planner composes on.
    fn candidate(skill_id: &str, inputs: &[&str], outputs: &[&str]) -> CapabilityCandidate {
        CapabilityCandidate {
            capability: CapabilityTag::new(format!("cap.{skill_id}")),
            skill_ref: Some(skill_id.to_string()),
            source: CandidateSource::Installed,
            profile: Some(CapabilityProfile {
                skill_id: skill_id.to_string(),
                provides: Vec::new(),
                consumes: Vec::new(),
                permissions: Vec::new(),
                inputs: inputs.iter().map(|s| s.to_string()).collect(),
                outputs: outputs.iter().map(|s| s.to_string()).collect(),
            }),
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }

    fn intent() -> GoalIntent {
        GoalIntent {
            raw: "compose a plan".to_string(),
            goal_embedding: Vec::new(),
            required: Vec::new(),
            composite: true,
            max_risk: RiskLevel::Green,
        }
    }

    /// The dependency ids of a node in the produced graph.
    fn deps_of<'a>(g: &'a ExecutionGraph, id: &str) -> Vec<String> {
        g.get(id)
            .map(|n| n.dependencies.clone())
            .unwrap_or_default()
    }

    /// A single selected candidate produces a 1-node graph with no edges.
    #[test]
    fn single_candidate_is_one_node_no_edges() {
        let planner = DefaultCapabilityPlanner::new();
        let sel = vec![candidate("solo.skill", &["text/plain"], &["image/png"])];
        let g = planner.plan(&intent(), &sel, None).unwrap();
        assert_eq!(g.node_count(), 1);
        let node = g.get("solo.skill").expect("skill node present");
        assert!(node.dependencies.is_empty(), "single node has no deps");
        assert!(matches!(node.kind, NodeKind::Skill { .. }));
    }

    /// Two skills where `a.outputs ∩ b.inputs ≠ ∅` produce a dependency edge
    /// a → b (b depends_on a). (R3.2)
    #[test]
    fn type_overlap_produces_dependency_edge() {
        let planner = DefaultCapabilityPlanner::new();
        let a = candidate("a.producer", &[], &["text/csv"]);
        let b = candidate("b.consumer", &["text/csv"], &["image/png"]);
        let g = planner.plan(&intent(), &[a, b], None).unwrap();
        assert_eq!(g.node_count(), 2);
        assert_eq!(deps_of(&g, "b.consumer"), vec!["a.producer".to_string()]);
        assert!(deps_of(&g, "a.producer").is_empty(), "producer is a root");
        // Emitted as frozen Skill nodes with the OpenClaw executor tag.
        match &g.get("a.producer").unwrap().kind {
            NodeKind::Skill {
                provider_id,
                action_id,
                ..
            } => {
                assert_eq!(
                    provider_id,
                    crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID
                );
                assert_eq!(action_id, "a.producer");
            }
            other => panic!("expected Skill node, got {other:?}"),
        }
    }

    /// Composition is by I/O TYPE, never by skill name: skills with unrelated
    /// names but matching types compose, while name-similar skills with no type
    /// overlap do NOT. (R3.2, R12.3)
    #[test]
    fn composition_is_type_directed_not_name_directed() {
        let planner = DefaultCapabilityPlanner::new();
        // Unrelated names, but out/in types match → edge.
        let x = candidate("alpha.tool", &[], &["data/table"]);
        let y = candidate("zeta.widget", &["data/table"], &[]);
        let g = planner.plan(&intent(), &[x, y], None).unwrap();
        assert_eq!(deps_of(&g, "zeta.widget"), vec!["alpha.tool".to_string()]);

        // Similar-looking names, but NO type overlap → no edge.
        let p = candidate("shared.name.v1", &["in/one"], &["out/one"]);
        let q = candidate("shared.name.v2", &["in/two"], &["out/two"]);
        let g2 = planner.plan(&intent(), &[p, q], None).unwrap();
        assert!(deps_of(&g2, "shared.name.v1").is_empty());
        assert!(deps_of(&g2, "shared.name.v2").is_empty());
    }

    /// Fan-in: when ≥2 producers feed one consumer, a frozen structural Merge
    /// node is inserted and the consumer depends on it. (R3.4)
    #[test]
    fn fan_in_inserts_structural_merge_node() {
        let planner = DefaultCapabilityPlanner::new();
        let p1 = candidate("p.one", &[], &["data/table"]);
        let p2 = candidate("p.two", &[], &["data/table"]);
        let c = candidate("c.sink", &["data/table"], &[]);
        let g = planner.plan(&intent(), &[p1, p2, c], None).unwrap();

        // A Merge structural node exists for the consumer.
        let merge = format!("merge::{}", "c.sink");
        let merge_node = g.get(&merge).expect("merge node inserted for fan-in");
        assert!(matches!(merge_node.kind, NodeKind::Merge));
        // Merge depends on BOTH producers (sorted, deterministic).
        assert_eq!(
            deps_of(&g, &merge),
            vec!["p.one".to_string(), "p.two".to_string()]
        );
        // Consumer depends on the merge, not directly on the producers.
        assert_eq!(deps_of(&g, "c.sink"), vec![merge.clone()]);
        // Graph stays acyclic.
        assert!(DependencyResolver::topological_order(&g).is_some());
    }

    /// No composable candidate (missing skill_ref/profile) is an honest
    /// `CilError::Plan`, not an empty graph.
    #[test]
    fn no_composable_candidate_is_plan_error() {
        let planner = DefaultCapabilityPlanner::new();
        let mut c = candidate("x.skill", &[], &[]);
        c.skill_ref = None; // not a concrete skill
        let err = planner.plan(&intent(), &[c], None).unwrap_err();
        assert!(matches!(err, CilError::Plan(_)));
    }

    /// Planning is deterministic: identical inputs yield an identical node set
    /// and identical dependency ordering.
    #[test]
    fn planning_is_deterministic() {
        let planner = DefaultCapabilityPlanner::new();
        let build = || {
            vec![
                candidate("b.consumer", &["text/csv"], &["image/png"]),
                candidate("a.producer", &[], &["text/csv"]),
            ]
        };
        let g1 = planner.plan(&intent(), &build(), None).unwrap();
        let g2 = planner.plan(&intent(), &build(), None).unwrap();
        assert_eq!(g1.node_ids(), g2.node_ids());
        assert_eq!(deps_of(&g1, "b.consumer"), deps_of(&g2, "b.consumer"));
    }

    // ── Task 10.2: bounded plans + validation (R3.1, R3.5, R11.5) ──

    /// A plan within the configured caps is returned AND passes the frozen
    /// `DependencyResolver::validate` (R3.1, R3.5).
    #[test]
    fn plan_within_caps_validates_and_returns() {
        let planner = DefaultCapabilityPlanner::with_caps(8, 5);
        let a = candidate("a.producer", &[], &["text/csv"]);
        let b = candidate("b.consumer", &["text/csv"], &["image/png"]);
        let g = planner.plan(&intent(), &[a, b], None).unwrap();
        // The emitted graph passes the frozen validator with the OpenClaw kind
        // registered (mirrors what the planner enforces internally).
        let issues = DependencyResolver::validate(&g, &validation_registry());
        assert!(
            issues.is_empty(),
            "within-cap plan must pass DependencyResolver::validate, got {issues:?}"
        );
    }

    /// A plan whose skill-node count exceeds `max_breadth` is rejected with
    /// `CilError::Plan`, never emitted (R3.5). Two skills with disjoint I/O types
    /// stay independent → breadth 2 > cap 1.
    #[test]
    fn breadth_over_cap_is_plan_error() {
        let planner = DefaultCapabilityPlanner::with_caps(1, 5);
        let a = candidate("a.tool", &["in/x"], &["out/x"]);
        let b = candidate("b.tool", &["in/y"], &["out/y"]);
        let err = planner.plan(&intent(), &[a, b], None).unwrap_err();
        assert!(matches!(err, CilError::Plan(_)), "got {err:?}");
    }

    /// A plan whose longest composition chain exceeds `max_depth` is rejected
    /// with `CilError::Plan`, never emitted (R3.5, R11.5). `a → b` has depth 2 >
    /// cap 1, while breadth 2 stays under the generous breadth cap.
    #[test]
    fn depth_over_cap_is_plan_error() {
        let planner = DefaultCapabilityPlanner::with_caps(8, 1);
        let a = candidate("a.producer", &[], &["text/csv"]);
        let b = candidate("b.consumer", &["text/csv"], &[]);
        let err = planner.plan(&intent(), &[a, b], None).unwrap_err();
        assert!(matches!(err, CilError::Plan(_)), "got {err:?}");
    }

    /// Caps are config data, not hardcoded: `from_config` applies the
    /// `CilConfig` breadth/depth caps.
    #[test]
    fn from_config_applies_configured_caps() {
        let mut cfg = crate::openclaw::cil::config::CilConfig::default();
        cfg.planner_max_breadth = 1;
        let planner = DefaultCapabilityPlanner::from_config(&cfg);
        let a = candidate("a.tool", &["in/x"], &["out/x"]);
        let b = candidate("b.tool", &["in/y"], &["out/y"]);
        let err = planner.plan(&intent(), &[a, b], None).unwrap_err();
        assert!(matches!(err, CilError::Plan(_)), "got {err:?}");
    }

    /// A fan-in plan (structural Merge node) built via the default planner both
    /// stays within default caps and passes validation — the emitted graph the
    /// caller receives is always validated.
    #[test]
    fn fan_in_plan_passes_validation_within_default_caps() {
        let planner = DefaultCapabilityPlanner::new();
        let p1 = candidate("p.one", &[], &["data/table"]);
        let p2 = candidate("p.two", &[], &["data/table"]);
        let c = candidate("c.sink", &["data/table"], &[]);
        let g = planner.plan(&intent(), &[p1, p2, c], None).unwrap();
        let issues = DependencyResolver::validate(&g, &validation_registry());
        assert!(
            issues.is_empty(),
            "fan-in plan must validate, got {issues:?}"
        );
    }

    // ── Task 10.4: Property 6 — plan validity (R3.1) ──

    use proptest::prelude::*;

    /// Small open-vocabulary type-tag pool. Drawing inputs/outputs from a shared
    /// pool means edges (`a.outputs ∩ b.inputs ≠ ∅`) sometimes form and sometimes
    /// don't, exercising isolated nodes, chains, fan-in, and fan-out shapes.
    const TYPE_POOL: [&str; 4] = ["t/a", "t/b", "t/c", "t/d"];

    /// Strategy for a small subset of the type pool (0..=4 distinct tags).
    fn tag_set_strategy() -> impl Strategy<Value = Vec<String>> {
        proptest::collection::vec(0usize..TYPE_POOL.len(), 0..=TYPE_POOL.len()).prop_map(|idxs| {
            let mut set: BTreeSet<String> = BTreeSet::new();
            for i in idxs {
                set.insert(TYPE_POOL[i].to_string());
            }
            set.into_iter().collect()
        })
    }

    /// Strategy for 1..=6 candidates, each with a unique `skill_id` and arbitrary
    /// input/output tag sets drawn from the shared pool.
    fn candidates_strategy() -> impl Strategy<Value = Vec<CapabilityCandidate>> {
        proptest::collection::vec((tag_set_strategy(), tag_set_strategy()), 1..=6).prop_map(
            |io_pairs| {
                io_pairs
                    .into_iter()
                    .enumerate()
                    .map(|(i, (inputs, outputs))| {
                        let in_refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
                        let out_refs: Vec<&str> = outputs.iter().map(String::as_str).collect();
                        candidate(&format!("skill.{i}"), &in_refs, &out_refs)
                    })
                    .collect()
            },
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(56))]

        /// **Property 6: Plan validity** — every `ExecutionGraph` emitted by
        /// `CapabilityPlanner` passes the frozen `DependencyResolver::validate`
        /// (empty issue list) and has a topological order (acyclic). An honest
        /// `CilError::Plan` (e.g. no composable candidate) is acceptable — the
        /// property is specifically that every *emitted* graph is valid.
        ///
        /// **Validates: Requirements 3.1**
        #[test]
        fn every_emitted_plan_is_valid(selected in candidates_strategy()) {
            // Generous caps: the property is about VALIDITY of emitted graphs,
            // not size rejection, so the plan is never declined for breadth/depth.
            let planner = DefaultCapabilityPlanner::with_caps(64, 64);
            match planner.plan(&intent(), &selected, None) {
                Ok(graph) => {
                    let issues =
                        DependencyResolver::validate(&graph, &super::validation_registry());
                    prop_assert!(
                        issues.is_empty(),
                        "emitted graph failed DependencyResolver::validate: {issues:?}"
                    );
                    prop_assert!(
                        DependencyResolver::topological_order(&graph).is_some(),
                        "emitted graph must be acyclic (have a topological order)"
                    );
                }
                // Honest plan error (e.g. no composable candidate) is not a
                // property violation — we only assert validity on emitted graphs.
                Err(CilError::Plan(_)) => {}
                Err(other) => prop_assert!(false, "unexpected non-Plan error: {other:?}"),
            }
        }
    }

    // ── Task 10.5: Property 7 — composition type-safety (R3.2) ──

    /// Resolve a single dependency edge `dep → node` in the emitted graph to the
    /// underlying LOGICAL skill→skill composition pair `(producer_skill,
    /// consumer_skill)`, accounting for the frozen structural coordinators the
    /// planner inserts, or `None` if the edge is pure coordinator plumbing (not a
    /// composition relationship).
    ///
    /// The planner emits three kinds of node and wires them so a composition
    /// `producer → consumer` may be split across coordinators:
    ///
    /// - direct `producer_skill → consumer_skill` (no coordinator),
    /// - `producer_skill → parallel::P` and `parallel::P → consumer_skill`
    ///   (fan-out: real producer is `P`),
    /// - `producer_skill → merge::C` and `merge::C → consumer_skill`
    ///   (fan-in: real consumer is `C`),
    /// - both chained: `P → parallel::P → merge::C → C`.
    ///
    /// So we resolve as follows (a `merge::x` dep of a skill, and a `parallel::x`
    /// node's own dep on `x`, are coordinator wiring — not compositions — and
    /// return `None`; the real composition edges live on the merge node's deps
    /// and on consumers depending on a `parallel::x`).
    fn composition_pair(node_id: &str, dep: &str) -> Option<(String, String)> {
        // The real producer skill behind a dependency id (unwrap a `parallel::P`).
        let producer_skill = |d: &str| -> Option<String> {
            if let Some(p) = d.strip_prefix("parallel::") {
                Some(p.to_string())
            } else if d.starts_with("merge::") {
                // A dep that is a merge node is never a producer of a composition.
                None
            } else {
                Some(d.to_string())
            }
        };

        if let Some(consumer) = node_id.strip_prefix("merge::") {
            // merge::C depends on its producers → real consumer is C.
            let producer = producer_skill(dep)?;
            Some((producer, consumer.to_string()))
        } else if node_id.starts_with("parallel::") {
            // parallel::P depends on P → producer→coordinator wiring, not a
            // composition edge.
            None
        } else {
            // node_id is a real skill (the consumer).
            if dep.starts_with("merge::") {
                // skill depends on merge::skill → merge→consumer wiring; the
                // composition edges are the merge node's own deps.
                None
            } else {
                // direct skill dep, or a `parallel::P` fan-out coordinator.
                let producer = producer_skill(dep)?;
                Some((producer, node_id.to_string()))
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(56))]

        /// **Property 7: Composition type-safety** — in every emitted plan, every
        /// logical composition edge `a → b` satisfies `a.outputs ∩ b.inputs ≠ ∅`.
        ///
        /// We inspect EVERY dependency edge in the emitted graph, resolve the
        /// frozen structural coordinators (`merge::<c>` / `parallel::<p>`) back to
        /// the real producer/consumer SKILL pair via [`composition_pair`], and
        /// assert the producer skill's output type tags intersect the consumer
        /// skill's input type tags. Pure coordinator-wiring edges (skill→merge,
        /// parallel→producer) are correctly skipped. This is the sole composition
        /// predicate — type-tag overlap, never skill name/category (R3.2, R12.3).
        ///
        /// An honest `CilError::Plan` (e.g. no composable candidate) is not a
        /// violation — the property is about EMITTED edges.
        ///
        /// **Validates: Requirements 3.2**
        #[test]
        fn every_emitted_composition_edge_is_type_safe(selected in candidates_strategy()) {
            // Generous caps so plans are never declined for size — the property
            // is about type-safety of the edges, not plan bounds.
            let planner = DefaultCapabilityPlanner::with_caps(64, 64);

            // skill_id → (inputs, outputs) from the generated candidates.
            let mut io: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
            for cand in &selected {
                if let (Some(skill_ref), Some(profile)) = (&cand.skill_ref, &cand.profile) {
                    io.insert(
                        skill_ref.clone(),
                        (
                            profile.inputs.iter().cloned().collect(),
                            profile.outputs.iter().cloned().collect(),
                        ),
                    );
                }
            }

            match planner.plan(&intent(), &selected, None) {
                Ok(graph) => {
                    for node_id in graph.node_ids() {
                        let node = graph.get(&node_id).expect("node id resolves");
                        for dep in &node.dependencies {
                            let Some((producer, consumer)) =
                                composition_pair(&node_id, dep)
                            else {
                                continue; // coordinator wiring, not a composition
                            };
                            // Both endpoints must be real generated skills.
                            let (_, producer_out) = io
                                .get(&producer)
                                .expect("resolved producer is a generated skill");
                            let (consumer_in, _) = io
                                .get(&consumer)
                                .expect("resolved consumer is a generated skill");
                            let overlap = producer_out.intersection(consumer_in).next().is_some();
                            prop_assert!(
                                overlap,
                                "composition edge {producer} → {consumer} has no I/O type \
                                 overlap: outputs({producer})={producer_out:?} ∩ \
                                 inputs({consumer})={consumer_in:?} = ∅ \
                                 (via graph edge {dep} → {node_id})"
                            );
                        }
                    }
                }
                // Honest plan error is not a property violation.
                Err(CilError::Plan(_)) => {}
                Err(other) => prop_assert!(false, "unexpected non-Plan error: {other:?}"),
            }
        }
    }

    // ── Task 12.3: graph-backed dependency-satisfiability check (R3.2, R12.3) ──

    use crate::openclaw::cil::graph::CapabilityGraph;
    use crate::openclaw::registry::{
        DiscoverySource, ProductionSkillRegistry, SkillDependency, SkillMetadata, SkillState,
    };
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};

    /// Minimal `SkillMetadata` for a skill that declares `deps` as hard
    /// dependencies — used only to drive `CapabilityGraph` `depends` edge
    /// derivation for the satisfiability tests.
    fn meta_with_deps(skill_id: &str, deps: &[&str]) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: format!("Skill {skill_id}"),
            description: "planner-graph-test skill".to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "media".to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: chrono::Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec![],
            categories: vec![],
            semantic_version: "1.0.0".to_string(),
            dependencies: deps
                .iter()
                .map(|d| SkillDependency {
                    skill_id: (*d).to_string(),
                    version_requirement: "*".to_string(),
                    optional: false,
                })
                .collect(),
            compatibility_requirements: vec![],
            trust_tier: TrustTier::Local,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            state: SkillState::Discovered,
            state_changed_at: chrono::Utc::now(),
        }
    }

    /// Build a `CapabilityGraph` over a fresh temp `skills.db` and rebuild its
    /// edges from `skills` (empty profiles → only `depends`/`supersedes` edges).
    /// Returns the tempdir (kept alive by the caller) and the graph.
    fn graph_with_dependencies(skills: &[SkillMetadata]) -> (tempfile::TempDir, CapabilityGraph) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        // Frozen migrations create capability_edges (migration 6).
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");
        graph.rebuild(skills, &[]).expect("rebuild edges");
        (dir, graph)
    }

    /// When two selected skills are related by a hard `depends` edge AND their
    /// I/O types compose (so the plan already orders the dependency first), the
    /// graph satisfiability check passes and the plan is returned. (R3.2)
    #[test]
    fn graph_dep_satisfied_by_io_ordering_is_ok() {
        // b.consumer depends on a.producer (graph), and their I/O types compose
        // (a.outputs = ["text/csv"] ∩ b.inputs = ["text/csv"]), so the emitted
        // plan orders a → b — the dependency is satisfied.
        let a = candidate("a.producer", &[], &["text/csv"]);
        let b = candidate("b.consumer", &["text/csv"], &["image/png"]);
        let skills = vec![
            meta_with_deps("a.producer", &[]),
            meta_with_deps("b.consumer", &["a.producer"]),
        ];
        let (_dir, graph) = graph_with_dependencies(&skills);
        let planner = DefaultCapabilityPlanner::new();
        let g = planner
            .plan(&intent(), &[a, b], Some(&graph))
            .expect("dependency ordered by I/O composition → satisfiable");
        assert_eq!(deps_of(&g, "b.consumer"), vec!["a.producer".to_string()]);
    }

    /// When two selected skills are related by a hard `depends` edge but their
    /// I/O types do NOT compose, the type-directed plan leaves the dependency
    /// unordered; the graph satisfiability check declines honestly rather than
    /// emitting a mis-ordered plan. Critically, this does NOT add a non-I/O
    /// composition edge (R3.2 preserved). (R3.2, R12.3)
    #[test]
    fn graph_dep_unordered_by_io_is_declined() {
        // b.consumer depends on a.producer (graph), but their I/O types are
        // disjoint (no overlap), so the type-directed planner leaves them
        // unordered → dependency-unsatisfiable → honest decline.
        let a = candidate("a.producer", &["in/x"], &["out/x"]);
        let b = candidate("b.consumer", &["in/y"], &["out/y"]);
        let skills = vec![
            meta_with_deps("a.producer", &[]),
            meta_with_deps("b.consumer", &["a.producer"]),
        ];
        let (_dir, graph) = graph_with_dependencies(&skills);
        let planner = DefaultCapabilityPlanner::new();
        let err = planner.plan(&intent(), &[a, b], Some(&graph)).unwrap_err();
        assert!(matches!(err, CilError::Plan(_)), "got {err:?}");
    }

    /// An out-of-plan hard dependency (the dep skill is not among the selected
    /// candidates) is treated as satisfied externally (installed / acquired by
    /// the AcquisitionOrchestrator — R2.4); the planner does not decline. (R3.2)
    #[test]
    fn graph_dep_outside_selection_is_not_the_planners_concern() {
        // b.consumer depends on ext.base (graph), but ext.base is NOT selected.
        let b = candidate("b.consumer", &["in/y"], &["out/y"]);
        let skills = vec![
            meta_with_deps("ext.base", &[]),
            meta_with_deps("b.consumer", &["ext.base"]),
        ];
        let (_dir, graph) = graph_with_dependencies(&skills);
        let planner = DefaultCapabilityPlanner::new();
        let g = planner
            .plan(&intent(), &[b], Some(&graph))
            .expect("out-of-plan dependency is satisfied externally");
        assert_eq!(g.node_count(), 1);
    }

    /// Graph-off parity: the SAME selection that is declined WITH a conflicting
    /// graph succeeds WITHOUT a graph (`None`), and the wired-graph builder
    /// behaves identically to passing the graph as the call argument. (R3.2)
    #[test]
    fn graph_off_parity_and_builder_equivalence() {
        let a = candidate("a.producer", &["in/x"], &["out/x"]);
        let b = candidate("b.consumer", &["in/y"], &["out/y"]);
        let skills = vec![
            meta_with_deps("a.producer", &[]),
            meta_with_deps("b.consumer", &["a.producer"]),
        ];
        let (_dir, graph) = graph_with_dependencies(&skills);

        // No graph → no check → the type-directed plan is emitted unchanged.
        let planner = DefaultCapabilityPlanner::new();
        let g_none = planner
            .plan(&intent(), &[a.clone(), b.clone()], None)
            .expect("graph-off plan is emitted (parity)");
        assert_eq!(g_none.node_count(), 2);

        // Wired via builder → declines identically to passing the graph arg.
        let wired = DefaultCapabilityPlanner::new().with_capability_graph(Arc::new(graph));
        let err = wired.plan(&intent(), &[a, b], None).unwrap_err();
        assert!(matches!(err, CilError::Plan(_)), "got {err:?}");
    }
}
