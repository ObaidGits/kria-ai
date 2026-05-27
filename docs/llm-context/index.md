# KRIA LLM Context

This folder is the compact AI-facing context pack for KRIA development.

Use it when an assistant or LLM needs fast orientation before editing the project. It complements the canonical docs in `docs/architecture/`, `docs/orchestration/`, `docs/contracts/`, and `docs/operations/`.

## Files

| File | Purpose |
|---|---|
| `entry-points.md` | Main source entry points for desktop, core runtime, UI, tools, safety, LLM, voice, sidecar, server, fleet, and evals. |
| `project-graph-summary.md` | High-level dependency and flow summary for the workspace. |
| `query-guide.md` | How to trace dependencies, UI/backend flows, feature ownership, and likely blast radius. |
| `project-graph.json` | Machine-readable project graph for tooling and model context. |
| `context-scope.json` | Scan scope and ignore rules used to define the AI context pack. |
| `routing-corpus.jsonl` | Small routing/intent corpus for L0-style classifier examples. |

## Recommended Reading Order

1. `../index.md`
2. `../architecture/overview.md`
3. `../architecture/core-runtime.md`
4. `entry-points.md`
5. `query-guide.md`
6. `project-graph-summary.md`

For GUI work, also read:
- `../architecture/gui-cognition-runtime.md`
- `../orchestration/gui-execution.md`

For safety/HITL work, also read:
- `../architecture/safety-hitl-runtime.md`
- `../contracts/hitl-mvp/01-boundary.md`

## Rules

- Treat this folder as orientation, not execution authority.
- Verify source files before invasive code changes.
- Canonical architecture and contract truth lives in the main `docs/` folders.
- Do not reintroduce a separate AI context folder; keep AI-facing project context here.
