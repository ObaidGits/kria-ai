//! A7.14 Execution Engine tests. Uses mock executors — the engine must be fully
//! testable without any concrete backend, proving the planner has no backend-specific
//! logic.

use super::*;
use crate::infra::isolation::ToolResult;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A mock executor recording invocations and returning a configurable result.
struct MockExecutor {
    provider_id: String,
    calls: AtomicU64,
    fail_first_n: u64,
}

impl MockExecutor {
    fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            calls: AtomicU64::new(0),
            fail_first_n: 0,
        }
    }
    fn failing(provider_id: impl Into<String>, fail_first_n: u64) -> Self {
        Self {
            provider_id: provider_id.into(),
            calls: AtomicU64::new(0),
            fail_first_n,
        }
    }
}

#[async_trait]
impl Executor for MockExecutor {
    fn provider_id(&self) -> String {
        self.provider_id.clone()
    }
    async fn execute(&self, req: &ExecutionRequest, _ctx: &ExecutionContext) -> ToolResult {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_first_n {
            ToolResult::err("mock forced failure")
        } else {
            ToolResult::ok(serde_json::json!({ "node": req.node_id, "action": req.action_id }))
        }
    }
}

fn registry_with(provider_id: &str) -> ExecutorRegistry {
    let mut r = ExecutorRegistry::new();
    r.register(Arc::new(MockExecutor::new(provider_id)));
    r
}

// ── Registry ──

#[test]
fn registry_registers_and_discovers() {
    let r = registry_with("native");
    assert!(r.has("native"));
    assert!(!r.has("openclaw"));
    assert_eq!(r.available_providers(), vec!["native".to_string()]);
}

// ── Planner ──

#[test]
fn planner_builds_graph_deterministically() {
    let goal = Goal::new("g1", "test")
        .with_step(PlanStep::new("a", "native", "act_a", serde_json::json!({})))
        .with_step(PlanStep::new("b", "native", "act_b", serde_json::json!({})).depends_on("a"));
    let reg = registry_with("native");
    let graph = ExecutionPlanner::plan(&goal, &reg).unwrap();
    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.roots(), vec!["a".to_string()]);
    assert_eq!(graph.dependents("a"), vec!["b".to_string()]);
}

#[test]
fn planner_rejects_unavailable_executor() {
    let goal =
        Goal::new("g1", "test").with_step(PlanStep::new("a", "gui", "act", serde_json::json!({})));
    let reg = registry_with("native");
    assert!(matches!(
        ExecutionPlanner::plan(&goal, &reg),
        Err(PlanError::ExecutorUnavailable { .. })
    ));
}

#[test]
fn planner_rejects_empty_goal() {
    let goal = Goal::new("g", "empty");
    let reg = registry_with("native");
    assert!(matches!(
        ExecutionPlanner::plan(&goal, &reg),
        Err(PlanError::EmptyGoal)
    ));
}

// ── Dependency resolution ──

#[test]
fn detects_cycle() {
    let mut g = ExecutionGraph::new("g", "goal");
    g.add_node(GraphNode::new("a", NodeKind::Barrier).depends_on("b"));
    g.add_node(GraphNode::new("b", NodeKind::Barrier).depends_on("a"));
    let reg = ExecutorRegistry::new();
    let issues = DependencyResolver::validate(&g, &reg);
    assert!(issues
        .iter()
        .any(|i| matches!(i, DependencyIssue::Cycle(_))));
}

#[test]
fn detects_missing_dependency() {
    let mut g = ExecutionGraph::new("g", "goal");
    g.add_node(GraphNode::new("a", NodeKind::Barrier).depends_on("ghost"));
    let reg = ExecutorRegistry::new();
    let issues = DependencyResolver::validate(&g, &reg);
    assert!(issues
        .iter()
        .any(|i| matches!(i, DependencyIssue::MissingDependency { .. })));
}

#[test]
fn topological_order_valid_dag() {
    let mut g = ExecutionGraph::new("g", "goal");
    g.add_node(GraphNode::new("a", NodeKind::Barrier));
    g.add_node(GraphNode::new("b", NodeKind::Barrier).depends_on("a"));
    g.add_node(GraphNode::new("c", NodeKind::Barrier).depends_on("b"));
    let order = DependencyResolver::topological_order(&g).unwrap();
    let pos = |id: &str| order.iter().position(|x| x == id).unwrap();
    assert!(pos("a") < pos("b"));
    assert!(pos("b") < pos("c"));
}

// ── Full engine execution ──

fn engine_with(provider_id: &str) -> ExecutionEngine {
    let mut e = ExecutionEngine::new();
    e.register_executor(Arc::new(MockExecutor::new(provider_id)));
    e
}

#[tokio::test]
async fn engine_executes_linear_goal() {
    let engine = engine_with("native");
    let goal = Goal::new("g", "linear")
        .with_step(PlanStep::new("a", "native", "act_a", serde_json::json!({})))
        .with_step(PlanStep::new("b", "native", "act_b", serde_json::json!({})).depends_on("a"));
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
    assert_eq!(result.completed_nodes.len(), 2);
}

#[tokio::test]
async fn engine_executes_parallel_nodes() {
    let engine = engine_with("native");
    // 10 independent nodes → all in one parallel wave.
    let mut goal = Goal::new("g", "parallel");
    for i in 0..10 {
        goal = goal.with_step(PlanStep::new(
            format!("n{i}"),
            "native",
            "act",
            serde_json::json!({ "i": i }),
        ));
    }
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
    assert_eq!(result.completed_nodes.len(), 10);
}

#[tokio::test]
async fn engine_retries_then_succeeds() {
    // Executor fails first 2 attempts, succeeds on 3rd. Default policy retries twice.
    let mut engine = ExecutionEngine::new();
    engine.register_executor(Arc::new(MockExecutor::failing("native", 2)));
    engine.set_recovery(RecoveryPolicy::retry(3, 1));

    let goal = Goal::new("g", "retry").with_step(PlanStep::new(
        "a",
        "native",
        "act",
        serde_json::json!({}),
    ));
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
    let m = engine.metrics();
    assert!(m.retry_count >= 2);
}

#[tokio::test]
async fn engine_fails_when_retries_exhausted() {
    let mut engine = ExecutionEngine::new();
    engine.register_executor(Arc::new(MockExecutor::failing("native", 100)));
    engine.set_recovery(RecoveryPolicy::retry(1, 1));

    let goal = Goal::new("g", "fail").with_step(PlanStep::new(
        "a",
        "native",
        "act",
        serde_json::json!({}),
    ));
    let result = engine.execute(&goal).await.unwrap();
    assert!(matches!(result.status, ScheduleStatus::Failed(_)));
}

#[tokio::test]
async fn engine_cancellation_stops_execution() {
    let engine = engine_with("native");
    let goal = Goal::new("g", "cancel").with_step(PlanStep::new(
        "a",
        "native",
        "act",
        serde_json::json!({}),
    ));
    let graph = engine.plan(&goal).unwrap();
    let ctx = ExecutionContext::new("g", "corr");
    ctx.cancellation.cancel(); // cancel before running
    let result = engine.execute_graph(&graph, &ctx).await;
    assert_eq!(result.status, ScheduleStatus::Cancelled);
}

#[tokio::test]
async fn engine_stress_100_node_chain() {
    let engine = engine_with("native");
    let mut goal = Goal::new("g", "chain");
    for i in 0..100 {
        let mut step = PlanStep::new(format!("n{i}"), "native", "act", serde_json::json!({}));
        if i > 0 {
            step = step.depends_on(format!("n{}", i - 1));
        }
        goal = goal.with_step(step);
    }
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
    assert_eq!(result.completed_nodes.len(), 100);
}

#[tokio::test]
async fn engine_stress_100_parallel_nodes() {
    let engine = engine_with("native");
    let mut goal = Goal::new("g", "wide");
    for i in 0..100 {
        goal = goal.with_step(PlanStep::new(
            format!("n{i}"),
            "native",
            "act",
            serde_json::json!({ "i": i }),
        ));
    }
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
    assert_eq!(result.completed_nodes.len(), 100);
}

// ── Mixed executors (proves planner has no backend-specific logic) ──

#[tokio::test]
async fn engine_mixed_executors() {
    let mut engine = ExecutionEngine::new();
    engine.register_executor(Arc::new(MockExecutor::new("native")));
    engine.register_executor(Arc::new(MockExecutor::new("openclaw")));
    engine.register_executor(Arc::new(MockExecutor::new("mcp")));

    let goal = Goal::new("g", "mixed")
        .with_step(PlanStep::new("a", "native", "act", serde_json::json!({})))
        .with_step(PlanStep::new("b", "openclaw", "skill", serde_json::json!({})).depends_on("a"))
        .with_step(PlanStep::new("c", "mcp", "tool", serde_json::json!({})).depends_on("b"));
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
    assert_eq!(result.completed_nodes.len(), 3);
    let m = engine.metrics();
    assert_eq!(m.executor_utilization.len(), 3);
}

// ── Optimizer ──

#[test]
fn optimizer_merges_duplicate_skills() {
    let mut g = ExecutionGraph::new("g", "goal");
    let params = serde_json::json!({"x": 1});
    g.add_node(GraphNode::new(
        "a",
        NodeKind::Skill {
            provider_id: "native".to_string(),
            action_id: "same".into(),
            params: params.clone(),
        },
    ));
    g.add_node(GraphNode::new(
        "b",
        NodeKind::Skill {
            provider_id: "native".to_string(),
            action_id: "same".into(),
            params,
        },
    ));
    let report = GraphOptimizer::optimize(&mut g);
    assert_eq!(report.duplicates_merged, 1);
    assert_eq!(g.node_count(), 1);
}

// ── Context ──

#[tokio::test]
async fn context_stores_outputs_and_vars() {
    let ctx = ExecutionContext::new("g", "corr");
    ctx.set_var("k", serde_json::json!("v")).await;
    ctx.set_output("node1", serde_json::json!(42)).await;
    assert_eq!(ctx.get_var("k").await, Some(serde_json::json!("v")));
    assert_eq!(ctx.get_output("node1").await, Some(serde_json::json!(42)));
}

// ── OpenClaw executor adapter (A7.4) via mock SkillRuntime ──

use crate::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};

/// Mock SkillRuntime standing in for the Docker backend, so the OpenClaw executor
/// adapter is testable without live Docker.
struct MockSkillRuntime {
    succeed: bool,
}

#[async_trait]
impl SkillRuntime for MockSkillRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Docker
    }
    async fn execute(&self, spec: LaunchSpec, _ctx: RuntimeContext) -> ToolResult {
        if self.succeed {
            ToolResult::ok(serde_json::json!({ "skill": spec.skill_id }))
        } else {
            ToolResult::err("mock runtime failure")
        }
    }
}

#[tokio::test]
async fn openclaw_executor_runs_through_engine() {
    let runtime: Arc<dyn SkillRuntime> = Arc::new(MockSkillRuntime { succeed: true });
    let executor = OpenClawExecutor::new(runtime);

    let mut engine = ExecutionEngine::new();
    engine.register_executor(Arc::new(executor));

    let goal = Goal::new("g", "openclaw").with_step(PlanStep::new(
        "s1",
        "openclaw",
        "oc_calculator",
        serde_json::json!({ "expression": "2+2" }),
    ));
    let result = engine.execute(&goal).await.unwrap();
    assert_eq!(result.status, ScheduleStatus::Completed);
}

#[tokio::test]
async fn openclaw_executor_reports_health_and_metrics() {
    let runtime: Arc<dyn SkillRuntime> = Arc::new(MockSkillRuntime { succeed: true });
    let executor = OpenClawExecutor::new(runtime);
    assert_eq!(executor.provider_id(), "openclaw");
    assert!(executor.health().await.available);

    let ctx = ExecutionContext::new("g", "c");
    let req = ExecutionRequest {
        node_id: "n".into(),
        action_id: "oc_calculator".into(),
        params: serde_json::json!({}),
        resource_hint: None,
    };
    let r = executor.execute(&req, &ctx).await;
    assert!(r.success);
    let m = executor.metrics().await;
    assert_eq!(m.executions, 1);
    assert_eq!(m.successes, 1);
}
