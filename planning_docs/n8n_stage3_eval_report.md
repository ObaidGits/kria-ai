# KRIA n8n Stage 3.0 Eval Report

Date: 2026-05-30
Final verdict: READY

## 1. Summary

Stage 3.0 bounded routing was evaluated against the routing dataset and live n8n
execution path.

The routing dataset contains 100 prompts. The current approved catalog covers
60 of them. The other 40 prompts target future/unapproved workflows
(`daily_business_brief`, `github_issue_triage`, `invoice_extract_to_sheet`,
`jira_create_ticket`) and were skipped as out of current catalog scope.

## 2. Routing Dataset Results

Command:

```bash
./scripts/run_n8n_stage3_routing_eval.sh
```

Report:

```text
/home/obaid/.kria/eval_reports/n8n_stage3_routing_eval_20260530_003842.txt
```

Results:

| Metric | Target | Actual | Status |
| --- | ---: | ---: | --- |
| Approved workflows | >= 3 | 6 | PASS |
| Dataset prompts | 100 | 100 | PASS |
| Evaluated prompts | current approved catalog | 60 | PASS |
| Easy accuracy | 100% | 100.0% | PASS |
| Medium accuracy | >= 90% | 100.0% | PASS |
| Hard clarification rate | >= 95% | 100.0% | PASS |
| False auto-run rate | 0% | 0.0% | PASS |
| Failed evaluated prompts | 0 | 0 | PASS |

Approved workflows evaluated:

- `calendar_create_meeting`
- `gmail_inbox_digest`
- `gmail_search_messages`
- `gmail_send_draft`
- `slack_post_update`
- `test_workflow`

Skipped future/unapproved workflow prompts:

| Workflow | Skipped prompts |
| --- | ---: |
| `daily_business_brief` | 10 |
| `github_issue_triage` | 10 |
| `invoice_extract_to_sheet` | 10 |
| `jira_create_ticket` | 10 |

## 3. Runtime Evals

| Suite | Result | Report |
| --- | --- | --- |
| Basic n8n eval | 11 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_eval_20260530_003711.txt` |
| Live E2E | 11 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_live_e2e_20260530_003718.txt` |
| Stage 2.6 catalog E2E | 69 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_stage2_6_catalog_e2e_20260530_003729.txt` |
| Full capability eval | 23 passed / 0 failed / 18 skipped | `/home/obaid/.kria/eval_reports/n8n_capability_20260530_003801.txt` |
| Phase 6 readiness gate | 6 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase6_readiness_20260530_003842.txt` |

Live E2E verified:

```text
chat prompt -> suggestion -> confirmation -> n8n webhook -> signed callback
-> KRIA state machine -> governance -> persistence -> SSE
```

Stage 2.6 catalog E2E verified the five production workflows by ID, display
name, and alias, with callback persistence and governance for each.

## 4. Static And UI Checks

| Command | Result |
| --- | --- |
| `cargo fmt` | PASS |
| `cargo build -p kria-desktop` | PASS |
| `cargo test -p kria-core n8n --lib` | 40 passed / 0 failed |
| `cargo test -p kria-desktop n8n` | 9 passed / 0 failed |
| `cargo check -p kria-core` | PASS |
| `cargo check -p kria-desktop` | PASS |
| `cd ui && npm run check` | PASS |
| `cd ui && npm run test:run` | 28 passed / 0 failed |
| `cd ui && npm run build` | PASS |
| `cd tests/e2e && npm run test:tauri-mock -- n8n-workflow-hub.tauri-mock.e2e.spec.ts` | 1 passed / 0 failed |
| `git diff --check` | PASS |

Known warnings:

- Rust warnings in unrelated/pre-existing modules, including Telegram and older
  agent test code.
- Existing Vitest warnings in `app.tool-choice.test.ts`.
- Vite chunking warning for `workflowSession.ts`.

None of these warnings failed the Stage 3 implementation or eval gates.

## 5. Safety Verification

| Safety rule | Evidence | Status |
| --- | --- | --- |
| No auto-run | `can_auto_run = false`, eval false auto-run 0.0% | PASS |
| Hard prompts ask clarification | hard clarification 100.0% | PASS |
| Ambiguous prompts show candidates | ranking engine returns top candidates and `needs_clarification` | PASS |
| Confirmation required before execution | UI command rejects `confirmed = false`; local API uses `Confirm workflow` | PASS |
| No LLM/embedding/vector scoring | eval script and code use metadata only | PASS |

## 6. Limitations

The 40 skipped prompts are not failures. They refer to future workflows that are
not part of the approved Stage 2.6 production catalog. They should become active
eval rows only after those workflows are registered, validated, approved, and
given routing-quality metadata.

## 7. Final Verdict

READY.

KRIA is ready for the bounded Stage 3 first slice over approved workflows:
metadata ranking, top-3 suggestions, user confirmation, and no automatic
workflow execution.
