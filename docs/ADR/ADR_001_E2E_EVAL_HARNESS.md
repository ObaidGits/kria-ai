# ADR: End-to-End Evaluation Harness for KRIA

Status: Proposed  
Date: 2026-05-05  
Owner: Systems Architecture  
Scope: `crates/kria-eval` (new) and evaluation seams in `crates/kria-core` (`agent`, `tools::exec`, HTTP boundaries, `safety`, storage wiring)

## 1) Decision Summary

We will implement a production-grade E2E evaluation harness that executes comprehensive prompt suites (for example, `TestPrompts.txt`) safely and deterministically, without using the desktop UI path.

The decision is built on five mandatory pillars:

1. Programmatic Injection: add a new crate `crates/kria-eval` that injects test prompts directly into the agent turn pipeline at TurnGate entry, bypassing Tauri/UI transport.
2. Global Sandbox Mode: introduce `KRIA_EVAL_MODE=1` to force mocked behavior for all process execution and HTTP/network clients, with fail-closed behavior for unmocked calls.
3. Policy Mocking: in Eval Mode, auto-approve RED-tier/PIN-gated actions so runs cannot deadlock waiting for human approval.
4. LLM-as-a-Judge: evaluate actual `ToolResult` JSON plus emitted events against expected outcomes using a local model judge with deterministic inference settings.
5. Ephemeral State: every test run uses isolated temporary filesystem roots and in-memory SQLite so runs are fully reproducible and side-effect free.

No Rust implementation code is included in this ADR.

## 2) Context and Problem Statement

Current testing is split across unit/integration tests plus manual prompting through UI/runtime paths. That is insufficient for large regression suites such as the comprehensive prompt matrix in `TestPrompts.txt`:

- Manual execution is slow and non-deterministic.
- UI transport adds non-essential variability for tool-behavior validation.
- RED-tier HITL approvals can block unattended runs.
- Process and network tools can mutate host state or depend on live network.
- Shared persistent state (filesystem/db) introduces test order dependence.

We need a repeatable E2E harness that validates routing, tool execution, policy outcomes, and result quality under strict safety isolation.

## 3) Architectural Decision

## 3.1 Pillar 1: Programmatic Injection via `crates/kria-eval`

### Decision

Create a new crate `crates/kria-eval` that drives `kria-core` directly, bypassing UI and desktop command handlers.

### Injection Boundary

Prompts are injected into the same agent runtime path used by production logic, but entry is programmatic:

1. `kria-eval` constructs user-turn input (`ChatMessage` list and session metadata).
2. It invokes agent loop execution directly (no Tauri invoke, no UI stores).
3. TurnGate planning runs normally (`plan_turn` and replanning behaviors remain active).
4. Stream/tool events are captured by the harness collector.

This preserves core runtime semantics while removing UI noise.

### New Crate Responsibilities (`crates/kria-eval`)

1. Prompt Suite Loader
- Parse structured prompts and expected outcomes from text/JSON fixtures.
- Maintain stable test IDs (`SYS-01`, `SAFE-04`, etc.) and tags.

2. Runner
- Execute one case or full suite.
- Support deterministic ordering and filtered subsets.

3. Turn Injector
- Build per-case turn input and invoke `AgentLoop` directly.
- Record TurnGate intent/resource plan metadata for explainability.

4. Observation Collector
- Capture stream events, tool-call payloads, policy outcomes, timing, and final assistant response.

5. Reporter
- Emit machine-readable artifacts (JSON) and human summary output.

### Required Core Seams

Minimal seam additions in `kria-core` for clean harnessing:

1. A stable evaluation-facing entrypoint in agent runtime (wrapping existing `run`/`run_with_profile` semantics) with deterministic options.
2. Structured exposure of key observation data needed by judge logic:
- tool name
- parsed args
- policy decision
- tool result payload
- execution status and timing

These seams are additive and do not replace production desktop flow.

## 3.2 Pillar 2: Global Sandbox Mode (`KRIA_EVAL_MODE=1`)

### Decision

Add a process-wide Eval Mode switch controlled by environment variable:

- `KRIA_EVAL_MODE=1` enables strict sandbox behavior.
- Default (`unset`/`0`) keeps normal production behavior.

### Enforcement Principle

Eval Mode must be centrally enforced and fail closed:

- If a boundary is not mock-capable, the call fails with a deterministic error.
- No silent fallback to host OS/network is allowed.

### Boundary 1: Process Execution

`ExecWrapper` becomes sandbox-aware:

1. In normal mode: execute commands as today.
2. In eval mode: resolve command fingerprint (`program + args`) against fixture map and return mocked `CommandOutput`.
3. If no fixture exists: return deterministic `ToolExecutionError::SpawnFailed` equivalent flagged as unmocked-boundary.

### Boundary 2: HTTP/Network Clients

All HTTP clients used by tools and network-facing modules route through a common client factory abstraction:

1. Normal mode: real `reqwest` clients.
2. Eval mode: mock transport returning fixture responses keyed by `(method, url, query/body signature)`.
3. Unknown request in eval mode: deterministic blocked error.

### Sandbox Fixture Model

Per test case (or suite-level defaults), fixtures define:

- command mocks (stdout/stderr/exit/status/duration)
- HTTP mocks (status/headers/body/latency)
- optional synthetic failures/timeouts

Fixture resolution order:

1. case override
2. suite default
3. global default
4. fail closed

### Explicit Non-Goals for Eval Mode

- Not a security sandbox replacement for production.
- Not seccomp/container hardening.
- It is deterministic simulation for test execution.

## 3.3 Pillar 3: Policy Mocking for RED/PIN Actions

### Decision

In Eval Mode, RED-tier actions that normally require HITL/PIN are auto-approved to avoid deadlock.

### Guardrails

1. BLACK-tier actions remain blocked.
2. Policy classification still runs and must still classify as RED/YELLOW/GREEN/BLACK.
3. Audit records must state that approval was synthetic (`DecidedBy::EvalHarness` or equivalent).
4. Stream events still emit approval-required and approval-result transitions (so UI/event contract coverage is preserved when needed).

### Behavior Contract

For RED action in Eval Mode:

1. `PolicyEngine` returns `requires_approval=true` as normal.
2. Approval gateway path immediately resolves `Approved` without waiting for user input.
3. Tool execution proceeds under sandbox mocks.

This keeps policy semantics visible while preventing run stalls.

## 3.4 Pillar 4: LLM-as-a-Judge (Local)

### Decision

Add a judge module in `kria-eval` that grades each case as Pass/Fail using:

- expected outcome text/metadata
- observed tool result JSON/events/policy behavior
- deterministic local model inference

### Two-Stage Grading Pipeline

Stage A: Deterministic Rule Checks (hard gates)

- required tool name(s) invoked
- policy gate behavior correct (for example PIN-required vs auto-execute)
- blocked/allowed outcome matches expectation
- essential output shape fields present

If Stage A fails, result is immediate Fail with explicit reason.

Stage B: Local LLM Judge

- Input packet includes prompt, expected outcome, observed evidence, and strict rubric.
- Model returns strict JSON verdict:
  - `grade`: `PASS` or `FAIL`
  - `confidence`: numeric 0-1
  - `reasons`: short list
  - `evidence_refs`: event/tool references

### Determinism Controls for Judge

1. Temperature fixed to `0.0`.
2. Top-p fixed and seed fixed where backend supports it.
3. Prompt template versioned and snapshotted.
4. Output schema validated; invalid output becomes deterministic Fail with parser reason.

### Judge Inputs

At minimum, the judge receives:

- case id and prompt
- expected outcome block
- ordered stream events
- policy decision trail
- tool call args and final `ToolResult`
- final assistant textual response

## 3.5 Pillar 5: Ephemeral State per Test Run

### Decision

Every test case runs with isolated transient state:

1. temporary filesystem root (tempdir)
2. in-memory SQLite databases
3. per-case runtime IDs and cleanup

### Filesystem Isolation

For each case, allocate a dedicated root like:

- `<temp>/kria-eval/<run_id>/<case_id>/...`

All harness-created artifacts, fixture files, and logs are rooted there.

Path-sensitive prompts are mapped via test fixture translation when needed so assertions remain realistic while avoiding host mutation.

### Database Isolation

Use in-memory SQLite (`:memory:` or equivalent open-in-memory APIs) for:

- audit logs
- memory/fact/snippet stores used by test runtime
- temporary metadata tables used by harness

No persistent DB writes from eval runs.

### Cleanup and Artifact Retention

1. Success cases: cleanup temp roots by default.
2. Failed cases: retain artifacts (configurable) for triage.
3. Always emit reproducibility metadata (seed, fixture versions, model id, case id).

## 4) Detailed System Design

## 4.1 `crates/kria-eval` Module Layout (Proposed)

1. `suite`
- parse and normalize prompt cases from source files

2. `fixtures`
- command/http mock definitions and lookup

3. `sandbox`
- eval mode config, boundary adapters, fail-closed guards

4. `injector`
- build turn input and invoke agent entrypoint

5. `collector`
- capture events, tool results, policy snapshots, timing

6. `judge`
- Stage A rules + Stage B LLM grading

7. `report`
- JSON/markdown summaries and per-case artifacts

8. `runner`
- orchestration for single case, suite, retries, and final exit status

## 4.2 Execution Flow

1. Load suite definitions and expected outcomes.
2. Build per-case ephemeral runtime (temp fs + in-memory db + sandbox fixtures).
3. Enable Eval Mode and construct core runtime with eval adapters.
4. Inject prompt directly into agent loop (TurnGate path preserved).
5. Collect full evidence packet from stream and tool boundaries.
6. Run Stage A checks, then LLM judge.
7. Emit case verdict and persist artifacts.
8. Teardown ephemeral state.
9. Aggregate run summary and return non-zero exit if any required case fails.

## 4.3 Data Contracts (Logical)

### `EvalCase`
- `id`
- `prompt`
- `expected_outcome`
- `tags`
- `fixtures_ref`

### `EvalObservation`
- `case_id`
- `events[]`
- `tool_calls[]`
- `policy_trace[]`
- `final_response`
- `timings`

### `EvalVerdict`
- `case_id`
- `stage_a_pass`
- `judge_grade`
- `confidence`
- `reasons[]`
- `artifacts`

### `EvalRunReport`
- `run_id`
- `summary` (pass/fail counts)
- `case_results[]`
- `environment` (model, seed, fixture versions)

## 5) Determinism Requirements

The harness is considered deterministic only when:

1. No host OS command or live network call occurs in Eval Mode.
2. Case ordering is stable unless explicit shuffle seed is set.
3. Timeouts, retries, and simulated delays are fixture-driven.
4. Judge inference uses fixed deterministic parameters.
5. Re-running same suite with same fixtures and seed yields same verdict set.

## 6) Rollout Plan

## Phase 0: Foundations

1. Create `crates/kria-eval` scaffold and suite parser.
2. Add eval config object + `KRIA_EVAL_MODE` bootstrap wiring.
3. Add run report schema and artifact directory contract.

Exit criteria:

- harness can load cases and produce dry-run report without executing tools.

## Phase 1: Injection + Observation

1. Add agent eval entry seam and programmatic turn execution.
2. Capture stream events, policy decisions, and tool outputs.

Exit criteria:

- one real case executes end-to-end through TurnGate path without UI.

## Phase 2: Global Sandbox Mode

1. Wire ExecWrapper mock adapter.
2. Wire HTTP client factory mock adapter.
3. Enforce fail-closed for unmocked boundaries.

Exit criteria:

- host process/network isolation proven by tests.

## Phase 3: Policy Mocking + Ephemeral State

1. Add RED auto-approval in Eval Mode.
2. Keep BLACK blocks intact and audited.
3. Enable per-case temp fs + in-memory db bootstrap/teardown.

Exit criteria:

- RED cases do not deadlock; repeated runs show no state bleed.

## Phase 4: LLM Judge + CI Integration

1. Implement Stage A rule checker.
2. Implement local model judge packet + strict JSON verdict parser.
3. Add CI target for deterministic eval subset.

Exit criteria:

- CI emits stable pass/fail matrix and artifacts.

## 7) Validation and Testing Strategy

1. Harness self-tests
- parser correctness
- fixture resolution precedence
- fail-closed behavior for missing mocks

2. Boundary tests
- ExecWrapper in eval mode never spawns OS process
- HTTP eval client never performs live requests

3. Policy tests
- RED auto-approve only in Eval Mode
- BLACK actions remain blocked

4. Isolation tests
- no filesystem/db cross-case leakage

5. Judge tests
- rule-check hard failures
- malformed LLM output handling
- deterministic verdict snapshots for known fixtures

## 8) Risks and Mitigations

1. Risk: mock fixture drift from production behavior
- Mitigation: add periodic fixture parity review against real tool contracts.

2. Risk: LLM judge false positives/negatives
- Mitigation: Stage A hard checks before LLM, schema-validated outputs, and low-temperature deterministic settings.

3. Risk: hidden live boundary escapes
- Mitigation: fail-closed adapters and explicit runtime counters for blocked live attempts.

4. Risk: test suite runtime cost
- Mitigation: case tagging, parallel workers with isolated state, and deterministic smoke subset for PRs.

## 9) Acceptance Criteria

This ADR initiative is complete only when all are true:

1. `crates/kria-eval` can run prompt suites by injecting turns into the agent path without UI.
2. `KRIA_EVAL_MODE=1` prevents host OS process execution and live HTTP/network calls.
3. RED-tier approvals auto-resolve in Eval Mode without manual PIN input.
4. Every evaluated case receives a structured Pass/Fail verdict from Stage A + LLM judge pipeline.
5. Each case executes with isolated temp filesystem and in-memory SQLite state.
6. Re-running a fixed suite with fixed fixtures and seed produces consistent verdicts.

## 10) Non-Goals

1. Replacing production safety policy behavior outside Eval Mode.
2. Rewriting tool business logic as part of harness rollout.
3. Making UI automation a prerequisite for E2E tool evaluation.
4. Using cloud-hosted judge models for baseline evaluation correctness.

## 11) Immediate Next Step

Start with a pilot vertical slice:

1. Add `crates/kria-eval` runner for a small subset (for example `SYS-01`, `SH-03`, `SAFE-04`).
2. Enable Eval Mode for ExecWrapper plus RED auto-approval.
3. Produce first deterministic report artifact and lock its schema for subsequent phases.
