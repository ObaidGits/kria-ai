# KRIA GUI Execution Architecture

This document is the canonical runtime summary for GUI execution. RFC details remain in `docs/decisions/rfc/007-gui-system-control.md` and `docs/decisions/rfc/008-recursive-intelligence.md`.

## Execution Chain

1. Intent is admitted and classified by `TurnGate`.
2. Intent normalization produces a typed GUI task specification.
3. Environment grounding collects bounded operational facts.
4. GUI planner compiles an immutable workflow/goal tree.
5. Executor runs bounded PRA-style action loops with safety checks.
6. Verifier validates outcome evidence and returns completion/failure.
7. Result synthesis produces conversational summaries + execution metadata for UI/LLM use.

## Safety Constraints

- Kill switch is globally authoritative.
- No direct desktop control bypasses policy gating.
- Action execution is immutable-plan first; adaptive behavior is bounded.
- Dangerous actions require explicit policy/HITL handling.

## Boundedness Constraints

- Grounding facts are scoped, capped, and short-lived.
- Replanning budgets are finite per turn.
- Verifier calls are bounded and non-recursive.
- Runtime must fail closed when confidence or evidence is insufficient.

## Operational Guidance

- Keep GUI planner, executor, and verifier contracts typed and separate.
- Do not reintroduce parallel hidden planners in executors or helper modules.
- Keep GUI behavior aligned with orchestration authority (`runtime-authority.md`).
- Treat GUI tool output as subject to the same synthesis layer as other tool domains.
