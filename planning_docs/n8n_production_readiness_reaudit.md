# KRIA n8n Production Readiness Reaudit

Date: 2026-05-29
Previous report: `planning_docs/n8n_phase0_to_6_verification_report.md`
Previous verdict: NO-GO
Current verdict: GO for Phase 0 through Phase 6 integration readiness; NO-GO for starting Stage 3 intelligence until the readiness gate is satisfied.

## 1. Executive Summary

The previous audit marked the integration NO-GO mainly because Phase 4.5 was
missing and several production hardening items were incomplete. This reaudit
verified the fixes in code, scripts, unit tests, browser smoke tests, and a
running KRIA plus n8n instance.

Result:

| Area | Result |
| --- | --- |
| Phase 0-6 implementation claim | PASS |
| Phase 4.5 authoring/validation | PASS |
| Live KRIA -> n8n -> callback loop | PASS |
| Reliability failure injection | PASS |
| UI smoke, including Playwright Tauri mock | PASS |
| Stage 3 intelligence readiness | BLOCKED as intended, 1/3 workflows |

Production readiness score: 90/100 for the bounded n8n integration currently
implemented. The missing 10 points are mainly because only one real approved
workflow is registered and Stage 3 intelligence must remain blocked.

## 2. Architecture Verification

### Architecture Diagram

```text
KRIA UI
  |-- n8n Settings
  |-- Workflow Hub
  |-- Workflow Cards
  |-- Run Progress
  |-- Evidence Viewer
  |
  v
Tauri Commands
  |-- get_n8n_status
  |-- get_n8n_runtime_status
  |-- invoke_n8n_workflow_from_ui
  |-- validate_n8n_workflow_draft
  |-- dry_run_n8n_workflow_validation
  |-- backup_n8n_workflow
  |-- rollback_n8n_workflow_backup
  |-- create_or_update_n8n_workflow_draft
  |
  v
KRIA Core n8n
  |-- Config and secret resolution
  |-- Catalog and approval metadata
  |-- Client signed invocation
  |-- Callback parser/HMAC/freshness
  |-- State machine and replay TTL
  |-- Governance
  |-- Workflow validation
  |
  v
n8n Runtime
  |-- managed Docker or external
  |-- webhook execution
  |-- signed callback to KRIA
```

### Data Flow Diagram

```text
User action or chat prompt
  -> KRIA workflow matcher/catalog
  -> signed n8n webhook request
  -> n8n workflow execution
  -> signed callback envelope
  -> KRIA local API /api/n8n/callback
  -> HMAC/freshness/schema checks
  -> state machine ingest
  -> governance verification
  -> JSONL persistence
  -> Tauri/SSE event emission
  -> UI store refresh
  -> workflow card/progress/chat result
```

### Event Flow Diagram

```text
n8n:runtime_status
n8n:workflow_invocation_started
n8n:workflow_invocation_accepted
n8n:workflow_invocation_failed
n8n:callback
n8n:governance
n8n:workflow_timeout
n8n:chat_result
```

Backend emitters and frontend listeners are present. The Phase 3 gate now fails
if the named lifecycle events are removed.

## 3. Code Verification

| Feature | Implementation evidence | Reachability |
| --- | --- | --- |
| Workflow validator | `crates/kria-core/src/n8n/workflow_validation.rs` | Exported from `n8n/mod.rs`, called by desktop authoring commands |
| Static JSON validation | `validate_n8n_workflow_json` | Covered by core tests |
| Callback contract validation | `callback_contract` checks in validator | Covered by bad fixture tests |
| Secret leak detector | `secret_leak` checks in validator | Covered by bad fixture tests |
| Dry-run validation | `dry_run_n8n_workflow_validation` | Registered Tauri command, returns `mutated_n8n=false` |
| Backup | `write_n8n_workflow_backup`, `backup_n8n_workflow` | Tested with backup roundtrip |
| Rollback | `rollback_n8n_workflow_backup` | Command registered and backup read path tested |
| Create/update safety | `create_or_update_n8n_workflow_draft` | Rejects failed validation, saves as draft, backs up existing registry entry |
| Destructive-safe CRUD fixture | `n8n_destructive_safe_crud_fixture_import_approve_disable_delete` | Rust test passes |
| Replay TTL/cap | `SEEN_EVENT_TTL_MS`, `MAX_SEEN_EVENTS`, dead-letter cap in `state.rs` | Core n8n suite passes |
| SSE redaction | `redacted_n8n_run_for_sse` and evidence shape redaction | Live SSE snapshot verified redacted |
| Token bootstrap hardening | `/api/auth/token` disabled unless `KRIA_LOCAL_API_ALLOW_TOKEN_ENDPOINT=1` | Live endpoint returned 403 |
| Lifecycle events | n8n/local API/runtime emitters and `ui/src/stores/n8n.ts` listeners | Phase 3 gate and Playwright smoke pass |

No new semantic routing, embeddings, recommendation engine, or autonomous
workflow chaining was introduced.

## 4. Test Results

Latest passing evidence:

| Suite | Result | Report |
| --- | --- | --- |
| Phase 0 contract | 4 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase0_contract_20260529_225139.txt` |
| Runtime modes | 5 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_runtime_modes_20260529_231240.txt` |
| Phase 2 UI | 5 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase2_ui_20260529_225146.txt` |
| Phase 3 progress/events | 6 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase3_progress_20260529_225210.txt` |
| Phase 4 management | 5 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase4_management_20260529_225212.txt` |
| Phase 4.5 authoring | 5 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_workflow_authoring_validation_20260529_225907.txt` |
| Phase 5 invocation | 5 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase5_invocation_20260529_225215.txt` |
| Phase 6 readiness | 6 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_phase6_readiness_20260529_231419.txt` |
| UI smoke | 3 passed / 0 failed | `/home/obaid/.kria/eval_reports/n8n_ui_smoke_20260529_230616.txt` |

Static/build checks passed:

```text
cargo check -p kria-core
cargo check -p kria-desktop
cargo test -p kria-core n8n --lib
cargo test -p kria-desktop n8n
cd ui && npm run check
cd ui && npm run test:run
cd ui && npm run build
cd tests/e2e && npm run typecheck
```

Known warnings:

- Existing Rust unused warnings outside the n8n blocker path.
- Existing Vitest mocked-store warnings in `app.tool-choice.test.ts`.
- Playwright emitted a `NO_COLOR`/`FORCE_COLOR` warning; test passed.

## 5. E2E Results

Fresh KRIA was started from current code and n8n was already healthy on
`127.0.0.1:5678`.

Live proof:

```text
./scripts/run_n8n_live_e2e.sh
SUMMARY: 10 passed / 0 failed / 10 total
Report: /home/obaid/.kria/eval_reports/n8n_live_e2e_20260529_231334.txt
```

This verified:

- KRIA local API health.
- n8n health.
- n8n Docker secret availability.
- Active test webhook.
- Chat prompt invocation.
- Real signed terminal callback.
- KRIA callback acceptance.
- Governance audit persistence.
- n8n event stream output.

## 6. CRUD Results

| Operation | Result |
| --- | --- |
| Create/import draft | Implemented; imported workflows remain draft |
| Read/list workflows | Existing status/hub paths pass |
| Update draft | Implemented through create/update-as-draft with validation |
| Delete registry entry | Existing command and UI path present |
| Import workflow | Existing command creates draft |
| Approve workflow | Requires metadata and rebuilds catalog |
| Disable workflow | Blocks catalog execution |
| Enable workflow | Approval path re-enables execution after metadata validation |
| Export/backup | Backup records implemented locally |
| Rollback | Backup restore command implemented |

Destructive-safe fixture:

```text
cargo test -p kria-desktop n8n_destructive_safe_crud_fixture
Result: 1 passed, 0 failed
```

The fixture avoids mutating production n8n while proving the registry lifecycle
and execution blocking semantics.

## 7. Event Flow Results

| Event | Backend | Frontend | Verification |
| --- | --- | --- | --- |
| `n8n:runtime_status` | Emitted by runtime/status commands | Store listener exists | Phase 3 gate |
| `n8n:workflow_invocation_started` | Emitted by UI and local API invocation paths | Store listener exists | Phase 3 gate, Playwright smoke |
| `n8n:workflow_invocation_accepted` | Emitted after accepted invoke | Store listener exists | Phase 3 gate, Playwright smoke |
| `n8n:workflow_invocation_failed` | Emitted on invoke failure | Store listener exists | Phase 3 gate |
| `n8n:callback` | Existing callback path | Store listener exists | Live E2E/reliability |
| `n8n:governance` | Existing governance path | Store listener exists | Live E2E/reliability |
| `n8n:workflow_timeout` | Emitted by maintenance timeout loop | Store listener exists | Phase 3 gate |
| `n8n:chat_result` | Existing terminal result path | Store listener exists | Live E2E |

## 8. Failure Injection Results

Reliability suite:

```text
./scripts/run_n8n_reliability_tests.sh
17 passed / 0 failed
Report: /home/obaid/.kria/eval_reports/n8n_reliability_20260529_231359.txt
```

Covered failures:

- invalid signature,
- missing signature,
- duplicate callback,
- out-of-order callback,
- post-terminal callback,
- malformed JSON,
- oversized payload,
- wrong workflow version,
- unknown workflow,
- failed callback governance,
- concurrent callback isolation,
- persistence verification.

Additional failure hardening:

- stale/future callbacks covered by core n8n tests,
- disabled workflow rejection covered by core/desktop tests,
- invalid workflow JSON covered by Phase 4.5 validator tests,
- token bootstrap endpoint verified live as disabled by default,
- SSE evidence payload verified redacted.

## 9. Security Findings

| Finding | Status | Evidence |
| --- | --- | --- |
| Literal secrets in config/workflow exports | Fixed | Phase 0 contract pass |
| Replay ID memory growth | Fixed | TTL/cap in `state.rs` |
| Local API token endpoint open | Fixed | `/api/auth/token` returned 403 by default |
| SSE evidence body exposure | Fixed | Live SSE snapshot returned redacted field shapes |
| Workflow JSON secret leakage | Fixed | Validator rejects secret-like literals |
| Bad workflow overwrite risk | Fixed | Update writes pre-update backup and saves only draft |
| Managed Docker secret CLI exposure | Fixed | Managed n8n env is written to a local 0600 env-file and Docker receives only `--env-file` |

## 10. Reliability Findings

The callback path is now production-reliable for the bounded workflow set:

- HMAC and freshness fail closed.
- Duplicate and out-of-order events fail closed.
- Terminal protection prevents post-terminal mutation.
- Governance persistence is durable.
- JSONL inbox and audit files are written.
- Timeout events are emitted.
- UI no longer depends only on raw "running" state.

Remaining operational note:

- There is currently only one real approved workflow with full metadata, so
  Stage 3 intelligence readiness correctly remains blocked.

## 11. R1-R10 Closure Matrix

| ID | Previous issue | Current status | Evidence |
| --- | --- | --- | --- |
| R1 | Phase 4.5 missing | FIXED | Validator, authoring commands, backup, rollback, dry-run, draft save |
| R2 | No authoring eval script | FIXED | `scripts/run_n8n_workflow_authoring_validation.sh` passes |
| R3 | No destructive-safe CRUD fixture | FIXED | `n8n_destructive_safe_crud_fixture_import_approve_disable_delete` passes |
| R4 | Missing named invocation events | FIXED | Backend emitters and frontend listeners verified by Phase 3 gate |
| R5 | No browser smoke test | FIXED | Playwright Tauri mock n8n hub smoke passes |
| R6 | Unbounded replay ID set | FIXED | TTL/cap in state store |
| R7 | Local API token endpoint open | FIXED | Endpoint disabled by default, live 403 |
| R8 | SSE includes evidence payload bodies | FIXED | SSE emits redacted field shapes and `side_effects_count` |
| R9 | Stage 3 readiness blocked | PROVEN INTENTIONAL | Phase 6 gate reports blocked because only 1/3 workflows |
| R10 | Phase 6 gate omits Phase 4.5 evidence | FIXED | Readiness evidence checks latest authoring validation report |

## 12. Phase Scorecard

| Phase | Score | Evidence |
| --- | --- | --- |
| Phase 0 | PASS | Contract script and core n8n tests pass |
| Phase 1 | PASS | Live E2E passes |
| Phase 1.5 | PASS | Runtime mode script passes |
| Phase 2 | PASS | UI hub script, UI tests, build pass |
| Phase 3 | PASS | Progress/event gate passes |
| Phase 4 | PASS | Management gate and desktop tests pass |
| Phase 4.5 | PASS | Authoring validation gate passes |
| Phase 5 | PASS | Deterministic invocation gate passes |
| Phase 6 | PASS | Readiness gate implemented and blocks Stage 3 correctly |

## 13. Production Risks

| Risk | Current severity | Mitigation |
| --- | --- | --- |
| Only one real approved workflow | Medium | Register at least two more metadata-complete workflows before Stage 3 |
| Full n8n live import/export not used in dry-run | Low for current scope | Current authoring pipeline is non-mutating and draft-only; add live compatibility roundtrip only before activating generated workflows |
| Existing non-n8n Rust/UI warnings | Low | Track separately; not blocking n8n readiness |
| Long-running timeout UX depends on maintenance loop interval | Low | Timeout event exists; keep reliability coverage |

## 14. Final Recommendation

GO for claiming KRIA n8n Phase 0 through Phase 6 complete for the bounded,
deterministic integration.

NO-GO for starting Stage 3 intelligence routing until Phase 6 readiness reports
READY. The current block is correct and expected:

```text
Stage 3 readiness status: BLOCKED (only 1/3 approved workflows have routing-quality metadata)
```

Next production-safe step is to register at least three real approved workflows
with complete metadata, then rerun:

```text
./scripts/run_n8n_phase6_readiness_gate.sh
./scripts/run_n8n_live_e2e.sh
./scripts/run_n8n_reliability_tests.sh
```
