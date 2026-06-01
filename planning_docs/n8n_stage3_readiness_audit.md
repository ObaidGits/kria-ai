# KRIA n8n Stage 3 Readiness Audit

Date: 2026-05-29
Scope: workflow inventory, metadata quality, overlap analysis, routing readiness
Status: NOT READY for Stage 3 Intelligent Workflow Routing

## 1. Executive Summary

Phase 0 through Phase 6 infrastructure is ready, but Stage 3 workflow selection
is not ready yet. The blocker is catalog quality, not execution.

Current inventory:

| Source | Count | Notes |
| --- | ---: | --- |
| KRIA workflow registry | 1 | `test_workflow` |
| Approved KRIA workflows | 1 | `test_workflow` |
| Executable KRIA workflows | 1 | `test_workflow` |
| Live n8n workflows | 2 | `KRIA Test Workflow`, `get_all_mails` |
| Live n8n workflows not registered in KRIA | 1 | `get_all_mails` |
| Stage 3 routing-quality workflows by current gate | 1/3 | Gate remains blocked |

Latest gate evidence:

```text
./scripts/run_n8n_phase6_readiness_gate.sh
Stage 3 readiness status: BLOCKED (only 1/3 approved workflows have routing-quality metadata)
Report: /home/obaid/.kria/eval_reports/n8n_phase6_readiness_20260529_231419.txt
```

Verdict: **NOT READY**.

KRIA can run the current approved workflow reliably, but it cannot yet prove
that it can choose correctly among multiple production workflows.

## 2. Evidence Sources

Audited sources:

| Source | Finding |
| --- | --- |
| `config/default.toml` | 1 registered workflow |
| `~/.kria/config.toml` | 1 runtime registered workflow |
| `docker exec n8n n8n list:workflow` | 2 live n8n workflows |
| `docker exec n8n n8n export:workflow ...` | exported live workflow definitions |
| `crates/kria-core/src/n8n/readiness.rs` | Stage 3 gate requires 3 routing-quality approved workflows |
| `crates/kria-core/src/n8n/matching.rs` | deterministic matcher uses exact ID, display name, alias, or tag |
| `./scripts/run_n8n_phase5_invocation.sh` | deterministic invocation gate passed |

## 3. Catalog Audit

### 3.1 Workflow Inventory

| Workflow | Source | Approved | Executable | Metadata Quality | Routing Ready | Notes |
| --- | --- | --- | --- | ---: | --- | --- |
| `test_workflow` | KRIA registry | Yes | Yes | 68/100 | Limited | Good diagnostic workflow; not a production business workflow |
| `KRIA Test Workflow` | Live n8n | Indirectly mapped by endpoint | Yes in n8n | 30/100 | No as standalone | Underlying n8n workflow for `test_workflow`; no separate KRIA registry metadata |
| `get_all_mails` | Live n8n | No | No | 18/100 | No | Inactive, unregistered, no KRIA callback contract, no routing metadata |

### 3.2 Approval And Execution

| Workflow | Approval State | Execution State | Reason |
| --- | --- | --- | --- |
| `test_workflow` | Approved in KRIA | Executable | Approved registry entry, webhook endpoint active, callback path verified |
| `KRIA Test Workflow` | Not a separate KRIA entry | Active in n8n | It is the live n8n implementation behind `test_workflow` |
| `get_all_mails` | Not approved | Not executable through KRIA | Missing registry entry, inactive in n8n, no callback, no approval metadata |

## 4. Metadata Audit

### 4.1 Scoring Rubric

Routing Quality Score is 0-100:

| Area | Points |
| --- | ---: |
| Stable workflow ID and display name | 10 |
| Approval and executable state | 15 |
| Clear functional description | 15 |
| Tags, aliases, and category coverage | 15 |
| Expected inputs and outputs | 15 |
| Example prompts | 15 |
| Safety/governance metadata | 10 |
| Low overlap with other workflows | 5 |

### 4.2 Workflow Scores

| Workflow | ID/Name | Approval/Execution | Description | Tags/Aliases/Category | Inputs/Outputs | Examples | Safety | Overlap | Score |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `test_workflow` | 10 | 15 | 9 | 11 | 8 | 0 | 10 | 5 | 68 |
| `KRIA Test Workflow` | 8 | 8 | 0 | 0 | 0 | 0 | 9 | 5 | 30 |
| `get_all_mails` | 7 | 0 | 0 | 0 | 0 | 0 | 6 | 5 | 18 |

### 4.3 Missing Metadata

| Workflow | Missing Metadata |
| --- | --- |
| `test_workflow` | `category`, `example_prompts`, actual schema files for `input_schema_ref` and `output_schema_ref`, production purpose |
| `KRIA Test Workflow` | KRIA registry entry, description, tags, aliases, category, example prompts, input/output schema refs |
| `get_all_mails` | KRIA registry entry, approval, activation decision, callback contract, description, tags, aliases, category, example prompts, input/output schemas, governance metadata |

### 4.4 Routing Quality Notes

`test_workflow` passes the current code-level Stage 3 metadata predicate because
it is approved and has display name, description, approval metadata, tags, and
aliases. It is still weak as a production routing example because it is a
diagnostic workflow.

`get_all_mails` is not safe to route until it is imported as a draft, validated,
given a KRIA callback contract or synchronous result contract, approved, and
made active intentionally.

## 5. Overlap Analysis

### 5.1 Current Catalog Ambiguity

Current KRIA registry has only one workflow, so no multi-workflow ambiguity can
be proven yet.

| Prompt | Current Behavior | Risk |
| --- | --- | --- |
| `Run test_workflow` | Unique match | Low |
| `Run Test Workflow` | Unique match | Low |
| `Run test n8n workflow` | Unique alias match | Low |
| `Run diagnostic` | Unique tag match | Low today, risky later if many diagnostic workflows exist |
| `Check my email` | No match | Good fail-closed behavior |
| `Get all mails` | No match | Correct because `get_all_mails` is not in KRIA registry |

### 5.2 Current Conflict Matrix

| Workflow A | Workflow B | Current Conflict | Notes |
| --- | --- | --- | --- |
| `test_workflow` | `get_all_mails` | None in KRIA | `get_all_mails` is not registered |
| `test_workflow` | `KRIA Test Workflow` | Mapping duplicate | They represent the same diagnostic endpoint, not two routable KRIA workflows |

### 5.3 Future Conflict Classes

The first real Stage 3 catalog will likely create conflicts in these areas:

| Conflict Class | Example Ambiguous Prompt | Possible Candidates | Required Behavior |
| --- | --- | --- | --- |
| Email read vs email send | `handle this email` | inbox digest, search mail, send draft | Ask clarification |
| Messaging channel | `send the report to the team` | Gmail, Slack, Teams | Ask channel and recipient |
| Reporting destination | `publish the update` | Slack, LinkedIn, email, CRM note | Ask destination |
| Ticket vs issue | `create a bug task` | Jira, GitHub issue, linear task | Ask tracker |
| Finance document handling | `process this invoice` | extract to sheet, create accounting bill, email approval | Ask target action |
| Daily summary | `brief me` | calendar brief, inbox digest, business KPI digest | Ask summary scope |

## 6. Recommended Workflow Catalog Improvements

Minimum missing workflows to add before Stage 3:

| Workflow ID | Purpose | Inputs | Outputs | Tags | Routing Examples |
| --- | --- | --- | --- | --- | --- |
| `gmail_inbox_digest` | Summarize recent important emails | time window, labels, priority filter | digest, message refs | email, inbox, digest | `summarize my inbox`, `show important emails` |
| `gmail_search_messages` | Search email by sender/topic/date | query, sender, date range | matching messages | email, search, gmail | `find emails from accounting` |
| `gmail_send_draft` | Create a safe email draft, not auto-send | recipient, subject, body, attachments | draft link, preview | email, draft, send | `draft an email to Alex` |
| `calendar_create_meeting` | Create calendar meeting after confirmation | title, attendees, time, duration | event link | calendar, meeting, schedule | `schedule a meeting tomorrow` |
| `slack_post_update` | Post approved update to Slack | channel, message, attachments | message permalink | slack, message, update | `post this update to Slack` |

Ten useful production workflows:

| Workflow ID | Type | Purpose | Inputs | Outputs | Tags |
| --- | --- | --- | --- | --- | --- |
| `gmail_inbox_digest` | Personal productivity | Inbox summary | time window, labels | digest | email, inbox, digest |
| `gmail_search_messages` | Personal productivity | Email search | query, filters | message list | email, search |
| `gmail_send_draft` | Personal productivity | Email draft creation | recipient, subject, body | draft | email, draft |
| `calendar_create_meeting` | Personal productivity | Meeting scheduling | attendees, time | event | calendar, meeting |
| `daily_personal_brief` | Personal productivity | Calendar plus inbox brief | date, sources | brief | daily, brief |
| `slack_post_update` | Business | Team update posting | channel, message | permalink | slack, update |
| `jira_create_ticket` | Business | Ticket creation | summary, description, priority | ticket URL | jira, ticket |
| `github_issue_triage` | Business | Issue labeling/triage | repo, issue criteria | triage report | github, issue |
| `invoice_extract_to_sheet` | Business | Invoice extraction | document, vendor | sheet row | invoice, finance |
| `crm_lead_capture` | Business | Lead capture from text/form | lead details | CRM record | crm, lead |

Three high-value business workflows:

| Workflow ID | Purpose | Why High Value |
| --- | --- | --- |
| `invoice_extract_to_sheet` | Extract invoice data into a reviewable sheet | Saves manual data entry and has clear inputs/outputs |
| `crm_lead_capture` | Create CRM lead from email/form/chat | Direct revenue workflow with measurable business impact |
| `jira_create_ticket` | Turn user reports into tracked tickets | Improves operational follow-through |

Three personal productivity workflows:

| Workflow ID | Purpose | Why Useful |
| --- | --- | --- |
| `gmail_inbox_digest` | Summarize inbox | Frequent, low-risk read-only workflow |
| `calendar_create_meeting` | Schedule meetings | Common task with clear confirmation step |
| `daily_personal_brief` | Combine calendar and email priorities | Natural KRIA assistant use case |

## 7. Objective Stage 3 Readiness Gate

Recommended gate before enabling Stage 3 first slice:

| Requirement | Minimum | Current | Status |
| --- | ---: | ---: | --- |
| Approved workflows | 3 | 1 | FAIL |
| Routing-ready workflows by KRIA gate | 3 | 1 | FAIL |
| Production-purpose workflows | 3 | 0 | FAIL |
| Average metadata score | 80/100 | 68/100 for only registered workflow | FAIL |
| Minimum per-workflow score | 75/100 | 68/100 | FAIL |
| Example prompts per workflow | 10 | 0 | FAIL |
| Input/output schema files exist | 100% | 0% found | FAIL |
| No exact alias/tag collisions | 100% | 100% today | PASS |
| Routing eval dataset exists | 100 prompts | Created in `planning_docs/n8n_routing_eval_dataset.md` | PASS |
| Easy eval expected accuracy | 100% | Not run against multi-workflow catalog | PENDING |
| Medium eval expected accuracy | >= 90% | Not run | PENDING |
| Hard ambiguity behavior | >= 95% asks clarification | Not run | PENDING |

## 8. Exact Work Needed

Recommended next implementation order:

1. Register at least three real workflows as KRIA drafts, not approved.
2. Add complete metadata: description, category, tags, aliases, example prompts,
   input schema, output schema, expected evidence, data scope, HITL policy.
3. Validate each workflow JSON using Phase 4.5 validation.
4. Add callback or synchronous output contract for each workflow.
5. Approve only Green/read-only or HITL-protected Yellow workflows first.
6. Run live E2E for each approved workflow.
7. Run the routing eval dataset without semantic routing to establish the
   deterministic baseline.
8. Only then implement Stage 3 first slice: metadata ranking, top 3 suggestions,
   user confirmation, and no auto-run on ambiguity.

## 9. Final Verdict

Stage 3 Intelligent Workflow Routing: **NOT READY**.

Reason:

```text
Only 1/3 approved workflows have routing-quality metadata.
The only registered workflow is diagnostic, not a production workflow catalog.
The live n8n instance contains one additional inactive Gmail workflow, but it is
not registered, not approved, and not callback-safe for KRIA.
```

The correct next step is catalog buildout and eval preparation, not new routing
architecture.
