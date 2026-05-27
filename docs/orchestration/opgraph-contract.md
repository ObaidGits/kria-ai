# OpGraph Execution Contract

## Purpose

Define the lifecycle and safety boundaries for OpGraph in the Batch 3 runtime.

## Lifecycle

1. **Decompose** user input into an OpGraph (planning-only).
2. **Validate** bounds (nodes, edges, cycles).
3. **Freeze** the graph; no mutations during execution.
4. **Compile** OpGraph → GoalTree.
5. **Execute** via StageExecutor only.

## Mutation Rules

- No runtime mutation of nodes/edges after compilation.
- Any adjustment requires a new OpGraph and a new GoalTree.

## Cancellation & Rollback

- Cancellation propagates from the GoalTree/StageExecutor path only.
- Rollback boundaries are explicit nodes; no implicit retries or recursive planning.

## Event Boundaries

Events may propose adjustments but must never execute actions directly.
All execution remains behind Policy/HITL and verifier gates.
