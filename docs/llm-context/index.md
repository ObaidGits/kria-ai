# KRIA LLM Context

**Purpose:** Compact AI-facing orientation pack for KRIA development.
**Last updated:** 2026-05-27
**Authority:** Orientation only. Source code and canonical docs remain authoritative.

This folder gives an assistant fast project context before code or docs work. It replaces the older separate `ai-context` concept and should stay small enough to read early in a session.

## Files

| File | Purpose |
|---|---|
| `entry-points.md` | Current source entry points for desktop, core runtime, GUI cognition, HITL, tools, safety, model orchestration, voice, sidecars, server, fleet, and evals. |
| `project-graph-summary.md` | Human-readable dependency and flow summary for the workspace. |
| `query-guide.md` | Practical guide for tracing dependencies, UI/backend flows, feature ownership, and blast radius. |
| `project-graph.json` | Machine-readable project graph for tooling and model context. |
| `context-scope.json` | Scan/watch scope and ignore rules used for this context pack. |
| `routing-corpus.jsonl` | Small L0 classifier calibration corpus. Keep labels limited to the operations supported by `onnx_classifier.rs`. |

## Recommended Reading Order

1. `../index.md`
2. `../architecture/overview.md`
3. `../architecture/core-runtime.md`
4. `entry-points.md`
5. `query-guide.md`
6. `project-graph-summary.md`
7. `project-graph.json` only when machine-readable structure is useful.

## Domain-Specific Additions

For GUI cognition or desktop automation:
- `../architecture/gui-cognition-runtime.md`
- `../orchestration/gui-execution.md`
- `../orchestration/runtime-authority.md`
- `../evaluations/gui-e2e.md`

For safety, HITL, or action approval:
- `../architecture/safety-hitl-runtime.md`
- `../contracts/hitl-mvp/01-boundary.md`
- `../contracts/hitl-mvp/02-runtime-contracts.md`
- `../contracts/hitl-mvp/04-execution-gate.md`

For tool execution:
- `../orchestration/tool-system.md`
- `../decisions/adr/002-tool-execution-overhaul.md`
- `crates/kria-core/src/tools/registry.rs`

For model/provider orchestration:
- `../architecture/llm-orchestrator-runtime.md`
- `../operations/provider-orchestration.md`
- `crates/kria-core/src/llm/model_router.rs`
- `crates/kria-core/src/llm/orchestrator/`

For eval work:
- `../evaluations/overview.md`
- `crates/kria-eval/src/`
- `crates/kria-core/tests/`
- `tests/e2e/`

## Rules

- Treat this folder as map, not law.
- Verify source files before invasive changes.
- Keep AI-facing context here; do not create a second context folder.
- Keep `routing-corpus.jsonl` compatible with `onnx_classifier.rs`.
- Keep generated logs, eval reports, and local artifacts out of this context pack.
