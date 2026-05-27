# KRIA Evaluation System

## 1. Purpose

The eval subsystem provides deterministic validation of KRIA runtime behavior across orchestration, tools, safety, providers, and integrations.

Responsibilities:
- Run scenario-based evaluations against current runtime.
- Produce structured pass/fail and scoring artifacts.
- Detect regressions in authority boundaries and execution policy discipline.
- Support CI and local engineering validation workflows.

Non-goals:
- Eval harness is not runtime authority.
- Eval results do not directly mutate production policy without review.

## 2. Architecture Overview

Primary implementation:
- `crates/kria-eval/src/main.rs`
- `crates/kria-eval/src/runner.rs`
- `crates/kria-eval/src/judge.rs`
- `crates/kria-eval/src/sandbox.rs`

Architecture:
1. Scenario inputs are loaded and executed through runner.
2. Runtime outputs are collected and normalized.
3. Judge applies rubric/criteria scoring.
4. Reports are emitted for diagnostics and trend tracking.

## 3. Runtime Execution Flow

1. Eval mode initializes run context and scenario set.
2. Each scenario drives KRIA through controlled prompts/actions.
3. Outputs and side-effect indicators are captured.
4. Judge evaluates correctness, safety, and policy adherence.
5. Aggregated result determines overall run status.

Authority boundaries:
- Eval invokes runtime pathways but does not bypass safety architecture.
- Eval mode can alter approval behavior explicitly for testing contexts.

## 4. Core Components

| Component | Location | Contract |
|---|---|---|
| CLI entry | `kria-eval/src/main.rs` | Run configuration and mode selection |
| Runner | `kria-eval/src/runner.rs` | Scenario execution orchestration |
| Judge | `kria-eval/src/judge.rs` | Scoring and verdict logic |
| Sandbox utilities | `kria-eval/src/sandbox.rs` | Isolated test environment handling |

Invariants:
- Scenario execution is repeatable under controlled inputs.
- Scoring criteria are explicit and versionable.
- Failures surface actionable diagnostics.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | Validate authority flow and bounded loop behavior |
| Providers | Measure routing/fallback behavior and output quality under constraints |
| Tools | Validate tool invocation correctness, failure handling, and policy compliance |
| Memory | Check recall/retention and context quality impacts |
| OpenClaw/n8n/MCP | Exercise substrate routing and governance consistency |
| Hardware | Include stress/degradation scenarios where relevant |
| Safety | Verify deny/approve/HITL behaviors and audit signals |
| GUI/Browser/Voice | Validate end-to-end behavior for interactive substrates |

## 6. Failure Handling & Recovery

- Scenario failure: capture trace, classify root cause, continue batch where configured.
- Harness/runtime mismatch: fail clearly with contract-level diagnostics.
- Transient substrate failures: apply bounded retry policy for deterministic tests only.
- Judge error: fail run explicitly rather than masking verdict integrity.

Recovery:
- Rerun with identical seed/config to confirm reproducibility before triage.

## 7. Performance & Constraints

Constraints:
- Full-matrix eval runs can be time- and cost-intensive.
- Provider/hardware variability can introduce noise without controlled configs.
- High-fidelity integration tests have slower execution than unit-like checks.

Tradeoff:
- Broader eval coverage improves confidence but increases runtime/cost.

## 8. Security & Safety

Controls:
- Potentially dangerous scenarios run in constrained/sandbox-aware contexts.
- Eval-time approval shortcuts must be explicit and isolated from production configs.
- Sensitive artifacts should be handled according to repository security practices.

Trust boundaries:
- External integrations under test remain untrusted and must not bypass safeguards.

## 9. Observability

Capture:
- Scenario-level pass/fail and score breakdowns.
- Latency and timeout distributions per subsystem.
- Safety decision outcomes during eval runs.
- Regression deltas across run history.

Diagnostics:
- Keep machine-readable outputs for CI and human-readable summaries for engineering triage.

## 10. Future Evolution

1. Expand deterministic benchmark suites for provider/tool/safety regressions.
2. Improve failure clustering for faster root-cause analysis.
3. Add subsystem-specific SLO gates (latency, denial correctness, recovery behavior).
4. Keep eval criteria aligned to implementation-grounded authority contracts.
