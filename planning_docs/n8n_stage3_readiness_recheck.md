# KRIA n8n Stage 3 Readiness Recheck

Date: 2026-05-30 00:10 IST
Verdict: READY

## 1. Scope

This recheck measures whether KRIA is ready to start Stage 3 Intelligent
Workflow Routing after the Stage 2.6 catalog buildout.

No Stage 3 routing was implemented during this work. No embeddings, semantic
search, AI workflow generation, recommendation engine, or model-based workflow
selection was added.

## 2. Current Catalog State

| Requirement | Current Evidence | Status |
| --- | --- | --- |
| Minimum approved workflows: 3 | 6 approved workflows in KRIA config | PASS |
| Minimum routing-quality workflows: 3 | Phase 6 gate reports 6/3 | PASS |
| Minimum production-purpose workflows: 3 | 5 production-purpose workflow entries added | PASS |
| Metadata includes category | Required by config/model/readiness gate | PASS |
| Metadata includes example prompts | Required by config/model/readiness gate | PASS |
| Schemas exist for each approved workflow | 12 schema files under `schemas/n8n/` | PASS |
| Live n8n workflows exist for new catalog entries | 5 Stage 2.6 workflows provisioned in n8n | PASS |
| Real callback path verified per workflow | Stage 2.6 E2E 54/54 | PASS |
| Reliability suite still passes | 17/17 | PASS |
| No semantic/model routing started | Phase 6 gate check passes | PASS |

## 3. Workflow Readiness Table

| Workflow | Approved | Executable | Metadata Quality | Routing Ready |
| --- | --- | --- | --- | --- |
| `test_workflow` | Yes | Yes | Diagnostic-quality | Yes, but diagnostic |
| `gmail_inbox_digest` | Yes | Yes | Complete | Yes |
| `gmail_search_messages` | Yes | Yes | Complete | Yes |
| `gmail_send_draft` | Yes | Yes | Complete, HITL-aware | Yes |
| `calendar_create_meeting` | Yes | Yes | Complete, HITL-aware | Yes |
| `slack_post_update` | Yes | Yes | Complete, HITL-aware | Yes |

## 4. Gate Evidence

Stage 3 readiness gate:

```text
./scripts/run_n8n_phase6_readiness_gate.sh
Report: /home/obaid/.kria/eval_reports/n8n_phase6_readiness_20260530_000705.txt
Result: 6 passed / 0 failed / 6 total
Stage 3 readiness status: READY_IF_ALL_CHECKS_PASS
6/3 approved workflows have routing-quality metadata
```

Catalog E2E:

```text
./scripts/run_n8n_stage2_6_catalog_e2e.sh
Report: /home/obaid/.kria/eval_reports/n8n_stage2_6_catalog_e2e_20260529_235855.txt
Result: 54 passed / 0 failed / 54 total
```

Reliability:

```text
./scripts/run_n8n_reliability_tests.sh
Report: /home/obaid/.kria/eval_reports/n8n_reliability_20260530_000137.txt
Result: 17 passed / 0 failed / 17 total
```

Full capability:

```text
./scripts/run_n8n_full_capability_eval.sh
Report: /home/obaid/.kria/eval_reports/n8n_capability_20260530_000620.txt
Result: 23 passed / 0 failed / 18 skipped
Testable pass rate: 100%
```

## 5. Routing Baseline

Deterministic baseline over the 100-prompt dataset:

```text
./scripts/run_n8n_routing_baseline.sh
Report: /home/obaid/.kria/eval_reports/n8n_routing_baseline_20260530_000705.txt
Approved workflows in catalog: 6
Dataset prompts: 100
Evaluated prompts: 60
Skipped future/unapproved prompts: 40
Easy accuracy: 24/24 = 100.0%
Medium accuracy: 24/24 = 100.0%
Hard clarification/no-auto-run rate: 12/12 = 100.0%
Hard false auto-run rate: 0/12 = 0.0%
```

Interpretation:

- Implemented catalog prompts pass the deterministic baseline.
- Ambiguous hard prompts do not auto-run.
- Forty prompts are intentionally skipped because they belong to workflows not
  included in this Stage 2.6 slice.

## 6. Remaining Risks

| Risk | Impact | Current Control |
| --- | --- | --- |
| Provider side effects are harnessed, not live Gmail/Slack/Calendar actions | Stage 3 can prove selection, but not provider credentials | Keep Stage 3 first slice suggestion/confirmation-only |
| Future dataset prompts are skipped | Full 10-workflow routing benchmark is incomplete | Add future workflows before claiming full 100-prompt coverage |
| Broad aliases can overlap | Ambiguous natural prompts may produce multiple candidates | Stage 3 must return top candidates and require confirmation |
| Yellow workflows could affect external systems when real nodes are connected | User trust/safety risk | Keep HITL/confirmation required |

## 7. Allowed Stage 3 First Slice

Stage 3 may begin only with the bounded first slice already defined by the
Phase 6 gate:

1. Rank workflows using existing metadata only.
2. Return top 3 suggestions.
3. Ask user to confirm.
4. Do not auto-run on ambiguous prompts.

Still prohibited:

- embeddings
- semantic search
- autonomous workflow chaining
- AI workflow generation
- recommendation engine
- hidden auto-run from vague prompts

## 8. Final Verdict

READY.

KRIA now meets the Stage 3 activation gate for the first routing slice. The
system has 6 approved routing-quality workflows, 5 production-purpose catalog
entries, passing callback/E2E/reliability evidence, and a deterministic routing
baseline with no false auto-runs for evaluated prompts.

Stage 3 should start conservatively with metadata ranking and explicit user
confirmation. Full 100-prompt coverage should wait until the four future
workflow families are added.
