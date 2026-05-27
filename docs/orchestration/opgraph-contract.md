# OpGraph Contract

## Purpose

`OpGraph` is KRIA's bounded planning graph for operational workflows. It is a
data contract that can compile into a `GoalTree`. It does not execute actions,
call tools, request approvals, retry work, or mutate runtime state.

Primary implementation areas:
- `crates/kria-core/src/agent/opgraph.rs`
- `crates/kria-core/src/agent/opgraph_compiler.rs`
- `crates/kria-core/src/agent/goal_tree.rs`
- `crates/kria-core/src/agent/stage_executor.rs`

## Lifecycle

Current lifecycle:

```text
intent / workflow facts
  -> OpGraph
  -> validate graph bounds and dependencies
  -> compile to GoalTree
  -> validate GoalTree
  -> StageExecutor executes immutable stages
```

The graph is planning-only. `StageExecutor` executes the compiled `GoalTree`,
not the graph directly.

## Graph Model

| Element | Current contract |
|---|---|
| `OpGraph` | Workflow id, objective, nodes, edges, facts, optional frozen flag |
| `OpNode` | Intent, subgoal, action stage, verification, checkpoint, or recovery boundary |
| `OpEdge` | Dependency relation between nodes |
| `OpNodeMetadata` | Risk, confirmation policy, evidence expectation, verifiability, domain, rollback ownership, retry policy, timeout policy |
| `WorkflowDomain` | Coding, debugging, browser, deployment, filesystem, Jira/DevOps, VM/container, communication, research, recovery, system operations, unknown |
| `ConfirmationPolicy` | None, notice, clarify, confirm |
| `RollbackOwnership` | None, stage, workflow |

Dependency edges used for topological ordering:
- `DependsOn`
- `Requires`
- `Blocks`

Other edge kinds express recovery behavior:
- `Fallback`
- `RetryAfter`
- `RollbackTo`

## Bounds

`OpGraph` validation enforces:
- maximum nodes: `MAX_OPGRAPH_NODES = 24`,
- maximum edges: `MAX_OPGRAPH_EDGES = 64`,
- unique node ids,
- all edge endpoints must exist,
- dependency graph must be acyclic for hard dependency edges.

`GoalTree` validation enforces:
- maximum stages: `MAX_STAGES = 8`,
- maximum actions per stage: `MAX_ACTIONS_PER_STAGE = 6`,
- maximum recovery attempts: `MAX_RECOVERY_ATTEMPTS = 2`,
- maximum workflow duration: `MAX_WORKFLOW_DURATION_SEC = 300`,
- maximum stage duration: `MAX_STAGE_DURATION_SEC = 60`,
- no empty action stages,
- no non-terminal `None` checkpoints.

`StageExecutor` also applies a total action cap (`MAX_TOTAL_ACTIONS = 100`) to
avoid runaway execution.

## Compilation

`GoalTreeOpGraphCompiler` owns graph-to-goal-tree conversion.

Compilation rules:
- validate the graph first,
- prefer explicit `ActionStage` nodes when present,
- otherwise compile GUI intent clauses through the rule-based workflow compiler,
- validate the resulting `GoalTree`,
- return typed errors for invalid graph, missing executable stages, goal-tree
  validation failures, or workflow compiler failures.

The compiler is a translation boundary. It must not execute tools or repair a
failed workflow.

## GoalTree Execution Contract

`GoalTree` is the execution contract consumed by `StageExecutor`.

Key runtime objects:
- `GoalTree`: workflow id, description, stages, completion contract, global
  abort policy, max duration, preconditions.
- `GoalStage`: ordered actions, checkpoint, recovery options, timeout.
- `StageAction`: tool/action name, params, verifier hints, timeout.
- `StageCheckpoint`: window/process/file/output/semantic target checks.
- `CompletionContract`: all stages passed, final verification, or user
  confirmation.

`StageExecutor` runs stages sequentially. It may use:
- `ToolExecutor`,
- `ExecutionVerifier`,
- `ForegroundLeaseManager`,
- workflow continuation runtime,
- session manager,
- transparency events,
- PSDG hooks.

It does not replan, mutate `GoalTree`, call the compiler, or invent new graph
nodes during execution.

## Mutation Rules

- OpGraph construction and validation happen before compilation.
- Runtime execution uses the compiled `GoalTree`.
- If workflow structure must change, create a new graph and compile a new
  `GoalTree`.
- Do not mutate nodes/edges/stages in place to make an execution failure pass.
- Events may report progress or request recovery, but they must not execute
  actions directly.

The `OpGraph` data structure includes a `frozen` field. Current runtime
immutability is enforced primarily by ownership boundaries and by executing the
compiled `GoalTree`; docs and callers should not rely on a separate freeze API
unless one is added.

## Cancellation And Recovery

Cancellation propagates through the executor path and tool cancellation tokens.

Recovery boundaries are explicit:
- graph recovery nodes describe possible rollback/recovery points,
- `GoalTree` stages carry bounded recovery actions,
- `StageExecutor` can retry or pause within configured bounds,
- recovery does not grant authority to reinterpret original intent or bypass
  policy.

All side-effecting recovery remains behind tool execution authority,
Policy/HITL, and verifier boundaries.

## Failure Semantics

Common errors:
- invalid graph shape,
- cyclic dependencies,
- missing executable stages,
- invalid goal-tree stage/action/checkpoint contract,
- stage timeout,
- cancellation,
- verifier failure,
- HITL pause or denial,
- exhausted recovery attempts.

Failure should be reported as a typed runtime result. Do not collapse an
execution failure into a successful graph compilation result.
