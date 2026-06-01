# KRIA n8n Stage 2.6 Catalog Buildout Report

Date: 2026-05-30 00:10 IST
Status: COMPLETE for Stage 3 routing-readiness catalog gate

## 1. Executive Summary

Stage 2.6 added five production-purpose workflow entries to KRIA and provisioned
matching destructive-safe n8n callback harness workflows for end-to-end proof.

New approved workflows:

| Workflow | Category | Risk | HITL | Endpoint |
| --- | --- | --- | --- | --- |
| `gmail_inbox_digest` | email | Green / read-only | none | `/webhook/kria-gmail-inbox-digest` |
| `gmail_search_messages` | email | Green / read-only | none | `/webhook/kria-gmail-search-messages` |
| `gmail_send_draft` | email | Yellow / reversible external | required review | `/webhook/kria-gmail-send-draft` |
| `calendar_create_meeting` | calendar | Yellow / reversible external | required review | `/webhook/kria-calendar-create-meeting` |
| `slack_post_update` | messaging | Yellow / reversible external | required review | `/webhook/kria-slack-post-update` |

Current KRIA catalog now contains 6 approved workflows:

- `test_workflow`
- `gmail_inbox_digest`
- `gmail_search_messages`
- `gmail_send_draft`
- `calendar_create_meeting`
- `slack_post_update`

Current live n8n instance contains 7 workflows:

- `KRIA Test Workflow`
- `get_all_mails`
- `KRIA Gmail Inbox Digest`
- `KRIA Gmail Message Search`
- `KRIA Gmail Draft Creator`
- `KRIA Calendar Meeting Creator`
- `KRIA Slack Update Poster`

No Stage 3 routing, embeddings, semantic search, or AI workflow generation was
implemented.

## 2. Implementation Artifacts

| Area | Artifact |
| --- | --- |
| Registry | `config/default.toml`, `~/.kria/config.toml` |
| Metadata model | `crates/kria-core/src/n8n/types.rs` |
| Approval gate | `crates/kria-core/src/n8n/readiness.rs` |
| Deterministic baseline matching | `crates/kria-core/src/n8n/matching.rs` |
| Desktop command metadata wiring | `crates/kria-desktop/src/commands/n8n.rs` |
| UI metadata display/import wiring | `ui/src/stores/n8n.ts`, `ui/src/components/N8nWorkflowManagementPanel.tsx`, `ui/src/components/N8nWorkflowCard.tsx` |
| Schemas | `schemas/n8n/*.input.json`, `schemas/n8n/*.output.json` |
| n8n provisioning | `scripts/provision_n8n_stage2_6_workflows.sh` |
| Catalog E2E gate | `scripts/run_n8n_stage2_6_catalog_e2e.sh` |
| Routing baseline gate | `scripts/run_n8n_routing_baseline.sh` |

## 3. Metadata Coverage

Every new workflow has:

- stable `workflow_id`
- display name
- description
- category
- tags
- aliases
- example prompts
- input schema ref
- output schema ref
- expected evidence
- owner
- credential requirements
- data scope
- external transfer flag
- HITL policy
- timeout class
- endpoint path
- approved status

The Stage 3 readiness predicate now requires routing-quality metadata including
category and example prompts, not only a display name and tags.

## 4. Schema Coverage

| Workflow | Input schema | Output schema |
| --- | --- | --- |
| `test_workflow` | `schemas/n8n/test_workflow.input.json` | `schemas/n8n/test_workflow.output.json` |
| `gmail_inbox_digest` | `schemas/n8n/gmail_inbox_digest.input.json` | `schemas/n8n/gmail_inbox_digest.output.json` |
| `gmail_search_messages` | `schemas/n8n/gmail_search_messages.input.json` | `schemas/n8n/gmail_search_messages.output.json` |
| `gmail_send_draft` | `schemas/n8n/gmail_send_draft.input.json` | `schemas/n8n/gmail_send_draft.output.json` |
| `calendar_create_meeting` | `schemas/n8n/calendar_create_meeting.input.json` | `schemas/n8n/calendar_create_meeting.output.json` |
| `slack_post_update` | `schemas/n8n/slack_post_update.input.json` | `schemas/n8n/slack_post_update.output.json` |

## 5. Callback And Lifecycle Support

The five new n8n workflows use the KRIA callback envelope:

- `schema_version`
- `correlation_id`
- `causation_id`
- `event_id`
- `sequence_number`
- `workflow_id`
- `workflow_version`
- `n8n_run_id`
- `status`
- `evidence`
- `side_effects`
- `occurred_at_ms`

The harness workflows sign the exact callback body using
`KRIA_N8N_SIGNING_SECRET`, send callbacks to KRIA, and return a safe webhook
acknowledgement. This proves KRIA invocation, callback authentication,
governance, persistence, SSE, and history for each workflow without performing
real external Gmail, Calendar, or Slack side effects.

## 6. E2E Validation Results

Catalog E2E:

```text
Report: /home/obaid/.kria/eval_reports/n8n_stage2_6_catalog_e2e_20260529_235855.txt
SUMMARY: 54 passed / 0 failed / 54 total
```

Coverage:

- prompt by workflow ID
- prompt by display name
- prompt by alias
- callback persisted
- governance verified
- SSE event observed
- history endpoint reports tracked runs

Full live E2E regression:

```text
Report: /home/obaid/.kria/eval_reports/n8n_live_e2e_20260530_000128.txt
SUMMARY: 10 passed / 0 failed / 10 total
```

Reliability regression:

```text
Report: /home/obaid/.kria/eval_reports/n8n_reliability_20260530_000137.txt
SUMMARY: 17 passed / 0 failed / 17 total
```

n8n prompt/API eval:

```text
Report: /home/obaid/.kria/eval_reports/n8n_eval_20260530_000141.txt
SUMMARY: 10 passed / 0 failed / 10 total
```

Full capability eval:

```text
Report: /home/obaid/.kria/eval_reports/n8n_capability_20260530_000620.txt
TOTAL: 41 | PASS: 23 | FAIL: 0 | SKIP: 18
Testable Pass Rate: 100%
```

Stage 3 readiness gate:

```text
Report: /home/obaid/.kria/eval_reports/n8n_phase6_readiness_20260530_000705.txt
Stage 3 readiness status: READY_IF_ALL_CHECKS_PASS
6/3 approved workflows have routing-quality metadata
```

## 7. Routing Baseline

Deterministic routing baseline:

```text
Report: /home/obaid/.kria/eval_reports/n8n_routing_baseline_20260530_000705.txt
Approved workflows in catalog: 6
Dataset prompts: 100
Evaluated prompts: 60
Skipped prompts for future/unapproved workflows: 40
Easy accuracy: 24/24 = 100.0%
Medium accuracy: 24/24 = 100.0%
Hard clarification/no-auto-run rate: 12/12 = 100.0%
Hard false auto-run rate: 0/12 = 0.0%
```

The 40 skipped prompts target workflows not included in this Stage 2.6 slice:

- `daily_business_brief`
- `github_issue_triage`
- `invoice_extract_to_sheet`
- `jira_create_ticket`

## 8. Static And UI Verification

Completed checks:

```text
cargo test -p kria-core n8n --lib
cargo test -p kria-desktop n8n
cargo check -p kria-core
cargo check -p kria-desktop
cd ui && npm run check
cd ui && npm run test:run
cd ui && npm run build
git diff --check
```

Known note: Solid/Vitest warnings in `app.tool-choice.test.ts` remain existing
test noise and did not fail the run.

## 9. Limitations

- The new n8n workflows are destructive-safe callback harnesses. They validate
  KRIA workflow selection, invocation, callbacks, governance, persistence,
  history, and SSE, but they do not yet perform real Gmail, Calendar, or Slack
  API side effects.
- Yellow workflows are intentionally marked with review/HITL-oriented metadata
  because real external side effects should remain confirmable.
- The routing baseline covers the implemented catalog. The full 100-prompt
  dataset includes 40 future-workflow prompts, which are skipped until those
  workflows are added.

## 10. Verdict

Stage 2.6 catalog buildout is complete.

KRIA now has enough approved routing-quality workflow metadata to unlock the
Stage 3 first slice: metadata ranking, top-3 suggestions, user confirmation,
and no auto-run on ambiguity.
