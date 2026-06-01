# KRIA n8n Stage 3.0 Implementation Report

Date: 2026-05-30
Status: READY for bounded Stage 3 first slice
Scope: metadata-only workflow suggestions, top-3 ranking, explicit confirmation

## 1. Executive Summary

Stage 3.0 first slice is implemented without semantic routing, embeddings,
vector search, recommendation engines, AI workflow generation, autonomous
selection, or automatic execution.

The implemented flow is:

```text
User prompt
-> metadata ranking
-> top workflow candidates
-> explicit user confirmation
-> workflow execution
```

No approved workflow is auto-run by the Stage 3 route. The UI and local API both
return suggestions first, and execution requires a confirmation action or
confirmation prompt.

## 2. Implemented Contract

| Requirement | Status | Evidence |
| --- | --- | --- |
| `WorkflowCandidate` | Implemented | `crates/kria-core/src/n8n/matching.rs` |
| `WorkflowRankingEngine` | Implemented | `crates/kria-core/src/n8n/matching.rs` |
| `WorkflowSuggestionResponse` | Implemented | `crates/kria-core/src/n8n/matching.rs` |
| `WorkflowConfirmationFlow` | Implemented | `crates/kria-core/src/n8n/matching.rs` |
| Tauri suggestion command | Implemented | `suggest_n8n_workflows` |
| Local API suggestion path | Implemented | `crates/kria-desktop/src/commands/local_api.rs` |
| Agent loop suggestion path | Implemented | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| Confirmation-only execution | Implemented | local API confirmation path and UI `confirmed: true` |
| Direct UI auto-run blocked | Implemented | `invoke_n8n_workflow_from_ui` rejects unconfirmed requests |
| Frontend suggestion UI | Implemented | `WorkflowSuggestionCard.tsx`, `N8nWorkflowHub.tsx` |
| Routing eval script | Implemented | `scripts/run_n8n_stage3_routing_eval.sh` |

## 3. Ranking Inputs

The ranking engine uses only approved workflow metadata:

- workflow ID
- display name
- aliases
- tags
- category
- example prompts

It does not use LLM scoring, embeddings, semantic search, vector databases, or
runtime recommendations.

## 4. Safety Behavior

| Prompt class | Behavior |
| --- | --- |
| Exact workflow ID/name/alias/tag | Return candidate and require confirmation |
| Ambiguous match | Return top candidates and require user choice |
| Hard or broad prompt | Return clarification/suggestions, no execution |
| Yellow/HITL workflow | Requires explicit confirmation before execution |
| Unknown workflow | Returns not-found response, no execution |

`can_auto_run` is always `false` in Stage 3.0 suggestion responses.

## 5. Frontend Integration

Added native bounded-routing UI:

- prompt input in the n8n workflow hub,
- `WorkflowSuggestionCard`,
- candidate list,
- confidence badge,
- risk badge,
- Confirm and Cancel controls,
- workflow cards changed from direct `Run` to `Review`,
- no invocation command is sent until Confirm is clicked.

The Playwright Tauri-mock smoke test now verifies:

```text
Review -> suggest_n8n_workflows -> no invoke_n8n_workflow_from_ui
Confirm -> invoke_n8n_workflow_from_ui
```

## 6. Runtime Integration

Local API behavior:

- `Run <workflow>` returns suggestions only.
- `Confirm workflow <workflow_id>` executes.
- Unknown workflows return a user-safe not-found message.

Tauri behavior:

- `suggest_n8n_workflows` returns ranked candidates.
- `invoke_n8n_workflow_from_ui` now requires `confirmed = true`.

Agent loop behavior:

- workflow-like prompts produce bounded suggestions.
- confirmed prompts can invoke the existing n8n tool path.

## 7. Verification Evidence

Key evidence from the implementation run:

| Check | Result |
| --- | --- |
| Stage 3 routing eval | READY, 0 failures |
| Phase 6 readiness gate | READY_IF_ALL_CHECKS_PASS, 6/3 workflows |
| Basic n8n eval | 11 passed / 0 failed |
| Live E2E | 11 passed / 0 failed |
| Stage 2.6 catalog E2E | 69 passed / 0 failed |
| Full capability eval | 23 passed / 0 failed / 18 skipped |
| Core n8n unit tests | 40 passed / 0 failed |
| Desktop n8n tests | 9 passed / 0 failed |
| UI Vitest | 28 passed / 0 failed |
| Playwright Tauri mock | 1 passed / 0 failed |
| `git diff --check` | passed |

## 8. Scope Boundary

This is not full intelligent routing. It is the first bounded Stage 3 slice.

Still not implemented by design:

- semantic ranking,
- embeddings,
- vector databases,
- AI workflow generation,
- autonomous workflow choice,
- background recommendations,
- auto-run from natural language.

## 9. Verdict

READY for bounded Stage 3 activation over the approved workflow catalog.

This verdict applies to the metadata-only suggestion and confirmation flow. It
does not claim support for future workflows that are still unapproved or absent
from the KRIA catalog.
