# Implementation Plan: Memory Graph Production Redesign

## Overview

Backend-first, implementation-ready plan for MGR-001–048. Gates execute strictly F0→F5, with optional F6 only after signed release evidence; exact implementation paths must be adapted to discovered repository ownership rather than added as a parallel architecture.

**Status (evidence-reconciled 2026-07-29):** Checkboxes below were re-marked to match the actual code and on-disk Evidence Artifacts, replacing an earlier state where all 388 boxes were checked without supporting evidence. A task is `[x]` only when its implementation exists and its listed commit-specific Evidence Artifacts (or passing unit tests) exist; `[-]` where the implementation exists but the required heavy validation/evidence/independent campaign has not been produced; `[ ]` where no artifacts exist yet.

### Checkbox legend

- `[x]` **Done** — implemented and unit-tested, or the gate's Evidence Artifacts exist and validate.
- `[-]` **Partial** — code/capability is implemented, but a required validation run, evidence artifact, independent review, or dead-code cutover is still outstanding.
- `[ ]` **Not started** — no implementation run or evidence artifact exists on disk yet.

### Current reconciled state

- **F0–F2 (tasks 0, 1, 2): Done.** Evidence tooling, SQLite authority schema v2, and the semantic model/links/interchange base are implemented and unit-tested; `evidence/F0`, `evidence/F1`, `evidence/F2` manifests validate.
- **F3 (task 3): Partial.** Retrieval strategies, classifier/RRF, gates, cognition, scheduler, and the v2 API (3.1–3.8, 3.9.1–3.9.6) are implemented and unit-tested. The ≥200-query judged evaluation (3.9.7) is **done** — evidence at `evidence/F3/run-001/` (220 queries, Recall@10=0.9948, nDCG@10=1.0000, all thresholds passed). The 100k materialized run + legacy-API deletion (3.9.8) are deferred.
- **F4 (task 4): Partial.** Client/session/reducers, seven destinations, semantic list, inspector, scene, and Canvas2D (4.1–4.8) are implemented and unit-tested. The E2E/screenshot/axe/Orca/WebKitGTK campaigns (4.9.2–4.9.5) and old-UI dead-code deletion (4.9.6) are deferred.
- **F5 (task 5): Partial / Not started.** No heavy-campaign artifacts exist on disk (only `evidence/F5/run-001/manifest.json`, which itself lists 8 deferred NBW items). The 100k fixture is not materialized (2,000 records present vs 100,000 required), and no SBOM, performance samples, visual/a11y matrix, or multi-window traces exist.
- **F6 (task 6): Not started.** `evidence/F6/run-001/study/preregistration.json` is marked `DEFERRED`; no 3D spike or study exists.

Reviews recorded as `owner-self-review` are accepted for this single-developer pre-production repository per `.kiro/steering/dev-context.md`; independent multi-role sign-off is not treated as a blocker here.

## Notes

The prior task plan contained useful audit context but used stale phase numbering and unsupported `[x]`/`[~]` claims. Treat it as superseded planning history; no previous checkbox is implementation evidence.

## Execution Contract

- Execute gates strictly `F0 → F1 → F2 → F3 → F4 → F5`; `F6` is optional and may start only from a signed F5 manifest. No later-gate polish may mask an earlier P0 failure.
- Before every major task, inspect the named current modules, imports, tests, and composition roots. Paths below are **targets subject to repository discovery**: adapt or relocate the existing implementation using repository conventions; do not build a parallel architecture, duplicate authority, compatibility write path, second scene model, or renderer-owned domain model.
- `kria-core` owns domain semantics; Tauri/Axum are thin adapters; SQLite is the sole transactional authority; FTS5, vectors, caches, analytics, scenes, and renderers are rebuildable projections.
- Hard cutovers are preferred for this single-user pre-production repository. After parity evidence, delete superseded schemas, writers, routes, DTOs, stores, UI models, tests, dependencies, and claims. Rollback means capability disable, bounded v2 refetch, local read-only, Recovery_Mode, or KRIA-data reset/reimport—never restoration of an unsafe legacy path.
- Verification is colocated with each implementation slice. Phase-gate evidence tasks aggregate and sign artifacts; they do not replace focused tests. Heavy 100k/model/release/WebKitGTK/Orca/fault/SBOM runs are serialized on the owner laptop.
- Canonical evidence root: `.kiro/specs/memory-graph-production-redesign/evidence/<gate>/<run-id>/`; every artifact is checksummed and referenced by `manifest.json` with predecessor hashes, exact command, commit/dirty digest, fixtures, environment, assertions, metrics, and reviewer verdicts.
- Required suite IDs and quantitative oracles are defined in `validation.md`; requirements, decisions, risks, findings, opportunities, and artifact mappings are defined in `traceability.md` and remain normative.

## Task Dependency Graph

```mermaid
flowchart LR
  F0[F0 Evidence reset] --> F1[F1 Authority · security · lifecycle · recovery]
  F1 --> F2[F2 Records · links · truth · sources]
  F2 --> F3[F3 Retrieval · cognition · canonical API]
  F3 --> F4[F4 Seven-destination Digital Twin · list first]
  F4 --> F5[F5 Scale · release proof · cleanup]
  F5 -. optional only .-> F6[F6 3D GO or complete deletion]
```

```json
{
  "waves": [
    { "wave": 0, "tasks": ["0"], "dependsOn": [], "description": "F0 evidence reset and contract freeze" },
    { "wave": 1, "tasks": ["1"], "dependsOn": ["0"], "description": "F1 SQLite authority, security, lifecycle, and recovery" },
    { "wave": 2, "tasks": ["2"], "dependsOn": ["1"], "description": "F2 typed semantics, links, truth, entities, sources, and interchange base" },
    { "wave": 3, "tasks": ["3"], "dependsOn": ["2"], "description": "F3 retrieval, cognition, canonical API, and backend gates" },
    { "wave": 4, "tasks": ["4"], "dependsOn": ["3"], "description": "F4 seven-destination Digital Twin, semantic list first, then conditional Canvas2D" },
    { "wave": 5, "tasks": ["5"], "dependsOn": ["4"], "description": "F5 production hardening, cutover, and release evidence" },
    { "wave": 6, "tasks": ["6"], "dependsOn": ["5"], "description": "F6 optional preregistered 3D GO or complete deletion" }
  ]
}
```

Within each gate, numbered major tasks and subtasks are dependency-ordered unless a task explicitly permits independent lightweight work. The serial critical path is `0.1→0.5→1.1→1.9→2.1→2.7→3.1→3.9→4.1→4.9→5.1→5.8`, then optional `6.1→6.3`.

## Tasks

- [x] 0. F0 — Evidence reset and contract freeze

  **Objective:** Establish machine-checkable IDs, deterministic fixtures, honest current-state claims, exact dependency/model/license facts, and reproducible command/evidence tooling before behavior changes.
  **Targets (subject to discovery):** `.kiro/specs/memory-graph-production-redesign/{requirements,design,validation,traceability,risk-analysis,implementation-roadmap,tasks}.md` (read/reference except this file); new/adapted `crates/kria-eval/src/memory_graph/`; `tests/fixtures/memory-graph/`; `scripts/memory_graph/`; `models/manifest/`; command wiring in `justfile`/CI only when implemented later.
  **Prerequisites:** Normative MGR-001–048, MGD-001–046, 65 findings, 31 opportunities, roadmap, validation contract, and audit are available; no old completion state is trusted.
  **Invariants/non-negotiables:** Status starts Planned/Unverified; expected answers are independent of the system under test; no generated fixture contains real private data; baseline limitations are not acceptance targets; all IDs and artifact references are closed-world validated.
  **Implementation steps:** Complete F0.1–F0.5 in order, then generate the F0 gate manifest.
  **Failure/degraded behavior:** Unknown/duplicate/orphan IDs, missing checksums, unpinned facts, dirty-state omission, or absent reviewer fields fail the gate; unresolved facts are recorded `Unknown`/`Unavailable`, never inferred.
  **Focused validation:** coverage/orphan linter; fixture determinism and schema tests; manifest validator self-tests; claim inventory review; baseline smoke commands; V-REG-01 only for touched tooling.
  **Evidence:** `evidence/F0/<run-id>/{manifest.json,reports/coverage.json,reports/current-claims.json,reports/baseline.json,fixtures/,supply-chain/,reviews/}`.
  **Completion proof:** F0 manifest validates; coverage is MGR 48/48, MGD 46/46, findings 65/65, opportunities 31/31; every command and fixture hash is reproducible; verified implementation count remains zero.
  **Rollback/containment:** Revert malformed planning/tooling only; preserve truthful claim corrections and never restore stale checked status.
  **IDs:** MGR-001, MGR-011, MGR-027, MGR-029, MGR-047, MGR-048; MGD-018, MGD-021, MGD-022, MGD-029, MGD-042; MG-M27, MG-M28.

  - [x] 0.1 F0.1 — Build the evidence ID registry and coverage/orphan linter

    **Objective:** Make every normative ID, suite, gate, risk, workstream, artifact class, command, fixture, and manifest edge parseable and fail closed.
    **Targets (subject to discovery):** new/adapted `crates/kria-eval/src/memory_graph/{mod,registry,coverage}.rs` or `scripts/memory_graph/coverage.py`; fixture schemas under `tests/fixtures/memory-graph/schemas/`; CI/`justfile` command only after local command works.
    **Prerequisites:** F0 entry documents; inspect existing `kria-eval::{suite,report,runner}.rs` and script conventions before choosing Rust versus Python.
    **Invariants/non-negotiables:** One canonical parser; exact ranges; no task checkbox counts as evidence; reverse orphans fail; statuses other than Planned/Unverified require an existing valid manifest hash.
    **Implementation steps:** Execute 0.1.1–0.1.5; keep diagnostics deterministic and machine-readable.
    **Failure/degraded behavior:** Parse ambiguity or undefined IDs returns nonzero with file/line/ID diagnostics; no best-effort pass.
    **Focused validation:** linter unit/golden tests with missing, duplicate, out-of-range, reverse-orphan, bad-gate-order, checklist-only-pass, and checksum-invalid fixtures.
    **Evidence:** `evidence/F0/<run-id>/reports/{id-inventory,coverage,reverse-orphans}.json` and `commands/CMD-MG-COVERAGE.json`.
    **Completion proof:** clean repository inputs produce exact required totals and mutation fixtures each fail for the intended reason.
    **Rollback/containment:** Remove only broken command wiring; retain registry schema and failing policy until corrected.
    **IDs:** MGR-027, MGR-029, MGR-048; MGD-018, MGD-022, MGD-042; all MG-C/H/M/L and MG-O IDs.

    - [x] 0.1.1 Parse MGR-001–048, MGD-001–046, MG-C01–07, MG-H01–17, MG-M01–28, MG-L01–13, MG-O01–31, V-*, R-*, W-*, A-*, CMD-*, fixture, and F0–F6 definitions into one normalized registry with source file and line.
    - [x] 0.1.2 Validate forward mappings from each MGR/MGD to design section, workstream, suite, risk, gate, and artifact class, plus exactly one audit-ledger occurrence for every finding/opportunity.
    - [x] 0.1.3 Validate reverse orphans, duplicate IDs, invalid ranges, undefined codes, later-gate predecessor gaps, and any non-Planned status lacking a valid manifest path/hash.
    - [x] 0.1.4 Add negative golden inputs for each failure class and a stable JSON report schema suitable for CI annotations.
    - [x] 0.1.5 Expose one documented command that writes no spec status, emits totals, and exits nonzero unless coverage is exactly `48/48`, `46/46`, `65/65`, `31/31` with zero reverse orphans.

  - [x] 0.2 F0.2 — Freeze deterministic fixtures, seeds, and independent judged corpus

    **Objective:** Provide reproducible 100/1k/10k/100k, paired-policy, vector-oracle, interchange, visual, and ≥200-query quality inputs with independent expected answers.
    **Targets (subject to discovery):** `tests/fixtures/memory-graph/generators/`; `tests/fixtures/memory-graph/generated/`; `crates/kria-eval/src/memory_graph/{fixtures,judgments}.rs`; reviewer templates.
    **Prerequisites:** F0.1 registry/schema conventions; inspect existing `memory_bench.rs`, `llm_fixture.rs`, and UI E2E fixture style.
    **Invariants/non-negotiables:** Fixed seeds from validation.md; expected memberships/paths/ranks are generated by an independent oracle; no SUT-derived golden regeneration; every package has SHA-256 and versioned generator metadata.
    **Implementation steps:** Execute 0.2.1–0.2.6; generate 100/1k metadata now, defer expensive 100k materialization to F3/F5 while freezing its generator/hash contract.
    **Failure/degraded behavior:** Nondeterminism, hash drift without version bump, missing planted case, judge conflict without adjudication, or secret-like fixture content fails validation.
    **Focused validation:** two-run byte/hash determinism; schema validation; planted-case exact assertions; judge/adjudication completeness checks.
    **Evidence:** `evidence/F0/<run-id>/fixtures/*.json`, `reports/fixture-determinism.json`, `reports/judged-corpus-plan.json`.
    **Completion proof:** all nine fixture contracts resolve to the required seeds/count semantics and a second clean generation matches manifests.
    **Rollback/containment:** Delete invalid generated output, never silently update expected answers; bump generator version only with reviewed rationale.
    **IDs:** MGR-006, MGR-007, MGR-027, MGR-036, MGR-048; MGD-015, MGD-025, MGD-039, MGD-042; MG-H01, MG-H09, MG-M27.

    - [x] 0.2.1 Implement `mg-unit-v2` seed `0x4D475201` with every record/link kind, policy/mode/truth state, invalid row, and idempotency collision.
    - [x] 0.2.2 Implement `mg-small-v2`/`mg-medium-v2` seeds `0x4D475202`/`203` with seven-destination states, long/RTL/CJK labels, outbox/model/corruption/import/source-cancel cases.
    - [x] 0.2.3 Freeze `mg-release-v2` seed `0x4D475204` with 100k authority records, degree distribution, cycles, hidden intermediaries, temporal boundaries, 1/2/3/4-hop paths, and exact independent memberships.
    - [x] 0.2.4 Implement paired-world and vector oracles (`0x4D475205`/`206`) covering hidden labels/IDs/counts/topology/timing and normalized/non-normalized/tie/zero/NaN/Inf/wrong-length vectors.
    - [x] 0.2.5 Author `mg-retrieval-judged-v2` seed `0x4D475207` with ≥200 stratified identifier, phrase, semantic, entity/relation, temporal, goal, contradiction, source, forbidden, and adversarial queries; require two judges or recorded adjudication.
    - [x] 0.2.6 Implement interchange/visual contracts (`0x4D475208`/`209`) with unknown optional/required fields, checksums, no secrets, deterministic revisions/states/layout inputs, and fixture-manifest schema validation.

  - [x] 0.3 F0.3 — Inventory current claims, routes, schemas, models, and licenses

    **Objective:** Produce a code-backed baseline of what is Current, Planned, Unavailable, or Unknown and identify every path that must be cut over.
    **Targets (subject to discovery):** current `crates/kria-core/src/memory/`, `stores/`, schema `0001`–`0010`; desktop `commands/memory.rs`; server `memory_routes.rs`/`auth.rs`; UI `graph/`; model download/config manifests; Cargo/npm/Python locks; root license metadata.
    **Prerequisites:** F0.1; repository discovery must include live registration/composition roots, not filenames alone.
    **Invariants/non-negotiables:** Dormant code/comments/tests are not capability proof; no license is inferred from package name or repository comment; secrets are never copied into reports.
    **Implementation steps:** Execute 0.3.1–0.3.6 and assign an owner/cutover gate to every live or uncertain path.
    **Failure/degraded behavior:** Unresolved registration or license facts remain Unknown and block relevant release claim; no optimistic classification.
    **Focused validation:** source-to-registration reachability review; route/command smoke inventory; lock/model checksum verification; claim wording review.
    **Evidence:** `evidence/F0/<run-id>/reports/{current-claims,write-paths,read-paths,ui-paths,schema-inventory,model-license-inventory}.json`.
    **Completion proof:** every durable writer/read route/import/export/UI renderer/model/dependency has classification, evidence locator, and target cutover gate.
    **Rollback/containment:** None for evidence; correct mistaken records through reviewed inventory revisions.
    **IDs:** MGR-001, MGR-011, MGR-029, MGR-033, MGR-035, MGR-047; MGD-018, MGD-021, MGD-023, MGD-029; MG-C01–C07, MG-M28.

    - [x] 0.3.1 Trace all durable writes from native tools, desktop, server, MCP, OpenClaw, sidecars, imports, cognition, lifecycle, goals, feedback, and tests to their actual store/transaction boundary.
    - [x] 0.3.2 Trace all memory/graph/search/analytics/trace/export reads, cache keys, serialization boundaries, and client-side filtering assumptions.
    - [x] 0.3.3 Inventory schema tables/triggers/indexes/pragmas/migration checksums and identify competing authority, direct SQL, free-text links, mutable events, and obsolete ANN paths.
    - [x] 0.3.4 Inventory active `MemoryUniverse`/fallback/list behavior, dormant `GraphCanvas3D`, synthetic topology, inert controls, visual claims, command names, and test mocks that simulate success.
    - [x] 0.3.5 Pin observed embedding labels, files, source revisions, artifact/tokenizer hashes, dimensions, runtime, normalization, and license disposition as Known or Unknown without downloading during F0.
    - [x] 0.3.6 Reconcile Cargo/npm/Python/model/asset/project license facts and record every conflict, missing lock/checksum, reachability owner, and F1/F5 disposition.

  - [x] 0.4 F0.4 — Implement command catalog and Evidence Artifact manifest tooling

    **Objective:** Make every suite invocation reproducible and every claimed artifact complete, checksummed, environment-bound, and reviewer-bound.
    **Targets (subject to discovery):** `crates/kria-eval/src/memory_graph/{command,manifest,artifact}.rs`; `scripts/memory_graph/`; `justfile`; evidence JSON schemas; CI hooks only after local validation.
    **Prerequisites:** F0.1 registry and F0.2 fixture schemas.
    **Invariants/non-negotiables:** Commands capture cwd/argv/exit code; artifact paths are repository-relative or immutable URI; manifests cannot self-certify missing files; a Pass cannot rely on a checkbox or screenshot alone.
    **Implementation steps:** Execute 0.4.1–0.4.5; support existing and planned command IDs from validation.md.
    **Failure/degraded behavior:** Interrupted run remains Blocked/Fail with partial artifact inventory; invalid checksum, unknown ID, missing predecessor, null required environment, or absent mandatory review fails manifest validation.
    **Focused validation:** schema/property tests over malformed manifests; tamper tests; command wrapper exit propagation; reviewer independence checks.
    **Evidence:** `evidence/F0/<run-id>/{manifest.json,commands/,reports/manifest-validator.json,reviews/}`.
    **Completion proof:** a synthetic pass and each malformed variant produce the expected deterministic verdict; artifact tampering is detected.
    **Rollback/containment:** Disable CI promotion if tooling fails; retain raw command outputs and never infer pass from wrapper failure.
    **IDs:** MGR-027, MGR-028, MGR-029, MGR-048; MGD-022, MGD-042; MG-M27, MG-M28.

    - [x] 0.4.1 Define and runtime-validate the complete `manifest.json` schema from validation.md, including commit/dirty digest, predecessor hashes, fixture/model/schema/scene versions, hardware/power/locale/AT, samples, assertions, artifacts, waivers, and reviews.
    - [x] 0.4.2 Implement streaming SHA-256/size/media-type collection and reject missing, escaping, mutable, duplicate, or checksum-invalid artifact references.
    - [x] 0.4.3 Implement wrappers for existing `CMD-RUST-UNIT`, `CMD-COGNITION`, `CMD-GUI-E2E`, `CMD-ADVERSARIAL`, `CMD-UI-*` and planned `CMD-MG-*` without pretending absent commands exist.
    - [x] 0.4.4 Enforce reviewer role, independence, reviewed hashes, signature method, UTC timestamp, and non-waivable P0/security/privacy/integrity/a11y/license conditions.
    - [x] 0.4.5 Add predecessor/gate promotion logic that changes only generated evidence status and refuses to derive implementation status from `tasks.md` boxes.

  - [x] 0.5 F0.5 — Capture current baseline and issue the F0 gate manifest

    **Objective:** Record honest cold/warm behavior and limitations, freeze reference-hardware protocol, and authorize F1 without claiming readiness.
    **Targets (subject to discovery):** `crates/kria-eval/src/memory_graph/baseline.rs`; existing `ui/e2e/memory-graph-baseline.spec.ts`; evidence/review templates.
    **Prerequisites:** F0.1–F0.4.
    **Invariants/non-negotiables:** Baseline numbers are descriptive; correctness accompanies latency; no broad 100k generation/build is required in F0; current SVG/3D/model/security limitations remain explicit.
    **Implementation steps:** Execute 0.5.1–0.5.4 and sign only after all F0 reports validate.
    **Failure/degraded behavior:** Unavailable measurement is recorded with cause and command; it cannot be replaced by estimated acceptance.
    **Focused validation:** focused smoke of current routes/UI; baseline schema validation; F0 coverage command; manifest tamper check.
    **Evidence:** `evidence/F0/<run-id>/{manifest.json,reports/baseline.json,reviews/spec-owner.json,reviews/qa-evidence.json}`.
    **Completion proof:** signed F0 predecessor hash is usable by F1 and contains no Verified implementation claim.
    **Rollback/containment:** Re-run only invalid baseline slices; do not delay security work for cosmetic baseline completeness when limitations are explicitly Blocked.
    **IDs:** MGR-001, MGR-027, MGR-029, MGR-048; MGD-018, MGD-021, MGD-042; MG-M27–MG-M28.

    - [x] 0.5.1 Define Reference Hardware ID and capture CPU/RAM/GPU/storage/display/DPI, OS/kernel/WebKitGTK/runtime/build profile, power/thermal/network, locale/theme/input/AT, warm-up, and sample protocol.
    - [x] 0.5.2 Capture focused current search/graph/write/startup latency, query shape, CPU/RAM/frame/idle behavior, security exposure, accessible route, and screenshots with explicit known limitations.
    - [x] 0.5.3 Run the ID/fixture/manifest commands and resolve every F0 orphan, schema error, checksum error, and misleading Current/Planned/Unavailable/Unknown claim.
    - [x] 0.5.4 Generate and obtain Spec Owner plus QA/Evidence review for the F0 manifest; record F1 risk owners and serialized heavy-run constraints.

- [x] 1. F1 — SQLite authority, security, lifecycle, and recovery

  **Objective:** Establish the sole SQLite v2 authority, one composition root and governed transaction boundary, fail-closed policy/security, truthful lifecycle/crypto behavior, integrity recovery, and rebuild relay base.
  **Targets (subject to discovery):** adapt current `crates/kria-core/src/memory/{mod,manager,api,contract,modes,lifecycle,governance,observability}.rs`, `write_policy/`, `stores/`, `db/`; new cohesive modules may follow design §3 only after mapping/deleting overlaps; desktop/server/tool integration points.
  **Prerequisites:** signed F0 manifest; schema/authority/security/lifecycle/corruption contracts frozen; remote mode disabled by default.
  **Invariants/non-negotiables:** A1–A12; accepted write is semantic rows + immutable event + audit + outbox + idempotency result + exactly one graph revision when visible, all or none; policy precedes planning/count/rank/cache/serialization; no alternate durable store; no false crypto claim.
  **Implementation steps:** Complete F1.1–F1.9 serially where they touch schema/AuthorityTx; route/cutover work follows passing boundary properties.
  **Failure/degraded behavior:** Unsafe mutation/remote is disabled; authority integrity failure enters read-only Recovery_Mode; derived failure is named Partial; publication failure reconciles from revisions; no failure broadens policy.
  **Focused validation:** V-SCHEMA-01, V-AUTH-01..03, V-POLICY-01..02, V-LIFE-01, V-CRYPTO-01, V-REC-01, initial V-REBUILD-01/V-FAULT-01/V-SBOM-01, V-REG-01.
  **Evidence:** `evidence/F1/<run-id>/{manifest.json,junit/,reports/,security/,traces/,supply-chain/,reviews/}`.
  **Completion proof:** zero bypass/direct writes, partial commits, event mutations, policy leaks, false erasure claims, or insecure remote startup; Recovery_Mode and derived rebuild demonstrations pass.
  **Rollback/containment:** Disable mutation/remote, retain policy-safe local reads, enter Recovery_Mode, or reset/recreate pre-production v2; never restore legacy writer/auth/schema.
  **IDs:** MGR-003–005, MGR-008–009, MGR-017, MGR-028, MGR-032–035, MGR-040–043, MGR-045, MGR-047–048; MGD-006–011, MGD-019–020, MGD-023, MGD-027, MGD-033–038, MGD-041–042.

  - [x] 1.1 F1.1 — Create SQLite authority schema v2 with enforced invariants

    **Objective:** Fresh-create/reset a coherent versioned schema whose database constraints enforce authority, append-only history, canonical encoding, policy presence, and rebuild metadata.
    **Targets (subject to discovery):** `crates/kria-core/src/memory/db/{mod,migrations}.rs`; new next-numbered `db/schema/00NN_memory_graph_v2.sql`; replace conflicting assumptions in `stores/sqlite*.rs` after discovery.
    **Prerequisites:** F0 schema inventory; single migration owner; AuthorityTx types may compile behind tests but no writer cutover yet.
    **Invariants/non-negotiables:** canonical lowercase UUID text; RFC3339 UTC plus source offset; canonical schema-versioned JSON; booleans/checks; FK ON; WAL; `synchronous=FULL` for authority commit; busy timeout; immutable events/revisions/audit; no trigger makes derived index authoritative.
    **Implementation steps:** Execute 1.1.1–1.1.7 in one schema epoch and test fresh-create/reset before integration.
    **Failure/degraded behavior:** Unknown schema/checksum/pragma or failed quick check prevents mutation; malformed legacy data produces reconciliation report then reset/reimport, not dual authority.
    **Focused validation:** V-SCHEMA-01; SQL negative tests; fresh-create/reset; trigger/index inventory; pragma reopen assertions; migration fault injection.
    **Evidence:** `evidence/F1/<run-id>/reports/{schema-inventory,migration-reconciliation,pragma-check}.json`, `junit/V-SCHEMA-01.xml`.
    **Completion proof:** expected tables/indexes/triggers/checksums exactly match golden inventory and invalid rows/mutations fail at the correct boundary.
    **Rollback/containment:** Reset KRIA database and recreate v2; keep application local read-only until schema verifies.
    **IDs:** MGR-017, MGR-032–034, MGR-042, MGR-048; MGD-019, MGD-023, MGD-032, MGD-038, MGD-041; MG-H16, MG-M15.

    - [x] 1.1.1 Add `schema_versions` and singleton `authority_meta` with checksum, schema epoch, HLC, graph revision, and triggers rejecting extra/delete rows.
    - [x] 1.1.2 Add immutable `events` with start/completion/observation phases, typed outcomes, source/invocation/time/policy/checksum/schema fields, exactly-one payload representation, and UPDATE/DELETE abort triggers.
    - [x] 1.1.3 Add `idempotency_results`, append-only `graph_revisions`/`graph_changes`, append-only `audit_records`, and constraints for contiguous base revision and caller-partition key uniqueness.
    - [x] 1.1.4 Add `derived_outbox`, `derived_manifests`, `recovery_snapshots`, `shred_keys`, source/policy/lifecycle base fields, semantic uniqueness keys, retry/dead-letter state, and no secret key bytes.
    - [x] 1.1.5 Add required FK/CHECK/UNIQUE/partial indexes for event source identity, HLC, invocation, policy partitions, revisions, outbox semantics, shred transitions, and startup/query paths.
    - [x] 1.1.6 Assert WAL, foreign keys, synchronous, busy timeout, JSON availability/validation, canonical time/UUID/boolean encodings at every authority connection open.
    - [x] 1.1.7 Implement deterministic fresh-create/hard-reset/reconciliation report and remove any schema migration behavior that leaves two writable authorities.

  - [x] 1.2 F1.2 — Consolidate one memory composition root and typed value objects

    **Objective:** Ensure one initialized authority/policy/query/rebuild graph owns all memory services and adapters receive ports rather than constructing stores independently.
    **Targets (subject to discovery):** adapt `memory/{mod,manager,integration,runtime_backend,runtime_types,ids,error,types,contract}.rs`; composition wiring in desktop/server startup; new `model/` modules only when they replace overlaps.
    **Prerequisites:** F1.1 schema API stable.
    **Invariants/non-negotiables:** One connection/configuration owner; validated IDs/times/policy/truth/mode hashes cannot be raw unchecked strings at boundaries; adapters have no domain SQL.
    **Implementation steps:** Execute 1.2.1–1.2.5 and delete duplicate constructors only after startup tests pass.
    **Failure/degraded behavior:** Initialization error yields typed local unavailable/Recovery state; no fallback store or partially wired service.
    **Focused validation:** composition unit/integration tests, one-database identity assertions, invalid value-object properties, adapter construction tests.
    **Evidence:** `evidence/F1/<run-id>/reports/composition-root.json`, focused JUnit and dependency graph.
    **Completion proof:** repository write/read owners resolve to one composition root and test instrumentation observes one authority identity.
    **Rollback/containment:** Disable memory startup cleanly; do not instantiate old stores independently.
    **IDs:** MGR-017, MGR-020, MGR-032–035, MGR-043; MGD-012, MGD-023, MGD-033, MGD-038; MG-M15, MG-M18.

    - [x] 1.2.1 Map current `MemoryManager`, runtime backend, SQLite stores, policy, lifecycle, retriever, scheduler, and adapter construction; choose one existing root to evolve.
    - [x] 1.2.2 Introduce validated `RecordId`, `EventId`, `InvocationId`, `PolicyPartition`, `GraphRevision`, `UtcTimestamp`, `ValidInterval`, `IdempotencyKey`, and schema/version value objects.
    - [x] 1.2.3 Define narrow authority command/query/outbox/integrity ports in `kria-core`; inject one database handle/configuration and blocking-worker/scheduler services.
    - [x] 1.2.4 Wire desktop and server startup to the same core root while preserving distinct authenticated caller construction at adapter boundaries.
    - [x] 1.2.5 Delete duplicate store constructors/global mutable memory singletons after compile/startup/reference tests prove no live registration remains.

  - [x] 1.3 F1.3 — Implement AuthorityTx, immutable events, idempotency, audit, revisions, changes, and outbox

    **Objective:** Make every accepted durable command atomic, replay-safe, explainable, revisioned once, and projection-reconcilable.
    **Targets (subject to discovery):** adapt/new `memory/authority/{transaction,event_log,idempotency,revision}.rs`, `stores/sqlite_authority.rs`; replace direct transaction logic in current stores/API.
    **Prerequisites:** F1.1–F1.2; serialized writer ownership.
    **Invariants/non-negotiables:** Validate before BEGIN; reserve revision only for graph-visible accepted change; deterministic ordered changes; idempotency result committed in same tx; post-commit publication cannot alter truth; reads never synchronously write access counters.
    **Implementation steps:** Execute 1.3.1–1.3.7 exactly in transaction order.
    **Failure/degraded behavior:** Any pre-commit failure rolls back semantic/event/audit/outbox/idempotency/revision state; post-commit wake failure is recovered from revision/outbox cursor.
    **Focused validation:** V-AUTH-01..03, V-AUTH-02 mutation SQL, ≥100 concurrent replay schedules, crash points before/after each step.
    **Evidence:** `evidence/F1/<run-id>/{junit/V-AUTH-*.xml,traces/authority-crash/,reports/sql-state-hashes.json}`.
    **Completion proof:** all injected failures preserve pre-state; same key/hash returns byte-equivalent result once; different hash conflicts; one visible commit advances one revision.
    **Rollback/containment:** Set Read_Only/Recovery_Mode and reconcile committed outbox/revisions; never replay through direct SQL.
    **IDs:** MGR-005, MGR-008, MGR-033, MGR-035, MGR-042; MGD-010–011, MGD-023, MGD-033, MGD-038; MG-C07, MG-H17.

    - [x] 1.3.1 Define command envelope with caller, canonical command hash, idempotency key, base revision, invocation/source context, mode, deadline, and preview token where required.
    - [x] 1.3.2 Before transaction, validate schema, caller capability, mode, identity, limits, policy inputs, idempotency replay/hash conflict, and destructive-preview freshness.
    - [x] 1.3.3 Begin serialized SQL transaction; append invocation start Event when applicable, then apply semantic mutation using only transaction-scoped repositories.
    - [x] 1.3.4 Append immutable completion/command Event with HLC/UTC/offset/source/checksum/outcome and append accepted/rejected/deferred Audit_Record with reason codes/reversal link.
    - [x] 1.3.5 For graph-visible change, increment `authority_meta` exactly once, append contiguous `graph_revisions`, and append stable-ordinal `graph_changes`; keep non-visible/rejected commands revision-neutral.
    - [x] 1.3.6 Enqueue idempotent target/content/model outbox work and store the canonical result in `idempotency_results` before invariant/FK checks and COMMIT.
    - [x] 1.3.7 Publish only a post-commit wake/cursor; prove lost publication reconnects from revisions/outbox and cannot roll back or duplicate committed truth.

  - [x] 1.4 F1.4 — Implement Effective Policy, source trust, and all Memory Modes

    **Objective:** Enforce a fail-closed restrictive policy meet before writes and every observable read stage, with deterministic Permanent/Temporary/Session_Only/Read_Only/Disabled behavior.
    **Targets (subject to discovery):** adapt `memory/{modes,sensitivity,governance}.rs`, `write_policy/{admission,security,mod}.rs`; new `policy/{effective_policy,source_trust}.rs` only if replacing overlap.
    **Prerequisites:** F1.2–F1.3 command boundary.
    **Invariants/non-negotiables:** Most restrictive contributors; namespace/scope/capability intersection; sensitivity max; no implicit declassification; policy before planning/count/rank/cache/cursor/serialization; ≤2ms p95 policy evaluation excluding commit.
    **Implementation steps:** Execute 1.4.1–1.4.7 and apply one policy result type across ports.
    **Failure/degraded behavior:** Missing/unknown contributor or empty intersection denies; Disabled performs no durable memory read/write; Read_Only preserves authorized reads; no permissive fallback.
    **Focused validation:** V-POLICY-01 properties, V-POLICY-02 paired worlds, mode transition tables, policy performance microbenchmark.
    **Evidence:** `evidence/F1/<run-id>/{reports/policy-properties.json,security/paired-worlds.json,performance/policy.json}`.
    **Completion proof:** associative/commutative/idempotent meet passes ≥100 cases; paired worlds expose identical authorized shape; every mode has exact typed behavior.
    **Rollback/containment:** Force Read_Only or Disabled; preserve strictest known policy and invalidate incompatible cache/cursors.
    **IDs:** MGR-004, MGR-009, MGR-028, MGR-035, MGR-043, MGR-045; MGD-007, MGD-020, MGD-034, MGD-037; MG-C06, MG-H03.

    - [x] 1.4.1 Define source identity/trust/capability context for native, desktop, server, MCP, OpenClaw, sidecar, import, cloud, conversation, library, and tool outcomes.
    - [x] 1.4.2 Implement Effective Policy meet with policy-version/provenance hash and property-test associativity, commutativity, idempotence, monotonic restriction, and deny on empty intersection.
    - [x] 1.4.3 Implement audited declassification as a new governed provenance record; prohibit mutation of contributing source policy.
    - [x] 1.4.4 Implement Permanent, Temporary, Session_Only, Read_Only, and Disabled admission/read/session-purge semantics with typed mode errors and no hidden durable fallback.
    - [x] 1.4.5 Apply policy before SQL/query planning, authorized totals, traversal expansion, ranking, serialization, cache/cursor keys, logs, traces, and renderer DTOs.
    - [x] 1.4.6 Invalidate/discard in-flight responses, pending writes, traces, cursors, and cache entries when caller identity/scope/capability/policy hash changes.
    - [x] 1.4.7 Benchmark deterministic admission at ≥30 warm samples and enforce ≤2ms p95 excluding transaction commit while retaining correctness assertions.

  - [x] 1.5 F1.5 — Route every durable writer through WritePolicyEngine and hard-cut legacy paths

    **Objective:** Eliminate direct writes from native, desktop, server, MCP, OpenClaw, sidecar, import, cognition, tools, goals, feedback, and lifecycle code.
    **Targets (subject to discovery):** all callers found by F0 write inventory; current `memory/api.rs`, `stores/sqlite*.rs`, desktop `commands/{memory,mcp,openclaw}.rs`, server `memory_routes.rs`, tool/sidecar bridges.
    **Prerequisites:** F1.3–F1.4 pass focused authority/policy properties.
    **Invariants/non-negotiables:** Adapters construct caller/command only; one command bus; no direct semantic SQL; no alternate store on unavailable memory; start/completion event pair for invocations.
    **Implementation steps:** Execute 1.5.1–1.5.6 by source class, proving parity before deletion.
    **Failure/degraded behavior:** Unsupported/unavailable source returns typed no-memory/deferred/denied result; it cannot spool authoritative data elsewhere.
    **Focused validation:** per-source integration tests; direct-write inventory/semantic grep plus runtime interception; V-POLICY-01, V-AUTH-01, initial V-TOOL-01.
    **Evidence:** `evidence/F1/<run-id>/reports/write-path-inventory.json`, per-source command traces, JUnit.
    **Completion proof:** runtime instrumentation observes every accepted durable change through AuthorityTx and the linter reports zero bypasses.
    **Rollback/containment:** Disable the affected source’s memory capability; do not restore its direct writer.
    **IDs:** MGR-033, MGR-035, MGR-043–044, MGR-048; MGD-010, MGD-031, MGD-033, MGD-038; MG-C06–C07.

    - [x] 1.5.1 Convert core/native/conversation/library/feedback/goal/cognition writers to typed command candidates and remove store-level public mutation methods not used by AuthorityTx.
    - [x] 1.5.2 Convert Tauri commands to caller-context validation plus command-bus dispatch; remove adapter SQL/domain decisions.
    - [x] 1.5.3 Convert Axum routes to authenticated caller-context validation plus command-bus dispatch; reject unsupported remote mutation capabilities.
    - [x] 1.5.4 Convert MCP/OpenClaw/sidecar/tool invocation start/completion and meaningful outcome writes with source namespace, capability, version, invocation, trust, and policy context.
    - [x] 1.5.5 Convert import/source ingestion/lifecycle maintenance/reconciliation writes without bypassing idempotency, audit, revisions, or outbox.
    - [x] 1.5.6 Delete superseded direct-write functions, legacy transactions, fallback stores, test helpers that bypass policy, and mocks that return simulated write success.

  - [x] 1.6 F1.6 — Close loopback, authentication, authorization, origin, replay, and limit boundaries

    **Objective:** Keep local Tauri operation available while making server mode loopback-safe by default and remote startup fail closed.
    **Targets (subject to discovery):** `crates/kria-server/src/{main,lib,auth,memory_routes,routes}.rs`; config; desktop local API registration; shared core limits/errors.
    **Prerequisites:** F1.2 composition root and F1.4 caller/policy model.
    **Invariants/non-negotiables:** Loopback default; explicit remote enablement; complete identity/authz/restrictive origins/protected transport deployment/replay/rate/payload/deadline/audit config before listen; deny shape leaks no label/ID/count/topology/reason detail.
    **Implementation steps:** Execute 1.6.1–1.6.6 and test startup before binding.
    **Failure/degraded behavior:** Incomplete remote config refuses remote listener while local Tauri remains; malformed/expired/replayed/unauthorized requests receive shape-stable non-revealing deny.
    **Focused validation:** V-POLICY-02, V-XPORT-01 negative matrix, V-FAULT-01 startup/config/oversize/deadline cases.
    **Evidence:** `evidence/F1/<run-id>/security/{remote-startup,negative-matrix,deny-shape,redaction}.json`.
    **Completion proof:** anonymous, wrong-origin, wrong-scope, oversized, replayed, expired, cross-namespace requests reveal zero protected differences across all memory routes.
    **Rollback/containment:** Disable remote listener; retain loopback/auth/policy checks.
    **IDs:** MGR-003–004, MGR-020, MGR-028, MGR-043; MGD-006, MGD-020, MGD-037; MG-C05–C06.

    - [x] 1.6.1 Make bind address loopback by default and require explicit remote profile; validate security configuration atomically before socket accept.
    - [x] 1.6.2 Replace placeholder tokens with validated identity/session/expiry/replay semantics and operation-level grants mapped to CallerContext.
    - [x] 1.6.3 Enforce exact origin policy, protected-transport deployment requirement, request/body/rate/concurrency/deadline limits, and audit correlation.
    - [x] 1.6.4 Normalize deny envelopes/status/timing/length sufficiently to avoid protected reason/count/topology distinctions while retaining local correlation IDs.
    - [x] 1.6.5 Apply security to search, graph, path, prediction, trace, patch/SSE replay, command, lifecycle, source, health, and unsupported local-only routes.
    - [x] 1.6.6 Prove remote failure leaves Tauri local healthy and no route registration bypasses the common authentication/authorization middleware.

  - [x] 1.7 F1.7 — Implement governed forget, restore, hard delete, and honest crypto state

    **Objective:** Provide revision-bound lifecycle previews, reversible 30-day forgetting, idempotent permanent deletion reconciliation, and cryptographic wording that matches actual unreadability.
    **Targets (subject to discovery):** adapt `memory/lifecycle.rs`; new cohesive `lifecycle/{preview,forget,delete,crypto}.rs` only if replacing it; authority/outbox stores; adapter contracts and truthful minimal Health copy.
    **Prerequisites:** F1.3–F1.5.
    **Invariants/non-negotiables:** Same ID on restore; Forgotten excluded by default; hard-delete authority state wins even if purge lags; zero deleted content after reconciliation; Crypto-Shredded unavailable unless subject-bound external key destruction denies all plaintext paths.
    **Implementation steps:** Execute 1.7.1–1.7.7; keep application encryption optional but claims mandatory/honest.
    **Failure/degraded behavior:** Stale preview conflicts; post-commit purge failure remains Deleted with retry; absent crypto proof displays Hard Delete pending cryptographic erasure/OS-disk-encryption reliance.
    **Focused validation:** V-LIFE-01, V-CRYPTO-01, V-AUTH-01 fault points, policy-safe dependency preview tests.
    **Evidence:** `evidence/F1/<run-id>/{reports/lifecycle-residue.json,security/crypto-truth.json,traces/delete-reconciliation/}`.
    **Completion proof:** forget/restore identity and 30-day boundaries pass; interrupted purge converges; no UI/API path says Crypto-Shredded without passing plaintext-denial review.
    **Rollback/containment:** Disable destructive commit while preserving preview/forget/read; continue reconciliation; never relabel deletion as crypto erasure.
    **IDs:** MGR-017, MGR-040–041, MGR-045; MGD-027, MGD-036, MGD-041; MG-L02, MG-L11, MG-O10–O11.

    - [x] 1.7.1 Implement lifecycle preview over authorized dependencies, independent evidence, affected sources/scopes, cascade/keep choices, reversibility, base revision, and bounded 500/5000 limits.
    - [x] 1.7.2 Implement Forget as governed Truth_State transition with `restore_until=now+30d`, audit/change/outbox, and exclusion from default retrieval/query projections.
    - [x] 1.7.3 Implement Restore within window using the same stable ID and governed active/truth transition; reject expired/stale/unauthorized restore without mutation.
    - [x] 1.7.4 Implement Hard Delete commit that marks content Deleted, closes dependent relationships/links, records cascade choices, and enqueues FTS/vector/graph/trace/cache/export purge work.
    - [x] 1.7.5 Implement idempotent reconciliation and residue checks so failed/interrupted projection purge resumes and no deleted content is returned while purge is pending.
    - [x] 1.7.6 Model shred-key ID/version/status/destruction proof without storing secret bytes; expose crypto capability as unavailable until real payload encryption/key storage/destruction exists.
    - [x] 1.7.7 If encryption is implemented, prove destroyed-key denial through current/history/snapshot/cache/index paths and complete threat/key/backup/license review; otherwise enforce pending-erasure wording everywhere.

  - [x] 1.8 F1.8 — Implement integrity classification, Recovery_Mode, relay, and rebuild base

    **Objective:** Distinguish authority corruption from disposable projection corruption, prevent unsafe writes, and provide resumable deterministic convergence.
    **Targets (subject to discovery):** new/adapted `memory/authority/{integrity,recovery}.rs`, `stores/rebuild.rs`, `maintenance.rs`, `jobs.rs`, observability/Health ports.
    **Prerequisites:** F1.1 schema, F1.3 outbox/revisions, F1.7 lifecycle states.
    **Invariants/non-negotiables:** Authority corruption → fail-closed read-only Recovery_Mode; derived corruption → named Partial and isolated generation rebuild; rebuild never changes semantic authority/event/revision hashes; only verified restore/import exits Recovery_Mode.
    **Implementation steps:** Execute 1.8.1–1.8.7 with corruption fixtures and interruption points.
    **Failure/degraded behavior:** Diagnostic exposes corruption class/correlation ID only; invalid snapshot/import remains Recovery_Mode; dead-letter preserves reconciliation eligibility.
    **Focused validation:** V-REC-01, initial V-REBUILD-01, V-FAULT-01; startup quick/integrity/checksum/order/outbox cursor tests.
    **Evidence:** `evidence/F1/<run-id>/{reports/integrity.json,traces/recovery/,traces/rebuild/,reports/authority-hashes.json}`.
    **Completion proof:** authority corruption blocks writes; each derived projection can be dropped/rebuilt to equivalent manifest; interrupted runs resume/discard deterministically.
    **Rollback/containment:** Stay in Recovery_Mode or disable affected projection; reset/reimport verified pre-production data if authority cannot recover.
    **IDs:** MGR-017, MGR-032, MGR-042, MGR-045; MGD-020, MGD-041; MG-H16, MG-L02, MG-L11.

    - [x] 1.8.1 On startup assert schema checksums/pragmas and run `quick_check`, event checksum/HLC order, graph revision continuity, authority singleton, and outbox cursor sanity.
    - [x] 1.8.2 Add release/recovery `integrity_check`, full event/order checks, migration checksum, derived manifest membership/version comparison, and policy-safe fault classification.
    - [x] 1.8.3 Implement Recovery_Mode state machine with no durable command execution, bounded diagnostics, local verified restore/import actions, and explicit exit re-open verification.
    - [x] 1.8.4 Implement derived relay leasing/retry/backoff/dead-letter with semantic target/content/model idempotency and deletion precedence.
    - [x] 1.8.5 Implement temporary-generation rebuild in authority revision order with durable cursor, policy-authorized stream, interrupt resume/discard, member count/hash/version comparison, then atomic activation.
    - [x] 1.8.6 Add reconciliation for missing/orphan/version-mismatched FTS/vector/graph entries without semantic authority mutation.
    - [x] 1.8.7 Inject authority page/schema/event/revision corruption and isolated FTS/vector manifest corruption; assert exact Recovery versus Partial behavior.

  - [x] 1.9 F1.9 — Prove F1 properties and perform the authority/security hard cutover

    **Objective:** Aggregate focused evidence, remove old authority/security paths, and issue the F1 predecessor manifest.
    **Targets (subject to discovery):** F1 test modules under core/server/desktop/eval; legacy stores/routes/schema registrations identified in F0; evidence manifests/reviews.
    **Prerequisites:** F1.1–F1.8 focused tests pass.
    **Invariants/non-negotiables:** Cutover is hard, not dual-write; security rollback is forward-only; no P0 waiver; generated Evidence Artifacts—not boxes—determine gate status.
    **Implementation steps:** Execute 1.9.1–1.9.5.
    **Failure/degraded behavior:** Any leak/partial commit/event mutation/false crypto/integrity ambiguity blocks F2 and leaves mutation/remote disabled.
    **Focused validation:** all F1 suites plus targeted regressions and manifest validation.
    **Evidence:** complete `evidence/F1/<run-id>/` tree and Backend/Security-Privacy/Data-Integrity reviews.
    **Completion proof:** F1 manifest Pass with F0 predecessor hash; live registrations contain only v2 authority/write/security paths.
    **Rollback/containment:** Read-only local authority or Recovery_Mode; clean v2 reset; never revive removed bypass.
    **IDs:** All F1 requirement/decision IDs; V-AUTH-01..03, V-SCHEMA-01, V-POLICY-01..02, V-LIFE-01, V-CRYPTO-01, V-REC-01, V-REBUILD-01, V-FAULT-01, V-SBOM-01.

    - [x] 1.9.1 Run ≥100-case authority/idempotency/policy properties and persist exact seeds/minimized counterexamples for any failure.
    - [x] 1.9.2 Run server negative matrix, paired-world non-interference, lifecycle residue, crypto wording, corruption/recovery, rebuild interruption, and async/fault slices with correctness hashes.
    - [x] 1.9.3 Re-run write/read/route/schema registration inventories and delete legacy schema writers, direct graph/memory writes, permissive routes, duplicate roots, obsolete ANN authority assumptions, and simulated-success tests.
    - [x] 1.9.4 Generate checksummed F1 artifacts and obtain independent Backend, Security/Privacy, and Data Integrity reviews; record unresolved nonblocking work only in later mapped tasks.
    - [x] 1.9.5 Validate F1 manifest and predecessor chain; keep all task boxes unchecked until implementation and evidence are both actually present.

- [x] 2. F2 — Semantic records, Memory Links, entities, truth, time, sources, and interchange base

  **Objective:** Build every typed cognitive record and provenance contract, canonical governed Memory Links, mixed graph projection, dual time/truth maintenance, conservative entity resolution, consented sources, and open interchange foundation on the F1 authority boundary.
  **Targets (subject to discovery):** adapt `memory/{types,truth,entity_resolution,extraction,merge,goals,feedback,conversation,library,graph_intel}.rs`; v2 schema extension; new `model/` and `graph/` modules only by replacing overlap; shared API fixtures.
  **Prerequisites:** signed F1 manifest; same AuthorityTx/policy/recovery root; relation/record version registry frozen.
  **Invariants/non-negotiables:** Every type has stable ID/schema/source/actor/time/policy/truth/provenance; Memory Links are the only semantic-link model; generated navigation is never authority; graph traversal is cycle-safe/policy-first/≤3 hops; Valid and Transaction Time are independent; names/embeddings never auto-merge persons.
  **Implementation steps:** Complete F2.1–F2.7; serialize shared schema/DTO changes through one owner.
  **Failure/degraded behavior:** Unknown required semantics deny writes; unavailable endpoints/authority fields are omitted or Unavailable; stale previews conflict; source cancellation commits no partial semantic record.
  **Focused validation:** V-SEM-01, V-TRUTH-01, V-ENTITY-01, V-GRAPH-01 base, V-IO-01 base, V-POLICY-01, source consent/cascade cases, V-REG-01.
  **Evidence:** `evidence/F2/<run-id>/{manifest.json,junit/,reports/,fixtures/,traces/,reviews/}`.
  **Completion proof:** canonical round trips/lineage/time/entity/source properties pass; no parallel links, generated topology, policy broadening, name-only merges, or lost provenance.
  **Rollback/containment:** Disable affected semantic command/source; retain authority read-only; governed reversal for merges/corrections; reset/reimport rather than parallel schema.
  **IDs:** MGR-001–002, MGR-005, MGR-007, MGR-010–012, MGR-018–019, MGR-024, MGR-032, MGR-034, MGR-037, MGR-046, MGR-048; MGD-001, MGD-003–005, MGD-008–009, MGD-019, MGD-032, MGD-040, MGD-042, MGD-044.

  - [x] 2.1 F2.1 — Add all typed cognitive records and complete provenance

    **Objective:** Represent Event, Memory, Entity, Alias, Mention, Relationship, Evidence, Goal, Episode, Summary, Skill, Rule, Retrieval Trace, Feedback, Audit, Source, and Tool Observation as explicit versioned authority types.
    **Targets (subject to discovery):** v2 schema migration; adapt `memory/{types,runtime_types,contract,goals,feedback,conversation,library,extraction}.rs`; new `model/{record,provenance,truth,temporal}.rs` if it replaces duplicated definitions.
    **Prerequisites:** F1 AuthorityTx/schema/policy.
    **Invariants/non-negotiables:** Stable IDs; source/actor/creation time; Effective Policy; Truth State; Valid Time where applicable; immediate parents/method/version/time for derivations; unknown raw enums retained for diagnostics and denied for writes.
    **Implementation steps:** Execute 2.1.1–2.1.6 and route creation only through AuthorityTx.
    **Failure/degraded behavior:** Missing required provenance/policy rejects before commit; unknown optional read fields survive round trip; malformed derived rows are isolated from projection.
    **Focused validation:** V-SEM-01 serde/SQL round trips and ≥100 generated records; V-SCHEMA-01 extension checks; unknown-version negative tests.
    **Evidence:** `evidence/F2/<run-id>/{junit/V-SEM-01.xml,reports/record-roundtrip.json,reports/provenance-completeness.json}`.
    **Completion proof:** every supported type preserves every semantically significant field through SQL and contract serialization.
    **Rollback/containment:** Disable creation of malformed type/version; retain readable diagnostics without unsafe mutation.
    **IDs:** MGR-001, MGR-002, MGR-034, MGR-037–039; MGD-001, MGD-032; MG-C02, MG-H13, MG-M14, MG-L01.

    - [x] 2.1.1 Add/validate authority tables and Rust enums/structs for records, entities, aliases, mentions, evidence, episodes, goals/progress, consolidation runs, sources, tool observations, traces/items, and feedback.
    - [x] 2.1.2 Encode provenance source, actor, method/version/time, immediate parents, policy-safe structured locator, model/algorithm identity, and creation Event on every applicable type.
    - [x] 2.1.3 Implement canonical content hash, estimated token, staleness, truth, lifecycle, valid interval, supersession, episode, and goal-context fields with indexes/checks.
    - [x] 2.1.4 Preserve unknown optional fields/raw enum values for read diagnostics/interchange while rejecting commands that depend on unknown required semantics.
    - [x] 2.1.5 Add SQL↔Rust↔API property round trips for all record kinds, extreme Unicode/empty optional fields/time boundaries, and malformed row isolation.
    - [x] 2.1.6 Remove or adapt duplicate legacy fact/memory/entity structs so one canonical v2 model feeds graph, retrieval, API, and UI contracts.

  - [x] 2.2 F2.2 — Make relation registry and Memory Links canonical

    **Objective:** Replace free-text/parallel relationships with registry-governed semantic identity, multi-evidence links, endpoint rules, and canonical required link types.
    **Targets (subject to discovery):** v2 `relation_registry`, `relation_aliases`, `relationships`, `evidence`, `memory_links`; adapt `sqlite_graph.rs`, `graph_intel.rs`, `api.rs`, UI/API contract fixtures.
    **Prerequisites:** F2.1 typed endpoints/provenance.
    **Invariants/non-negotiables:** Required `derived_from`, `supports`, `contradicts`, `mentions_entity`, `superseded_by`; versioned direction/inverse/reflexivity/endpoint/evidence/validity rules; duplicate observation appends evidence, not active edge; mixed endpoint existence checked in AuthorityTx.
    **Implementation steps:** Execute 2.2.1–2.2.7 and hard-cut free-text paths only after round-trip evidence.
    **Failure/degraded behavior:** Unknown registry version, invalid endpoints, forbidden reflexivity/direction/time/evidence/policy, or stale base revision rejects atomically.
    **Focused validation:** V-SEM-01 relation identity properties; V-AUTH-01 relationship faults; V-TRUTH-01 evidence polarity/time cases.
    **Evidence:** `evidence/F2/<run-id>/{reports/relation-registry.json,reports/link-properties.json,junit/V-SEM-01.xml}`.
    **Completion proof:** semantic uniqueness and endpoint/direction/evidence properties pass; repository inventory finds no independent semantic-link authority.
    **Rollback/containment:** Disable relation mutation and retain canonical reads; governed compensating action for committed changes.
    **IDs:** MGR-005, MGR-018, MGR-034, MGR-037; MGD-008–010, MGD-040; MG-C07, MG-H08, MG-M05, MG-M12.

    - [x] 2.2.1 Seed/version registry rows with forward/inverse labels, aliases, directed/symmetric class, inverse name, reflexivity, endpoint kinds, validity/evidence/policy rules, and writable disposition.
    - [x] 2.2.2 Implement canonical semantic identity hash normalizing symmetric endpoints but preserving directed orientation, registry version, validity identity, and policy partition.
    - [x] 2.2.3 Validate polymorphic endpoint existence/kinds, canonical entity IDs, relation alias resolution, direction, reflexivity, Valid Time, Evidence, capability, and Effective Policy inside AuthorityTx.
    - [x] 2.2.4 Append support/contradiction Evidence with locator/actor/method/version/polarity/score semantics/policy; never duplicate an active semantic edge for another observation.
    - [x] 2.2.5 Implement governed create/edit/confirm/expire/delete/restore/undo as revision-bound commands with compensating history, not mutation erasure.
    - [x] 2.2.6 Implement migration/reset reconciliation from legacy free-text relationships with deterministic reports and explicit rejects for ambiguous mappings.
    - [x] 2.2.7 Delete legacy relationship tables/writers/DTO assumptions/tests after canonical link round-trip, uniqueness, and reversal evidence passes.

  - [x] 2.3 F2.3 — Implement policy-safe mixed projection and bounded graph traversal

    **Objective:** Produce one entity-primary typed graph contract and cycle-safe ≤3-hop projection without exposing hidden endpoints/intermediaries/frontiers.
    **Targets (subject to discovery):** adapt `graph_intel.rs`, `stores/sqlite_graph.rs`; new `graph/{projection,query,traversal}.rs`; contract fixtures in core/eval.
    **Prerequisites:** F2.1–F2.2.
    **Invariants/non-negotiables:** Node kinds entity/memory/evidence/source/aggregate; authority classes stored/derived/inferred/navigation; one revision/policy/truth/time/provenance per item; no repeated path node; entity-primary unless explicit expansion; hidden intermediary removes whole path.
    **Implementation steps:** Execute 2.3.1–2.3.6; API envelope publication waits for F3.
    **Failure/degraded behavior:** Unavailable endpoint edge omitted or policy-safe aggregate frontier with no hidden ID/count/topology; cap/deadline returns typed truncation, never unbounded fallback.
    **Focused validation:** V-GRAPH-01 cycle/hidden-world properties; V-SEM-01 projection goldens; query-plan assertions for batched endpoint/evidence reads.
    **Evidence:** `evidence/F2/<run-id>/{reports/graph-properties.json,reports/query-plans.json,traces/cyclic-fixtures/}`.
    **Completion proof:** deterministic cyclic and paired-scope fixtures terminate ≤3 hops, repeat no path node, and leak no hidden graph fact.
    **Rollback/containment:** Disable expansion/path and expose bounded entity/list records; never load or reveal full adjacency.
    **IDs:** MGR-002, MGR-004, MGR-007, MGR-009, MGR-012, MGR-023; MGD-003–004, MGD-015; MG-C03, MG-H09, MG-M04, MG-M16–M17.

    - [x] 2.3.1 Define canonical projection DTOs with stable IDs, kind, authority class, Graph Revision, Effective Policy summary, Truth State, Valid Time, provenance summary, typed metadata, endpoint summaries, and authorized actions placeholder.
    - [x] 2.3.2 Implement entity-primary query projection; require explicit bounded expansion for memory/evidence/source nodes and label generated facets as navigation containers.
    - [x] 2.3.3 Implement breadth-first and path traversal with max three hops, stable visited/path guards, per-hop/item/edge/deadline caps, batched endpoint/evidence reads, and deterministic ordering.
    - [x] 2.3.4 Filter policy before every seed/expansion/count/frontier operation; omit any path with hidden intermediary and expose no hidden stable identifier.
    - [x] 2.3.5 Return authorized truncation reason/frontier token and endpoint-complete edges; never use UUID slices as human labels.
    - [x] 2.3.6 Add cyclic, self-loop, parallel-evidence, depth-0/1/2/3/4, hidden intermediary, mixed endpoint, cancellation, and deadline property cases.

  - [x] 2.4 F2.4 — Implement Valid/Transaction Time, contradiction, supersession, and correction lineage

    **Objective:** Centralize current/historical truth semantics and preserve disagreements, source evidence, corrections, and reversibility without presenting stale claims as current.
    **Targets (subject to discovery):** adapt `memory/truth.rs`, `causal.rs`, `merge.rs`; new `model/{truth,temporal}.rs`; link/record query predicates.
    **Prerequisites:** F2.1–F2.3.
    **Invariants/non-negotiables:** Valid Time independent of transaction revision; one centralized active-validity predicate; precedence user-confirmed → verification recency → independent evidence quality → statistically significant Memory Worth; unresolved ties preserve both; superseded excluded from default current reads.
    **Implementation steps:** Execute 2.4.1–2.4.6.
    **Failure/degraded behavior:** Verification unavailable marks Unverified/Stale and preserves last verified value/time; absent authority displays Unavailable/omits claim; never infer current state from latest transaction alone.
    **Focused validation:** V-TRUTH-01 temporal boundaries/timezones/revisions; contradiction/supersession/correction goldens; relation parity cases.
    **Evidence:** `evidence/F2/<run-id>/{reports/truth-goldens.json,reports/temporal-properties.json,traces/correction-lineage/}`.
    **Completion proof:** open/exact boundaries, timezone conversion, expiry, supersession, contradiction, correction, and revision interaction match independent expected answers.
    **Rollback/containment:** Mark affected capability Unavailable; preserve competing records and history rather than select an uncertain truth.
    **IDs:** MGR-001, MGR-010–011, MGR-018, MGR-024, MGR-037; MGD-001, MGD-005, MGD-017; MG-H05, MG-H07, MG-M07–M08, MG-O09–O11.

    - [x] 2.4.1 Implement one active predicate for current records/links using valid interval, truth/lifecycle, supersession, policy, and requested transaction snapshot.
    - [x] 2.4.2 Implement historical instant/range evaluation independent of transaction revision and include timezone/source-time metadata in results.
    - [x] 2.4.3 Implement deterministic contradiction evaluation and unresolved conflict preservation with explicit Evidence polarity and precedence explanation.
    - [x] 2.4.4 Implement supersession with predecessor preservation, `superseded_by` link, closed Valid Time where applicable, default-current exclusion, and dependent invalidation.
    - [x] 2.4.5 Implement user correction/confirm/supersede/keep-both as previewed governed commands preserving before/after, evidence, decision, audit, reversal, and base revision.
    - [x] 2.4.6 Rename connected-component outputs to `component`; gate community/centrality output on named algorithm/version/parameters/predicate/revision/quality and invalidate comparability on changes.

  - [x] 2.5 F2.5 — Implement conservative entity resolution, merge, split, and reversal

    **Objective:** Preserve mention provenance and allow only strong-identifier automatic resolution; make all ambiguous identity decisions previewed, policy-safe, and reversible.
    **Targets (subject to discovery):** adapt `entity_resolution.rs`, `extraction.rs`, `merge.rs`; entity proposal/action schema and graph links.
    **Prerequisites:** F2.1 provenance, F2.2 links, F2.4 correction/time.
    **Invariants/non-negotiables:** Names/fuzzy text/embedding only propose; strong exact typed identifiers may resolve; merge never broadens policy; stale preview cannot commit; split/reversal restores exact memberships/links/history.
    **Implementation steps:** Execute 2.5.1–2.5.6.
    **Failure/degraded behavior:** Ambiguity remains Unresolved; policy mismatch denies; stale/reversal-expired action returns typed conflict without mutation.
    **Focused validation:** V-ENTITY-01 properties and round-trip hashes; V-POLICY-01 merge meet; graph/link integrity after split.
    **Evidence:** `evidence/F2/<run-id>/{reports/entity-resolution.json,traces/merge-split/,junit/V-ENTITY-01.xml}`.
    **Completion proof:** name-only auto-merge is impossible and accepted merge→split restores exact canonical partitions and links with audit retained.
    **Rollback/containment:** Disable automatic matching/proposals; use governed reversal/reset for committed pre-production mistakes.
    **IDs:** MGR-019, MGR-024, MGR-034; MGD-001, MGD-007; MG-M13–M14, MG-O03, MG-O12.

    - [x] 2.5.1 Implement type-specific Unicode/name, email, URL, repository, and path normalization with explicit strong/weak identifier classification.
    - [x] 2.5.2 Append mention locator/span/role/extractor/version/score-semantics provenance even when a strong identifier resolves to an existing canonical entity.
    - [x] 2.5.3 Create unresolved proposals for name/fuzzy/vector similarity with feature version, rationale, policy, base revision, and no topology mutation.
    - [x] 2.5.4 Implement merge preview showing canonical choice, aliases, mentions, links, evidence, policy meet, affected count, conflicts, reversibility, and stale-token behavior.
    - [x] 2.5.5 Implement accepted/rejected/reversed actions through AuthorityTx, preserving before/after and correcting canonical endpoints without losing evidence.
    - [x] 2.5.6 Implement split/reversal reconstruction and properties over multi-scope aliases, duplicate mentions, link direction, superseded records, and concurrent revision drift.

  - [x] 2.6 F2.6 — Implement consent-gated source ingestion and source lifecycle

    **Objective:** Make filesystem/repository/shell-history/library/import ingestion explicit, previewable, bounded, resumable, deduplicated, fenced, and deletable.
    **Targets (subject to discovery):** adapt `memory/{library,cold_start,cold_start_scan,extraction}.rs`, sidecar/import bridges, source schema/API contracts; no UI polish before F4.
    **Prerequisites:** F1 policy/lifecycle; F2.1 records; F2.4 truth.
    **Invariants/non-negotiables:** No consent means no scan; ≤1MiB chunks; each semantic write governed; interruption commits no partial semantic record; content is data and cannot invoke actions; source deletion uses lifecycle preview.
    **Implementation steps:** Execute 2.6.1–2.6.6.
    **Failure/degraded behavior:** Cancel within current bounded unit and preserve resumable cursor; secret/injection-shaped content is rejected, restricted, or fenced with reason; duplicate follows deterministic identity/version policy.
    **Focused validation:** source consent/cancel/dedup/injection/cascade cases in V-SEM-01, V-IO-01, V-FAULT-01.
    **Evidence:** `evidence/F2/<run-id>/{reports/source-ingestion.json,traces/source-cancel/,security/content-fencing.json}`.
    **Completion proof:** no-consent produces zero scan/write; cancel/resume is deterministic; duplicate and source-delete outcomes match expected manifests.
    **Rollback/containment:** Disable source adapter and preserve manual onboarding/local authority; never bypass consent to recover throughput.
    **IDs:** MGR-004, MGR-040, MGR-043, MGR-046; MGD-007, MGD-034; MG-L06, MG-O03, MG-O28.

    - [x] 2.6.1 Define source identity/version/trust/policy/consent/lifecycle/cursor states for native, MCP, OpenClaw, sidecar, import, library, conversation, filesystem, repository, and shell history.
    - [x] 2.6.2 Implement consent check before discovery and candidate preview with exclude/approve/manual onboarding semantics.
    - [x] 2.6.3 Stream bounded chunks, compute content/item/version hashes, preserve structured locators, and submit each complete semantic candidate through WritePolicyEngine.
    - [x] 2.6.4 Implement deterministic duplicate reuse/versioning and idempotent source event identities across retries.
    - [x] 2.6.5 Fence untrusted content, scan injection/secret sensitivity, prevent text-to-action interpretation, and propagate restrictive policy through derivatives.
    - [x] 2.6.6 Implement cancel/resume and source deletion dependency preview/cascade/keep-independent-evidence behavior with fault injection at every chunk boundary.

  - [x] 2.7 F2.7 — Establish interchange base, contract fixtures, semantic cutover, and F2 evidence

    **Objective:** Freeze open canonical interchange semantics and API golden fixtures, then remove parallel semantic models and issue F2 evidence.
    **Targets (subject to discovery):** `model/interchange.rs`, import/export authority commands, `mg-interchange-v2`, core/API golden fixtures, legacy graph/link/entity paths.
    **Prerequisites:** F2.1–F2.6.
    **Invariants/non-negotiables:** Canonical JSON manifest + content files; selected authorized records/events/links/provenance/truth/lifecycle/order/version/checksums; whole package validates before one idempotent commit; unknown optional retained; unknown required rejects with zero writes.
    **Implementation steps:** Execute 2.7.1–2.7.6.
    **Failure/degraded behavior:** Tamper/quota/unknown-required/policy error commits zero rows and returns bounded report; export excludes unauthorized secrets.
    **Focused validation:** V-IO-01 base, V-SEM-01, V-TRUTH-01, V-ENTITY-01, V-GRAPH-01, schema/serialization golden tests.
    **Evidence:** complete `evidence/F2/<run-id>/` plus Domain and Security/Privacy reviews.
    **Completion proof:** export→empty import→export preserves semantic IDs/order/links/provenance/state/unknown optional fields; no legacy semantic authority remains live.
    **Rollback/containment:** Keep interchange/source operation disabled; retain canonical authority reads and clean reset/reimport capability.
    **IDs:** MGR-002, MGR-018–019, MGR-032, MGR-034, MGR-046, MGR-048; MGD-019, MGD-032, MGD-040, MGD-042, MGD-044.

    - [x] 2.7.1 Define Interchange v1 canonical manifest, ordering, checksums, schema/ontology/relation/algorithm/model versions, selected scope, extension preservation, and secret exclusion rules.
    - [x] 2.7.2 Implement policy-selected streaming export with deterministic order and independent parser validation.
    - [x] 2.7.3 Implement whole-manifest/limits/checksum/policy/required-semantics validation before one idempotent AuthorityTx import.
    - [x] 2.7.4 Produce semantic/API fixtures for every record/link/truth/time/entity/source state, unknown values, malformed rows, cyclic graph, and policy-paired world.
    - [x] 2.7.5 Delete free-text links, duplicate entity/graph DTOs, generated-topology authority, and obsolete tests after round-trip/reversal evidence.
    - [x] 2.7.6 Run all F2 suites, generate manifest, and obtain Domain plus Security/Privacy reviews with signed F1 predecessor hash.

- [x] 3. F3 — Five-strategy retrieval, goals, cognition, canonical API, and backend gates

  **Objective:** Deliver exact FTS5/vector/graph/temporal/goal retrieval, deterministic classification and adaptive RRF, trace-finalized context, bounded cognition, canonical v2 API/patches, host parity, and 200-query/100k backend proof.
  **Targets (subject to discovery):** adapt `memory/{retriever,retrieval_opt,embedding,embeddings,goals,cognition,scheduler,jobs,active_learning,api,contract,observability}.rs`, stores and eval harness; desktop/server memory adapters; delete superseded APIs after evidence.
  **Prerequisites:** signed F2 manifest; exact model/license disposition approved for testing; judged corpus and 100k generator hashes frozen.
  **Invariants/non-negotiables:** SQLiteVectorStore exact cosine, no ANN current release; FTS5 offline floor; policy/truth/version gates before candidates/fusion; graph ≤3 hops; no silent weight redistribution; Used equals exact injected order; bounded workers/deadlines/queues; one canonical `memory.v2` contract.
  **Implementation steps:** Complete F3.1–F3.9; serialize heavy model/100k/evaluation/performance/corruption runs.
  **Failure/degraded behavior:** Unavailable strategy is named Partial; FTS5 remains safe floor; bad vector partition rejected; cognition pauses; API limit error never unbounds; patch gaps bounded-refetch; no full graph reload.
  **Focused validation:** V-VECTOR-01, V-RET-01..03, V-GRAPH-01, V-CONS-01, V-TOOL-01, V-XPORT-01, V-PERF-01, V-FAULT-01, V-REBUILD-01, V-REG-01.
  **Evidence:** `evidence/F3/<run-id>/{manifest.json,reports/,traces/,performance/,security/,junit/,reviews/}`.
  **Completion proof:** judged thresholds and 100k budgets pass; no >50ms async blocking span; foreground preemption ≤100ms; Tauri/Axum normalized parity; only v2 public memory/graph API remains.
  **Rollback/containment:** Disable failed strategy/cognition/patch streaming and retain FTS5 plus bounded v2 refetch; never broaden policy, mutate weights online, or restore full-graph API.
  **IDs:** MGR-006–009, MGR-020, MGR-023, MGR-025, MGR-028, MGR-032, MGR-036, MGR-038–039, MGR-042–045, MGR-048; MGD-012, MGD-015, MGD-017, MGD-024–025, MGD-031, MGD-033, MGD-039, MGD-043.

  - [x] 3.1 F3.1 — Implement exact SQLiteVectorStore and pinned MiniLM partition contract

    **Objective:** Replace ambiguous/ANN behavior with a rebuildable exact, policy-prefiltered 384d cosine implementation behind VectorStorePort.
    **Targets (subject to discovery):** adapt `stores/{ports,sqlite_vectors,ann_vectors}.rs`, `embedding.rs`/`embeddings.rs`; model manifests; eval vector oracle.
    **Prerequisites:** F2 records/policy/outbox; reviewed test-use license disposition.
    **Invariants/non-negotiables:** FastEmbed `all-MiniLM-L6-v2`; exact source revision/artifact/tokenizer/runtime/license; dim 384; finite `f32le`, exactly 1536 bytes, L2 normalization contract; f64 cosine accumulation; stable score-desc then ID; no LanceDB/Qdrant/HNSW/ANN dependency.
    **Implementation steps:** Execute 3.1.1–3.1.6.
    **Failure/degraded behavior:** Wrong hash/model/dimension/bytes/NaN/Inf/zero norm rejects partition/query; retrieval continues without vector as Partial.
    **Focused validation:** V-VECTOR-01 independent scalar oracle over ties/non-normalized/error vectors and 100k compatible rows; rebuild interruption.
    **Evidence:** `evidence/F3/<run-id>/{reports/vector-oracle.json,reports/model-manifest.json,traces/vector-rebuild/}`.
    **Completion proof:** every hit exactly matches independent f64 cosine membership/order within declared numeric tolerance; manifest and partition checks pass.
    **Rollback/containment:** Disable vector partition and purge/rebuild derived rows; retain FTS5/graph/time/goal.
    **IDs:** MGR-032, MGR-036, MGR-042, MGR-045, MGR-047; MGD-024, MGD-039; MG-H01, MG-M15.

    - [x] 3.1.1 Pin canonical model ID/source revision, artifact/tokenizer checksums, reviewed license disposition ID, FastEmbed/runtime versions, max tokens, pooling, dimension, dtype, and normalization in build/runtime manifest.
    - [x] 3.1.2 Implement `embedding_partitions` validation and `mem_vectors` constraints/indexes keyed by partition, policy, truth, content hash, and revision.
    - [x] 3.1.3 Decode exactly 384 little-endian finite f32 values; reject malformed lengths, NaN/Inf, zero norm, incompatible content/model/tokenizer/dimension.
    - [x] 3.1.4 SQL-prefilter compatible policy/truth/valid-time partition rows on a blocking worker, calculate exact cosine with f64 accumulation, and stable-sort score descending then record ID.
    - [x] 3.1.5 Implement upsert/delete/manifest/rebuild with authority outbox semantics, temporary generation, model migration cursor, and deterministic membership hash.
    - [x] 3.1.6 Remove current-release ANN code/dependency/feature registration or isolate it entirely outside release closure after exact-store parity evidence.

  - [x] 3.2 F3.2 — Implement FTS5 full-corpus search and offline floor

    **Objective:** Search authorized record content, entities/aliases, source metadata, goals, and relation labels with parameterized FTS5 and honest rank semantics.
    **Targets (subject to discovery):** adapt `stores/sqlite_search.rs`, schema FTS objects/triggers, `retriever.rs`; evaluation fixtures.
    **Prerequisites:** F2 semantics and F1 outbox/rebuild.
    **Invariants/non-negotiables:** External-content FTS5; `unicode61 remove_diacritics 2`, prefix 2/3/4; policy/truth/time preselection; user text never SQL; BM25 is Relative score only; rebuildable membership hash.
    **Implementation steps:** Execute 3.2.1–3.2.6.
    **Failure/degraded behavior:** FTS corruption marks Recall Partial and permits exact metadata reads/rebuild; no false empty authority claim; query syntax errors are typed InvalidRequest.
    **Focused validation:** V-RET-01 FTS slice, V-REBUILD-01 FTS generation, injection/Unicode/phrase/field/filter tests, query plans.
    **Evidence:** `evidence/F3/<run-id>/{reports/fts-contract.json,reports/fts-query-plans.json,traces/fts-rebuild/}`.
    **Completion proof:** independent full-corpus expected memberships, fields/rationales, authorized totals, and rebuild hashes match.
    **Rollback/containment:** Disable Recall ranking, expose exact authorized metadata/list reads and rebuild status; never broad LIKE scan beyond limits.
    **IDs:** MGR-006, MGR-009, MGR-036, MGR-042, MGR-045; MGD-015, MGD-025; MG-H01, MG-H04.

    - [x] 3.2.1 Build `search_documents` projection over memory/summary/skill/rule, entity names/aliases, source metadata, goals, and relation labels with policy/truth/time/hash/revision.
    - [x] 3.2.2 Create external-content FTS5 table/triggers for title/body/aliases/source/relation text while keeping semantic authority outside FTS.
    - [x] 3.2.3 Compile quoted phrases, normalized terms, exact identifiers, and field restrictions into bounded parameterized MATCH expressions; cap query length/filter complexity.
    - [x] 3.2.4 Return matched field, strategy-local rank/rationale, truth/policy/revision/navigation target, and exact/at-least/estimate total semantics.
    - [x] 3.2.5 Implement FTS delete/reconcile/temp-generation rebuild and membership hash comparison from authorized authority stream.
    - [x] 3.2.6 Prove offline operation, Unicode/diacritics/CJK/RTL, injection-shaped text, no-results-with-filters, corruption Partial, and 100k query plans.

  - [x] 3.3 F3.3 — Implement graph, temporal, and active-goal retrieval strategies

    **Objective:** Add three independently testable bounded candidate strategies that preserve graph/time/goal meaning and degrade separately.
    **Targets (subject to discovery):** adapt `retriever.rs`, `graph_intel.rs`, `truth.rs`, `goals.rs`; new `retrieval/{graph,temporal,goal}.rs` only by extracting existing responsibilities.
    **Prerequisites:** F2 graph/time/goals; F3.1–F3.2 candidate interfaces.
    **Invariants/non-negotiables:** Policy before seeds/expansion; graph max 3 hops, 120 nodes/180 edges for retrieval, per-hop 40/30/20; hidden intermediary omits path; temporal intersection never lets recency override truth; only authorized Active goals contribute.
    **Implementation steps:** Execute 3.3.1–3.3.6.
    **Failure/degraded behavior:** Each unavailable/timeout strategy reports its own Partial reason and returns no synthetic candidates; other strategies continue.
    **Focused validation:** V-RET-01 independent strategy ablations; V-GRAPH-01 cycles/hidden paths; V-TRUTH-01 intervals; active/non-active goal transition tests.
    **Evidence:** `evidence/F3/<run-id>/{reports/strategy-ablations.json,traces/graph/,traces/temporal/,traces/goal/}`.
    **Completion proof:** independent expected rank/membership for each strategy and every unavailable-state trace matches.
    **Rollback/containment:** Disable only failed strategy; keep policy/truth gates and remaining strategies.
    **IDs:** MGR-007, MGR-010, MGR-023, MGR-036–038; MGD-015, MGD-025; MG-H07, MG-H09, MG-O06–O09.

    - [x] 3.3.1 Resolve authorized entity/mention seeds and expand registry-filtered graph breadth-first ≤3 hops with visited guards, evidence minimums, per-hop caps, stable path-cost ties, and batched reads.
    - [x] 3.3.2 Omit entire paths with hidden intermediaries and emit only authorized frontier/truncation tokens; never reveal hidden IDs/counts/topology.
    - [x] 3.3.3 Parse declared temporal intent into instant/range/recency class and rank Valid-Time intersections plus named source/transaction recency under `temporal-v1`.
    - [x] 3.3.4 Exclude Deleted/Forgotten/default-Superseded and apply Stale/Unverified/Contradicted policy before temporal rank; preserve exact boundary/timezone behavior.
    - [x] 3.3.5 Select only caller/task/session-authorized Active goals, rank matching resumption/evidence context, and record goal ID/contribution; all other statuses contribute zero.
    - [x] 3.3.6 Add deadlines/cancellation/Partial traces and per-strategy exact goldens for cycles, hidden paths, temporal boundaries, goal transitions, and no-seed cases.

  - [x] 3.4 F3.4 — Implement deterministic query classifier and versioned adaptive RRF

    **Objective:** Select one documented query class/profile and fuse available ranks reproducibly without online weight mutation or silent redistribution.
    **Targets (subject to discovery):** adapt `retrieval_opt.rs`, `semantic_parser.rs`, retrieval weight schema; new `retrieval/{classifier,rrf,eval}.rs`.
    **Prerequisites:** F3.1–F3.3 strategies and judged corpus.
    **Invariants/non-negotiables:** Classes `identifier`, `exact_phrase`, `entity_relation`, `temporal`, `active_goal`, `exploratory` in declared precedence; fixed v1 budgets/weights and default k=60; unique candidate cap 320; unavailable strategy contribution is zero and named.
    **Implementation steps:** Execute 3.4.1–3.4.6.
    **Failure/degraded behavior:** Unknown classifier/profile version rejects activation and falls back to last approved profile; no per-request learning.
    **Focused validation:** V-RET-02 exact worksheet replay; classifier precedence goldens; tie-order properties; profile activation regression tests.
    **Evidence:** `evidence/F3/<run-id>/{reports/classifier-goldens.json,reports/rrf-replay.json,reports/profile-activation.json}`.
    **Completion proof:** every fused score recomputes exactly from stored one-based ranks, availability, weights, and k; candidate profile activates only with approved V-RET-03 evidence.
    **Rollback/containment:** Revert to last approved versioned profile; never silently alter or redistribute weights.
    **IDs:** MGR-001, MGR-011, MGR-025, MGR-036; MGD-017, MGD-025; MG-M07, MG-O16.

    - [x] 3.4.1 Implement deterministic classifier precedence and record class/version/reasons for identifier, quoted phrase, resolved entity/relation, parsed time, active-goal intent, and exploratory fallback.
    - [x] 3.4.2 Encode validation.md strategy budgets and design v1 weights/profile IDs/k as immutable versioned configuration bounded by hard maxima.
    - [x] 3.4.3 Fuse one-based ranks using weighted RRF with explicit availability, no missing-weight redistribution, stable semantic-ID dedup, and deterministic score/ID tie break.
    - [x] 3.4.4 Persist classifier/profile/version/k/availability/ranks/weights/contribution and separate evidence/goal/Memory-Worth terms for replay.
    - [x] 3.4.5 Build offline profile comparison/activation requiring judged thresholds, confidence intervals, forbidden/deletion invariants, and no >0.03 accepted metric regression.
    - [x] 3.4.6 Prohibit online/user-request feedback from directly mutating profile weights and test unknown/stale/partially available profile behavior.

  - [x] 3.5 F3.5 — Implement gates, diversity, token packing, and exact trace finalization

    **Objective:** Ensure only authorized, current, compatible, diverse, budget-fitting items reach model context and that `Used` is proven by final injected order.
    **Targets (subject to discovery):** adapt `retriever.rs`, agent prompt construction integration, trace schema/store; new `retrieval/{gates,packing,trace}.rs`.
    **Prerequisites:** F3.1–F3.4.
    **Invariants/non-negotiables:** Fixed gate order; no hidden record ID in filtered trace rows; diversity per source/episode/entity/kind; reserve 10% for exact identifiers and 10% active-goal context when present; never exceed token budget; finalize trace after actual prompt construction.
    **Implementation steps:** Execute 3.5.1–3.5.7.
    **Failure/degraded behavior:** Trace persistence failure must not fabricate Used; response records trace unavailable/failed according to policy; token estimator uncertainty remains under hard byte/token guard.
    **Focused validation:** V-RET-02 exact injected-order oracle; V-RET-01 truth/version/policy gates; V-POLICY-02 opaque filtered rows; prompt integration tests.
    **Evidence:** `evidence/F3/<run-id>/{traces/retrieval/,reports/packing-oracle.json,security/trace-redaction.json}`.
    **Completion proof:** every Used item equals committed injected set/order; every excluded candidate has policy-safe reason; packed tokens never exceed budget.
    **Rollback/containment:** Disable trace-dependent Used UI and use safe retrieval result; never reconstruct use from rank/proximity.
    **IDs:** MGR-001, MGR-004, MGR-025, MGR-036, MGR-038, MGR-044; MGD-017, MGD-025, MGD-033; MG-C04, MG-O01.

    - [x] 3.5.1 Gate authorization before strategy candidate creation, then Deleted/Forgotten/default-Superseded and declared Stale/Unverified/Contradicted policy.
    - [x] 3.5.2 Gate model/record/content version and Valid Time, then exact-deduplicate by semantic ID/content version before fusion.
    - [x] 3.5.3 Apply deterministic diversity by source, episode, entity, and record kind with cap `max(2, ceil(selected/3))` and stable tie order.
    - [x] 3.5.4 Greedily pack marginal utility per token while preserving identifier/active-goal reserves and hard caller budget.
    - [x] 3.5.5 Create trace header/items with policy hash/revision/query/class/profile/model/degradation, strategy ranks/scores, gates/reasons, token costs/allocations, and opaque unauthorized exclusions.
    - [x] 3.5.6 After prompt construction, transactionally finalize exact injected order/allocated tokens/response-task identity through the governed trace write path.
    - [x] 3.5.7 Separate Why stored, Why recalled, How used, Retrieved-filtered, and Available-safe explanations and prove only injected trace membership can produce Used.

  - [x] 3.6 F3.6 — Implement active goals and deterministic consolidation

    **Objective:** Support candidate/active goal workflows and bounded Episode→Summary→Skill→Rule compression while preserving source lineage and policy.
    **Targets (subject to discovery):** adapt `goals.rs`, `cognition.rs`, `dreaming.rs`, `reasoning.rs`, `scheduler.rs`; consolidation tables/links.
    **Prerequisites:** F2 typed records/links/truth; F3 trace and scheduler ports.
    **Invariants/non-negotiables:** Candidate cannot auto-promote without policy evidence; output identity = sorted parents + algorithm/version; all immediate `derived_from` links; restrictive policy; self-reflection untrusted/capped 0.6; insufficient independent evidence cannot promote.
    **Implementation steps:** Execute 3.6.1–3.6.6.
    **Failure/degraded behavior:** Model unavailable queues bounded work; crash resumes cursor without duplicate; correction marks dependents stale; uncertain evidence stays lower-level.
    **Focused validation:** V-CONS-01, V-TRUTH-01, V-POLICY-01, goal contribution tests, crash replay.
    **Evidence:** `evidence/F3/<run-id>/{reports/consolidation-ledger.json,traces/consolidation/,reports/goal-transitions.json}`.
    **Completion proof:** replay/crash yields one semantic output with complete immediate lineage and no policy/truth escalation.
    **Rollback/containment:** Pause consolidation and goal inference; retain explicit goals/source records and queued cursor.
    **IDs:** MGR-035, MGR-038–039, MGR-045; MGD-031, MGD-043; MG-O31.

    - [x] 3.6.1 Implement goal kind/title/status/priority/score semantics/owner/policy/provenance/evidence/progress/resumption and governed status transitions.
    - [x] 3.6.2 Keep inferred goals Candidate, require explicit/policy evidence for Active, and stop retrieval contribution immediately on pause/complete/conflict/stale/supersede/delete.
    - [x] 3.6.3 Build bounded episode boundaries/cursors and versioned consolidation candidate selection under scheduler/resource policy.
    - [x] 3.6.4 Derive Summary→Skill→Rule with sorted-parent semantic identity, immediate `derived_from` links, restrictive Effective Policy, truth/contradiction checks, and source history retention.
    - [x] 3.6.5 Enforce independent evidence/source diversity/success minimums, self-reflection trust cap, false-promotion reasons, and no automatic Rule escalation.
    - [x] 3.6.6 Implement durable resume/idempotency and downstream stale/reevaluation propagation when any source is corrected, superseded, forgotten, or deleted.

  - [x] 3.7 F3.7 — Implement paired tool/MCP/OpenClaw/sidecar learning without escalation

    **Objective:** Record meaningful success/failure outcomes and bounded aggregate reliability while preventing observations from granting authority or security capability.
    **Targets (subject to discovery):** adapt `active_learning.rs`, `feedback.rs`, tool/MCP/OpenClaw/sidecar completion hooks; new `cognition/tool_observation.rs` if replacing overlap.
    **Prerequisites:** F1 routed sources; F2 tool observation records; F3 trace attribution.
    **Invariants/non-negotiables:** One start/completion pair per invocation; typed outcomes; trivial repeated success aggregates only; n<20 Insufficient evidence/inert; never grant capability/widen scope/bypass approval/promote Rule/change security/delete/override explicit correction/newer version.
    **Implementation steps:** Execute 3.7.1–3.7.6.
    **Failure/degraded behavior:** Missing completion is diagnosable timeout/unknown; secret/unsafe result omits durable content; memory unavailable returns no-memory and no alternate store.
    **Focused validation:** V-TOOL-01, V-CONS-01, V-POLICY-02 source pairs, start/completion uniqueness/fault tests.
    **Evidence:** `evidence/F3/<run-id>/{reports/tool-observation-ledger.json,security/no-escalation.json,traces/tool-attribution/}`.
    **Completion proof:** all source classes pair once; n thresholds/precedence hold; mutation attempts outside named allowed ranking/archive policy fail.
    **Rollback/containment:** Disable learning effects while preserving audited invocation outcomes where policy permits.
    **IDs:** MGR-033, MGR-043–045; MGD-031, MGD-043; MG-O24, MG-O31.

    - [x] 3.7.1 Correlate start/completion by invocation and classify success, partial, expected/unexpected failure, timeout, cancellation, correction, undo, or unknown.
    - [x] 3.7.2 Store policy-safe tool/server/version/capability/goal/environment/input fingerprint/result summary/error/latency/retry/recovery/affected-record facts for meaningful outcomes.
    - [x] 3.7.3 Preserve failure Evidence/recovery result unless secret/unsafe; aggregate trivial repeated successes without creating durable memory volume.
    - [x] 3.7.4 Compute version/environment/window success rate and latency quantiles only at n≥20 with sample size; below threshold return Insufficient evidence.
    - [x] 3.7.5 Attribute task outcome across exact Used set under a named policy; keep Memory Worth inert below 20 and trace any allowed versioned contribution.
    - [x] 3.7.6 Add negative tests proving no observation can escalate capability/scope/approval/core promotion/security/deletion or override explicit correction/newer capability version.

  - [x] 3.8 F3.8 — Implement priority scheduler, cancellation, offline and pressure behavior

    **Objective:** Keep SQLite/CPU/model work off async executors, bound all queues/workers, and make foreground memory tasks preempt background cognition.
    **Targets (subject to discovery):** adapt `scheduler.rs`, `jobs.rs`, `maintenance.rs`, `observability.rs`, blocking-worker integration; resource/config modules.
    **Prerequisites:** F3 strategy/cognition job types.
    **Invariants/non-negotiables:** P0 stop/security/correction; P1 search/write/outbox; P2 reconciliation; P3 embedding/analytics; P4 consolidation/polish; >50ms work off Tokio executor; P0 yield/defer ≤100ms; wake queue ≤1024/coalesced; battery suspends P3/P4.
    **Implementation steps:** Execute 3.8.1–3.8.6.
    **Failure/degraded behavior:** Cancellation/deadline/pressure/worker failure stops or defers work without authority/cache corruption; durable events/outbox/cursors preserve eventual work.
    **Focused validation:** V-FAULT-01, V-PERF-01, initial V-RESOURCE-01, Tokio blocking-span instrumentation, offline/model-loss campaigns.
    **Evidence:** `evidence/F3/<run-id>/{performance/scheduler.json,traces/pressure/,reports/offline-degradation.json}`.
    **Completion proof:** zero graph-originated async blocking spans >50ms, P0 preempts ≤100ms, queue memory bounded, P3/P4 absent on battery, eventual catch-up passes.
    **Rollback/containment:** Pause P2–P4 and reduce concurrency/caches; preserve P0/P1 authority, FTS5, lifecycle, correction.
    **IDs:** MGR-009, MGR-022, MGR-028, MGR-039, MGR-042, MGR-045; MGD-015; MG-H03, MG-H14, MG-M16–M17.

    - [x] 3.8.1 Define bounded job envelope with priority, deadline, cancellation, coalescing key, authority cursor, resource class, retry budget, and correlation ID.
    - [x] 3.8.2 Move potentially >50ms SQLite, parsing, embedding, graph, analytics, and CPU work to bounded blocking/worker pools with no unbounded spawn.
    - [x] 3.8.3 Implement foreground arrival preemption/yield checks ≤100ms and deterministic fairness so required reconciliation eventually progresses.
    - [x] 3.8.4 Implement queue cap 1024 with coalescing/drop of rebuildable wakes while preserving durable outbox/event/cursor work.
    - [x] 3.8.5 Suspend P3/P4 on battery; shed caches/reduce concurrency on memory pressure; chunk/pause nonessential work on thermal/CPU/GPU/model pressure.
    - [x] 3.8.6 Emit redacted aggregate scheduler/latency/cache/revision/backlog/degradation metrics within ≤1% CPU and ≤1% interactive latency overhead.

  - [x] 3.9 F3.9 — Publish canonical API v2, revision patches, host parity, quality gates, and cutover

    **Objective:** Make `memory.v2` the sole public contract with bounded operations/errors/cursors/patches and equivalent Tauri/Axum behavior, then prove quality/scale and remove old APIs.
    **Targets (subject to discovery):** evolve `memory/{api,contract,error}.rs`; new `api/v2/{contract,dto,error,limits,capabilities}.rs`; desktop `commands/memory.rs` or replacement `memory_v2.rs`; server `memory_routes.rs` or discovered `routes/`; frontend runtime schemas/reducer contract fixtures only.
    **Prerequisites:** F3.1–F3.8; F2 projection/interchange.
    **Invariants/non-negotiables:** Envelope fields from design §8; operation hard caps/deadlines; cursor binds schema/query/policy/revision/sort/expiry; one WAL snapshot revision; patch applies only base=current; adapters contain no semantics; unsupported explicit; no full-corpus reload.
    **Implementation steps:** Execute 3.9.1–3.9.8; delete v1 only after parity/quality evidence.
    **Failure/degraded behavior:** Typed Unauthorized/Forbidden/InvalidRequest/Limit/Unsupported/Revision/Cursor/Refetch/Timeout/Cancelled/Dependency/Busy/Malformed/Integrity/Recovery/Idempotency/Crypto errors; prior valid snapshot remains labeled stale; bounded refetch.
    **Focused validation:** V-XPORT-01, V-UI-UNIT-01 reducer contract slice, V-FAULT-01 cursors/patches, V-RET-03 ≥200 queries, V-PERF-01 100k, V-REG-01.
    **Evidence:** complete `evidence/F3/<run-id>/{reports/host-parity.json,reports/retrieval-quality.json,performance/,traces/patch/,reviews/}`.
    **Completion proof:** normalized host goldens match; quality thresholds pass; warm p95 core ≤120ms/search ≤250ms/one-hop ≤500ms/predict ≤750ms; old routes/commands/full-refresh paths deleted.
    **Rollback/containment:** Disable mutation/remote/patches/individual strategies; retain bounded local v2 reads, FTS5, and active-query refetch.
    **IDs:** MGR-006–009, MGR-017, MGR-020, MGR-023, MGR-025, MGR-027, MGR-032, MGR-036, MGR-048; MGD-011–012, MGD-015, MGD-033, MGD-035, MGD-042; MG-H01, MG-H04, MG-H09, MG-H16–H17, MG-M18.

    - [x] 3.9.1 Define runtime-serializable v2 envelopes, DTOs, capabilities, warnings/degradation, errors, operation limits, retry/refetch instructions, and unknown-field/version behavior.
    - [x] 3.9.2 Implement `search`, `neighborhood`, `path`, `trace.get`, `aggregate`, `predict`, `temporal.diff`, `patch.list`, and lazy seven-section `inspect` with exact design defaults/hard maxima.
    - [x] 3.9.3 Implement `command.preview/commit/undo`, lifecycle, source, goal, health/capabilities, local interchange, and asynchronous export/import/rebuild jobs with exact payload/page/deadline limits.
    - [x] 3.9.4 Capture one WAL snapshot revision; issue authenticated cursor over schema/query/policy/revision/sort/expiry; ensure deterministic pages return authorized item at most once and expired/incompatible cursors request bounded refetch.
    - [x] 3.9.5 Emit base→target bounded patches/invalidation/recovery cursor after commit; specify duplicate/reorder/gap/policy/schema behavior and retain 10k revisions or 7 days.
    - [x] 3.9.6 Implement thin Tauri event/command and Axum HTTP/SSE adapters with common caller/context/limits/core ports and explicit capability differences for local-only operations.
    - [x] 3.9.7 Run ≥200 judged queries with Recall@10 ≥0.85, nDCG@10 ≥0.80, identifier/phrase ≥0.95, forbidden/deleted/forgotten/default-superseded exclusion 100%, per-class/ablation/95% bootstrap CI and regression block >0.03.
    - [x] 3.9.8 Materialize serialized 100k fixture run, assert correctness and query plans with ≥30 warm plus separate cold samples, cut over registrations, delete legacy memory/graph/search/full-refresh APIs and adapter business logic, then obtain Retrieval/Cognition/API/Security reviews and sign F3 manifest.

- [x] 4. F4 — Complete human Digital Twin, semantic list first, then conditional Canvas2D

  **Objective:** Integrate v2 through runtime schemas and revision-safe client state, deliver exactly seven truthful destinations and all core workflows in semantic DOM list/table plus inspector, then add renderer-neutral Semantic Scene and Canvas2D only after list/action parity.
  **Targets (subject to discovery):** evolve current `ui/src/shell/spaces/memory/` and `graph/`; new cohesive `api/`, `state/`, `scene/`, `destinations/`, `knowledge/` modules only while deleting replaced `graphData`, `memoryUniverseModel`, `lensController`, and duplicate semantics; UI E2E/visual/a11y suites.
  **Prerequisites:** signed F3 manifest, host parity, negative paths, performance floors, real capability states/traces.
  **Invariants/non-negotiables:** One caller policy/revision; no mixed snapshots or simulated controls; list/table is complete fallback and accessibility path; map/list/inspector use one scene/action controller; generated navigation not topology; Canvas hidden from accessibility tree; all meaning redundant in text/icon/pattern.
  **Implementation steps:** Complete F4.1–F4.9 strictly list/client/workflows before Canvas; serialize WebKitGTK/Orca/visual/resource runs.
  **Failure/degraded behavior:** Preserve prior snapshot as stale, show exact partial/offline/unauthorized/timeout/malformed/worker/renderer/recovery state with retry/correlation; renderer failure falls back to complete list; never show false empty memory.
  **Focused validation:** V-UI-UNIT-01, V-DT-01, V-E2E-01, V-A11Y-01, V-VIS-01, initial V-RESOURCE-01, V-ENTITY-01 UI slice, V-LIFE-01 UI slice.
  **Evidence:** `evidence/F4/<run-id>/{manifest.json,junit/,screenshots/,accessibility/,performance/,traces/,reviews/}`.
  **Completion proof:** all seven destinations and primary workflows complete without Canvas; map/list/inspector action hashes match; WCAG 2.2 AA/Orca and deterministic semantic visual review pass; old UI cutover complete.
  **Rollback/containment:** Keep complete list/table/inspector; disable divergent Canvas/map action and retain stale labeled snapshot + bounded refetch.
  **IDs:** MGR-001–002, MGR-006, MGR-008, MGR-010, MGR-012–016, MGR-017, MGR-021–026, MGR-030–031, MGR-038, MGR-040–041, MGR-045–046, MGR-048; MGD-001–005, MGD-013–017, MGD-026, MGD-030, MGD-035–036, MGD-046.

  - [x] 4.1 F4.1 — Build runtime schemas, canonical client, cache, and session/patch reducers

    **Objective:** Reject malformed/unsupported DTOs and keep each window’s query, policy, revision, selection, pending writes, representation, camera, and quality isolated and convergent.
    **Targets (subject to discovery):** new/adapted `memory/api/{client,schemas,errors,capabilities}.ts`, `state/{windowSession,snapshotCache,patchReducer}.ts`; replace `graphData.ts`, global graph stores, coarse event reloads.
    **Prerequisites:** F3 v2 fixtures/envelopes/patch protocol.
    **Invariants/non-negotiables:** Every request binds instance/generation/query/policy/base revision; cache key `(schema,revision,policyHash,queryHash)`; focus cancels/increments generation; mismatches discarded; patch atomic only if base=current; per-window ownership.
    **Implementation steps:** Execute 4.1.1–4.1.7.
    **Failure/degraded behavior:** Runtime parse failure is Malformed; scope/policy change discards response/cache; patch gap preserves stale snapshot and bounded refetch; pending failure rolls back optimistic display beside action.
    **Focused validation:** V-UI-UNIT-01 generated DTO rejection, patch reorder/duplicate/gap, request races, cache isolation, window ownership.
    **Evidence:** `evidence/F4/<run-id>/{junit/V-UI-UNIT-01.xml,traces/reducers/,reports/runtime-schema-coverage.json}`.
    **Completion proof:** property/golden reducer sequences converge to authoritative active query and never mix generations/policies/revisions/windows.
    **Rollback/containment:** Disable patches and use bounded v2 refetch; preserve last valid snapshot as Stale.
    **IDs:** MGR-008, MGR-013, MGR-017, MGR-020–021, MGR-031; MGD-011–012, MGD-014, MGD-035; MG-H06, MG-H17, MG-M03, MG-M06, MG-M18–M19.

    - [x] 4.1.1 Implement runtime validation for every v2 envelope/DTO/error/capability/page/degradation enum and deny unknown required schema/action values.
    - [x] 4.1.2 Implement one client operation map for Tauri/HTTP/SSE normalization, AbortController deadlines, correlation IDs, explicit unsupported capability, and no local semantic inference.
    - [x] 4.1.3 Implement `MemoryWindowSessionV2` exact states, per-instance request ownership, generation increment/cancel, query/policy/revision guards, and detached restore validation.
    - [x] 4.1.4 Implement immutable bounded snapshot cache keyed by schema/revision/policy/query with policy-change invalidation and no hidden-data-derived key/output.
    - [x] 4.1.5 Implement atomic matching patch, duplicate ignore, reorder/gap/schema/policy refetch, invalidation, pending-confirm-by-matching-revision, and typed rollback reducers.
    - [x] 4.1.6 Make single click select and double click expand/fit disjoint; remove selection safely on refresh and announce re-resolution/close.
    - [x] 4.1.7 Delete coarse full graph reload/global session/client-side policy filtering after reducer and E2E parity evidence.

  - [x] 4.2 F4.2 — Compose the seven-destination Memory Control Center shell

    **Objective:** Present exactly Overview, Recall, Knowledge, Timeline, Goals, Sources, and Health under one truthful revision/policy/capability header and responsive navigation.
    **Targets (subject to discovery):** `MemoryControlCenter.tsx`, `destinations/{Overview,Recall,Knowledge,Timeline,Goals,Sources,Health}.tsx`, shared shell/navigation/status CSS using existing tokens.
    **Prerequisites:** F4.1 client/session; inspect current Memory cards/inspector/cognition/lens registrations before replacement.
    **Invariants/non-negotiables:** No literal brain/sentience/emotion copy; Timeline omitted if unsupported; each destination exact state; controls exist only with implemented success/failure; Overview never infers health from missing data.
    **Implementation steps:** Execute 4.2.1–4.2.7 and keep destination data one revision or explicitly Stale/Unavailable.
    **Failure/degraded behavior:** Destination-level retry preserves intent; unauthorized content exposes no count/placeholder; shell remains navigable in offline/recovery/partial states.
    **Focused validation:** V-DT-01 destination/state/capability matrix; V-VIS-01 shell matrix; V-A11Y-01 landmarks/navigation.
    **Evidence:** `evidence/F4/<run-id>/{screenshots/V-DT-01/,reports/destination-state-matrix.json,accessibility/navigation.json}`.
    **Completion proof:** exactly seven destination definitions, real capability binding, one revision/policy context, no inert/false controls across all states.
    **Rollback/containment:** Hide unsupported destination/action while preserving Overview/Health/list access; never simulate content.
    **IDs:** MGR-001, MGR-010–011, MGR-017, MGR-031, MGR-038, MGR-045–046; MGD-001, MGD-005, MGD-030; MG-H12, MG-M08–M10, MG-L02, MG-L06–L07.

    - [x] 4.2.1 Build shared header with exact destination, Graph Revision, policy context, capability/degradation/offline/recovery status, stale timestamp, and evidence link without revealing hidden scope.
    - [x] 4.2.2 Build Overview from authority-backed recent changes, contradictions, active goals, pending cognition, and actions; use goal-led manual onboarding when empty and request source consent before scans.
    - [x] 4.2.3 Build Recall destination shell for full-corpus search, filters, result totals, strategy availability, rationale, and Why-this-answer trace navigation.
    - [x] 4.2.4 Build Knowledge destination shell with List as complete first representation, Map segment only after parity, inspector/path/correction status, and no global hairball.
    - [x] 4.2.5 Build Timeline only when valid-time/transaction-time snapshot/diff capability exists; label additions/expiry/contradiction/supersession/correction and requested timezone/range.
    - [x] 4.2.6 Build Goals and Sources shells with status/evidence/progress/resume and policy/consent/derivation/lifecycle workflows respectively.
    - [x] 4.2.7 Build Health from exact authority/index/model/outbox/backlog/resource/degradation/recovery/last-verified/remediation/Evidence Artifact state; developer details remain local gated.

  - [x] 4.3 F4.3 — Deliver semantic list/table and full-corpus Recall before Canvas

    **Objective:** Make finding, filtering, navigating, selecting, expanding, sorting, and inspecting the whole authorized corpus complete without a visual renderer.
    **Targets (subject to discovery):** `knowledge/SemanticList.tsx`, Recall result/table components, virtualization helpers, destination CSS/tests; evolve `MemoryGraphFallback.tsx` rather than leave duplicate fallback logic.
    **Prerequisites:** F4.1–F4.2.
    **Invariants/non-negotiables:** Search is backend full-corpus; local visible operation labeled `Filter this view`; list contains every scene item/action/status/direction; honest `showing N of M`/`at least`/`estimate`; no 300-stop Tab path.
    **Implementation steps:** Execute 4.3.1–4.3.7.
    **Failure/degraded behavior:** No-result preserves filters and never claims empty Authority Store; partial names omitted strategies; timeout/offline/malformed keeps intent/retry; unauthorized exposes no existence.
    **Focused validation:** V-DT-01 list-only workflows, V-E2E-01 search faults, V-A11Y-01 keyboard/Orca list tasks, V-VIS-01 semantic assertions.
    **Evidence:** `evidence/F4/<run-id>/{traces/list-actions/,screenshots/recall-list/,accessibility/list.json}`.
    **Completion proof:** every primary map-intended action can already complete through list/table and produces authority revision/audit outcomes.
    **Rollback/containment:** Keep list-only mode; disable Map segment.
    **IDs:** MGR-006, MGR-014, MGR-023–024, MGR-031; MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.

    - [x] 4.3.1 Build search input with 512-character validation, debounced/cancellable submit, implemented platform-correct shortcuts, query/filter chips, saved-filter seam only if approved, and announced result state.
    - [x] 4.3.2 Render mixed ranked results with kind, matched field, rationale, Relative score/profile, policy summary, Truth State, revision, source/time, and navigation target.
    - [x] 4.3.3 Render exact/at-least/estimated totals and truncation/cursor controls; distinguish full-corpus Search from `Filter this view` at all times.
    - [x] 4.3.4 Build virtualized semantic list/table exposing node/edge kind, authority class, direction source→target, evidence/status, selected/current/expanded state, and all authorized actions.
    - [x] 4.3.5 Implement sort/filter/page/expand/path/trace navigation without loading full adjacency; preserve selection and focus through bounded refetch.
    - [x] 4.3.6 Cover empty/loading/ready/partial/stale/offline/unauthorized/timeout/malformed/error/deleted/recovery states with exact copy, correlation, retry, and preserved intent.
    - [x] 4.3.7 Add list-only E2E for find entity/alias/content/source/relation/goal, no result, hidden result, partial strategy, inspect, path, correction, and lifecycle.

  - [x] 4.4 F4.4 — Build structured inspector and explain/correct/delete workflows

    **Objective:** Provide one operational claim→provenance/evidence/use/history→previewed correction/lifecycle path with independent section states.
    **Targets (subject to discovery):** evolve `MemoryInspector.tsx`/CSS; new `knowledge/Inspector.tsx`; dialogs/sheets/action controller; Converse deep-link integration discovered before edit.
    **Prerequisites:** F4.3 list and F3 inspect/command/trace/lifecycle APIs.
    **Invariants/non-negotiables:** Sections Identity, Truth, Evidence, Relationships, Use, History, Actions; each idle/loading/ready/empty/partial/stale/offline/error; Use separates Why stored/recalled/used; correction commit requires matching preview; pending unconfirmed styling.
    **Implementation steps:** Execute 4.4.1–4.4.8.
    **Failure/degraded behavior:** Section failures isolate; stale preview refreshes; typed commit failure stays beside action and rolls back optimism; focus returns to initiator; Deleted selection closes/announces safely.
    **Focused validation:** V-DT-01, V-E2E-01, V-ENTITY-01 UI, V-LIFE-01 UI, V-A11Y-01 inspector/dialog scripts.
    **Evidence:** `evidence/F4/<run-id>/{traces/inspect-correct-delete/,screenshots/inspector/,accessibility/inspector.json}`.
    **Completion proof:** users can explain origin/currentness/retrieval use and complete correction/merge/split/relation/forget/restore/delete with revision/audit proof from list only.
    **Rollback/containment:** Make affected mutation read-only while retaining explanation/evidence/history; never bypass preview.
    **IDs:** MGR-005, MGR-018–019, MGR-024–025, MGR-040–041; MGD-001, MGD-027, MGD-036; MG-C02, MG-C04, MG-C07, MG-H11, MG-O01–O04, MG-O08, MG-O10–O13.

    - [x] 4.4.1 Implement seven lazy inspector sections with independent state/retry/correlation and exact revision/time/policy/truth/provenance labels.
    - [x] 4.4.2 Render evidence source/locator/method/version/polarity/score semantics, relationship direction/registry/evidence/validity, and history corrections/supersession/contradiction.
    - [x] 4.4.3 Render Use as Why stored, Why recalled, How used; link Used only to exact trace injected item and show filtered reasons without hidden details.
    - [x] 4.4.4 Implement correction preview with current/proposed value, evidence, scope, affected count, reversibility, base revision, audit consequence, then commit result revision/audit/affected/undo.
    - [x] 4.4.5 Implement entity rename/type/alias/proposal accept/reject/merge/split/reverse with policy and stale-preview handling.
    - [x] 4.4.6 Implement relation create/edit/type/direction/evidence/confirm/expire/delete/undo and prediction materialization with Relative score/rationale and pending confirmation.
    - [x] 4.4.7 Implement contradiction confirm/supersede/keep-both and evidence-bearing path explanation only when capability authorizes each action.
    - [x] 4.4.8 Implement Forget/Restore/Hard Delete/plain-language crypto-state flows with dependency choices, 30-day window, reconciliation status, no Crypto-Shredded claim absent proof, focus containment/restoration.

  - [x] 4.5 F4.5 — Complete Goals, Sources, Timeline, Health, and offline/recovery workflows

    **Objective:** Make non-Knowledge destinations operational rather than dashboards, including consent, resume, time diff, pressure, and recovery actions.
    **Targets (subject to discovery):** destination modules and shared action/status components; source file picker/desktop capability integration; no unsupported remote ingest.
    **Prerequisites:** F4.2 shell, F4.4 inspector/actions.
    **Invariants/non-negotiables:** Every action maps to real capability; candidate goals distinguished from Active; consent before scan; Timeline omitted if unavailable; Recovery_Mode permits only diagnostics/verified recovery; offline floor explicitly shown.
    **Implementation steps:** Execute 4.5.1–4.5.6.
    **Failure/degraded behavior:** Partial section names exact unavailable capability; source cancel shows cursor; recovery failure remains Recovery_Mode; offline does not disable local FTS/lifecycle/correction if available.
    **Focused validation:** V-DT-01 all destinations; V-E2E-01 offline/pressure/recovery; V-FAULT-01; V-A11Y-01 destination workflows.
    **Evidence:** `evidence/F4/<run-id>/{traces/destinations/,screenshots/destinations/,accessibility/destinations.json}`.
    **Completion proof:** one real primary task succeeds in each destination with exact state, revision, policy, and failure behavior.
    **Rollback/containment:** Omit unsupported controls/destination capability while retaining Health explanation and list-first core.
    **IDs:** MGR-010, MGR-017, MGR-031, MGR-038–040, MGR-045–046; MGD-030, MGD-035; MG-M08, MG-L02, MG-L06, MG-O09–O10.

    - [x] 4.5.1 Render Goals statuses/provenance/linked memories/progress/conflicts and implement candidate review, activate/pause/complete, priority update, and resume context.
    - [x] 4.5.2 Render Sources policy/trust/consent/version/derivations/lifecycle; implement consent, candidate preview/exclude/approve, cancel/resume, and delete dependency flow.
    - [x] 4.5.3 Render Timeline snapshot/diff controls only with capability; show requested range/timezone/revision and additions/expiry/contradiction/supersession/correction in text plus visual state.
    - [x] 4.5.4 Render Health authority/index/model/outbox/backlog/pressure/degradation/recovery/last verified/remediation/evidence without inferred wellness or private content.
    - [x] 4.5.5 Implement offline/embedder-loss/LLM-loss/battery/memory/thermal/model-pressure messaging with exact preserved capabilities, queued work, and recovery.
    - [x] 4.5.6 Implement Recovery_Mode diagnostics and local verified restore/import flow; disable all writes and keep failed verification in Recovery_Mode.

  - [x] 4.6 F4.6 — Implement pure Semantic Scene and authorized action parity

    **Objective:** Derive one deterministic renderer-neutral semantic collection and action set from validated policy-safe DTOs, session, capabilities, and visual tokens.
    **Targets (subject to discovery):** evolve `GraphScene.ts`, `graphModel.ts`, `memoryUniverseModel.ts`; new `scene/{semanticScene,actions,layout,visualTokens}.ts` replacing duplicate logic.
    **Prerequisites:** list/action workflows F4.3–F4.5 pass; F4.1 schemas.
    **Invariants/non-negotiables:** Equal input→equal scene hash; endpoint complete; malformed/unauthorized omitted; navigation containers not edges; only authorized actions; selected identity stable under aggregation; map/list action/item hash parity.
    **Implementation steps:** Execute 4.6.1–4.6.6.
    **Failure/degraded behavior:** Scene diagnostics contain no private content; malformed item omitted and list remains usable; unsupported action absent rather than inert.
    **Focused validation:** V-UI-UNIT-01 scene purity/hash/action parity; V-VIS-01 semantic JSON; policy paired scenes.
    **Evidence:** `evidence/F4/<run-id>/{reports/scene-goldens.json,security/scene-pairs.json,traces/action-parity/}`.
    **Completion proof:** deterministic scene/action/list hashes and policy-pair non-interference pass for all query modes/states.
    **Rollback/containment:** Disable Map; list continues through same action controller.
    **IDs:** MGR-001–002, MGR-004, MGR-012, MGR-026; MGD-003–004, MGD-026, MGD-046; MG-C03, MG-H02, MG-M09–M11, MG-O19.

    - [x] 4.6.1 Define semantic item/action/token/layout-hint/diagnostic schemas with stable IDs, kind, authority, truth, direction, evidence, provenance, validity, revision, and policy-safe labels.
    - [x] 4.6.2 Implement pure builder that validates endpoints, omits malformed/unauthorized items, orders deterministically, derives no new facts, and hashes complete semantic output.
    - [x] 4.6.3 Implement search/overview treemap-grid, ego radial rings, path layered DAG, temporal lanes, and goal/source grouped-lane layout hints with deterministic seed from query hash/revision.
    - [x] 4.6.4 Represent repeated path semantic IDs visually without duplicating semantic collection identity; keep navigation groups as labeled containers.
    - [x] 4.6.5 Centralize typed select/expand/inspect/path/correct/merge/split/relate/forget/restore/delete/fit/back/forward actions and capability authorization.
    - [x] 4.6.6 Assert list/map/inspector item/action parity, scene purity, unknown/malformed isolation, generated-navigation exclusion, and no hidden policy cues.

  - [x] 4.7 F4.7 — Implement conditional authoritative Canvas2D, layout, LOD, culling, hit testing, and camera

    **Objective:** Add a calm query-scoped Canvas2D representation only after list/action parity, with deterministic bounded geometry and no second business model.
    **Targets (subject to discovery):** replace drawing responsibilities in `MemoryUniverse.tsx`/`KnowledgeGraphLens.tsx`; new `knowledge/{Graph2D,Camera,Status}.tsx` and bounded worker; retire SVG business logic after evidence.
    **Prerequisites:** F4.3 list completeness and F4.6 scene/action parity; target-hardware smoke confirms Canvas viable.
    **Invariants/non-negotiables:** Balanced 240 nodes/360 edges/80 labels/512KiB; hard 500/750/160/2MiB; no global force layout; >50ms layout uses bounded packed worker; viewport +64px overscan; selected/focused semantics never culled; zoom 0.25–4; pan margin 25%.
    **Implementation steps:** Execute 4.7.1–4.7.8.
    **Failure/degraded behavior:** Worker/renderer/context error yields complete list plus exact Renderer failure; cap shows truncation/narrow/expand; pressure follows quality ladder to list-first.
    **Focused validation:** V-UI-UNIT-01 geometry/camera/hit tests; V-E2E-01 renderer failure; V-RESOURCE-01 frame/heap/idle; V-VIS-01 scene semantics.
    **Evidence:** `evidence/F4/<run-id>/{performance/canvas.json,traces/camera-layout/,screenshots/knowledge-map/}`.
    **Completion proof:** deterministic scene renders within caps; camera fits contain requested bounds; hit/cull preserve selected/focused/path semantics; fallback completes all tasks.
    **Rollback/containment:** Disable Canvas and retain list/table/inspector; no SVG semantic fallback with duplicate logic.
    **IDs:** MGR-015–016, MGR-022–023, MGR-026, MGR-031; MGD-002, MGD-013, MGD-015, MGD-026, MGD-046; MG-H14–H15, MG-M01–M02, MG-M10, MG-L09, MG-L13.

    - [x] 4.7.1 Build Canvas drawing as a pure consumer of Semantic Scene and shared action dispatch; keep no policy, truth, relation, prediction, or lifecycle decisions in renderer.
    - [x] 4.7.2 Implement deterministic query layouts and packed worker protocol with query/revision seed, cancellation/generation guard, ≤50ms main-thread budget, and no continuous force simulation.
    - [x] 4.7.3 Enforce balanced/hard node-edge-label-byte caps and render honest truncation/frontier/narrowing controls.
    - [x] 4.7.4 Implement uniform spatial grid, viewport+64px culling, edge continuity for visible endpoints/selected path, and deterministic label collision/priority: selected, focus, path/Used, match, contradiction, neighbor, rank.
    - [x] 4.7.5 Implement mouse/touch/keyboard hit testing from spatial index with no O(corpus) scan and preserve hidden visual labels in semantic list.
    - [x] 4.7.6 Implement camera world coordinates, zoom `[0.25,4]`, 25% margin pan bounds, pointer/pinch-centroid zoom, two-finger coarse pan, inspector-aware reframing/offscreen marker.
    - [x] 4.7.7 Implement distinct Fit visible/selection/neighborhood, Reset, Back, Forward with query/filter/focus/camera history and revision-compatible requery before restore.
    - [x] 4.7.8 Implement quality ladder decoration→labels→analytics→120/180 scene→list-first without degrading truth/search/inspect/correct/lifecycle/keyboard actions.

  - [x] 4.8 F4.8 — Implement exact responsive, accessibility, visual-token, focus, input, and motion contracts

    **Objective:** Make every core workflow operable and semantically truthful at exact viewport/zoom/input/AT states with finite motion and existing design tokens.
    **Targets (subject to discovery):** Memory Control Center CSS/components, shared token definitions, Graph2D/list/inspector focus controller, shortcut help; use existing `--space-*`, `--color-*`, `--font-*`, `--motion-*`, `--radius-*` tokens and add semantic graph aliases only where absent.
    **Prerequisites:** F4.2–F4.7.
    **Invariants/non-negotiables:** Exact breakpoints/composition; ≥44px coarse targets; body ≥14px, map labels ≥12px readable LOD; focus ≥2px AA; one map tab stop; Canvas aria-hidden except summary; no hover-only meaning; max transition 400ms; no ambient animation; idle loops stop ≤2s.
    **Implementation steps:** Execute 4.8.1–4.8.9 with semantic visual assertions and human reviews.
    **Failure/degraded behavior:** At constrained width/zoom/forced colors use list-first/sheet/overlay; reduced motion immediate/static; focus always restored; no clipped action may be hidden without accessible alternative.
    **Focused validation:** V-A11Y-01, V-VIS-01, V-RESOURCE-01; axe, keyboard, Orca, forced-colors, 200%, RTL/CJK, pointer coarse, semantic screenshot JSON.
    **Evidence:** `evidence/F4/<run-id>/{accessibility/V-A11Y-01/,screenshots/V-VIS-01/,performance/motion-idle.json,reviews/{accessibility,visual-truth}.json}`.
    **Completion proof:** axe has no serious/critical; Orca completes all core tasks; semantic assertions show no invented topology/score/state; every accepted diff has reviewer rationale.
    **Rollback/containment:** Force list-first/minimal quality/reduced motion while retaining complete operations.
    **IDs:** MGR-013–016, MGR-022, MGR-026, MGR-031; MGD-013–014, MGD-026, MGD-046; MG-H10–H15, MG-M01–M03, MG-M24–M26, MG-L04–L05, MG-L08–L10.

    - [x] 4.8.1 At width ≥1200px compose 240px navigation + flexible workspace min 560px + reserved 360px inspector; collapse regions only by explicit user action.
    - [x] 4.8.2 At width 800–1199px compose 72px rail + flexible workspace + 320px focus-managed overlay inspector that reframes/marks selection.
    - [x] 4.8.3 At width <800px or content height <600px compose single-column search-first, mutually exclusive Map/List segment, and full-height focus-managed inspector sheet.
    - [x] 4.8.4 Enforce coarse-pointer ≥44×44px targets, persistent non-hover labels/actions, pinch-centroid zoom/two-finger pan, no single-finger map trap, and platform-correct implemented shortcuts only.
    - [x] 4.8.5 Implement map composite: one Tab entry; spatial Arrow; Home/End; typeahead; Enter select; Shift+Enter expand; Menu/Shift+F10 actions; Escape nested close/focus return; concise aria summary only.
    - [x] 4.8.6 Implement dialog/drawer/sheet initial focus, containment, inert background, Escape, live announcement, initiator restoration, and per-window focus isolation under patch/remove races.
    - [x] 4.8.7 Define redundant semantic visual aliases for kind shape/icon+text, authority line/badge, truth icon/pattern+text, direction arrow+source→target, selection/focus/pending/error; hidden policy gets no gap/color/count.
    - [x] 4.8.8 Enforce body ≥14px, graph labels ≥12px, long-label DOM wrap/map ellipsis with full accessible name, ≥2px AA focus, light/dark/forced-color/CVD-safe tokens, and legend generated only from present encodings.
    - [x] 4.8.9 Implement exact motion: focus 80ms linear; selection 120ms ease-out; inspector 180ms; camera 220ms cubic; scene 300ms; temporal 320ms; inferred→stored 240ms once; status ≤120ms; hard max 400ms; cancel on input; reduced motion immediate/≤80ms crossfade; no particles/glow/breathing/orbit/edge flow; stop render/rAF/event loop ≤2s.

  - [x] 4.9 F4.9 — Run human Digital Twin evidence and cut over the old UI

    **Objective:** Prove seven destinations, list-first workflows, scene/map parity, visual truth, accessibility, and resource floors before deleting the old Memory Universe path.
    **Targets (subject to discovery):** UI unit/component/E2E/Playwright/Orca suites, old `MemoryUniverse`, `KnowledgeGraphLens`, `GraphCanvas3D` registrations and models; preserve dormant 3D files only inaccessible until F6 if still needed for study.
    **Prerequisites:** F4.1–F4.8.
    **Invariants/non-negotiables:** No mocked simulated success in release E2E; screenshots require semantic JSON/human review; old UI deletion only after complete list/action parity; no 3D shipping control.
    **Implementation steps:** Execute 4.9.1–4.9.6.
    **Failure/degraded behavior:** Any Canvas/a11y/resource failure ships list-first only; any workflow failure blocks old UI deletion and F5, without reviving unsafe APIs.
    **Focused validation:** full F4 suite set and focused existing UI regressions.
    **Evidence:** complete `evidence/F4/<run-id>/` with Product/UX, Accessibility, Visual Truth, Frontend reviews.
    **Completion proof:** F4 manifest Pass; old synthetic/global/SVG business path absent; complete accessible Digital Twin works without Canvas and with Canvas if enabled.
    **Rollback/containment:** List-first authoritative product; disable Canvas/action independently; no legacy split-brain restoration.
    **IDs:** MGR-012–017, MGR-022, MGR-024, MGR-026–027, MGR-030–031, MGR-048; MGD-002, MGD-021, MGD-026, MGD-030, MGD-046; MG-C01, MG-H02, MG-M20.

    - [x] 4.9.1 Run unit/reducer/scene/action/geometry/camera tests and list-only seven-destination E2E against real v2 fixtures.
    - [x] 4.9.2 Run authority write→revision→patch/refetch→list/scene/inspector E2E plus offline/partial/stale/conflict/malformed/timeout/worker/renderer/delete/Recovery_Mode without simulated success.
    - [x] 4.9.3 Run screenshot/semantic matrix at 640×480, 800×600, 1176×775, 1440×900, 1920×1080, ultrawide; 100/125/150/200%; light/dark/forced colors; LTR/RTL/CJK; every state.
    - [x] 4.9.4 Run axe, complete keyboard, Orca transcripts/videos, 200% zoom, target-size, focus return, map/list semantics/action parity, and human accessibility review.
    - [x] 4.9.5 Run WebKitGTK frame/idle/heap/quality-ladder profiles; require p95 frame ≤33.3ms, loops stop ≤2s, idle CPU delta ≤2pp, bounded heap/queues, and list preservation.
    - [x] 4.9.6 Delete old global graph state, synthetic topology, inert controls, duplicate SVG renderer business logic, unsupported copy, v1 mocks/tests; sign F4 manifest with F3 predecessor.

- [x] 5. F5 — Production hardening, scale, multi-window, portability, and release proof

  **Objective:** Prove the entire v2 product at 100k under competing laptop load, corruption/deletion/rebuild/interchange/offline/multi-window campaigns, complete all visual/device/supply-chain/release evidence, and remove every superseded path/dependency/claim.
  **Targets (subject to discovery):** core/eval/UI/adapters/CI/release docs and packaging touched by previous gates; no new product scope.
  **Prerequisites:** signed F4 manifest and complete list-first product; release hardware/dependencies/fixtures frozen.
  **Invariants/non-negotiables:** Correctness in same run as performance; common work bounded by visible/query scope; no policy leak under timing/cache/logs; release closure exact/pinned/FOSS-reviewed; zero open blocking risk; 2D/list is public-ready independent of F6.
  **Implementation steps:** Complete F5.1–F5.8; serialize all heavy runs and freeze inputs between run/sign-off.
  **Failure/degraded behavior:** Disable remote/mutation/analytics/patches/detached windows/Canvas/cognition independently while retaining safe local authority/list; unknown license or P0 failure blocks release.
  **Focused validation:** all V-* F0–F5 suites, full 100k/regression/fault/rebuild/interchange/resource/visual/a11y/SBOM matrices and independent reviews.
  **Evidence:** `evidence/F5/<run-id>/` complete release bundle and predecessor chain.
  **Completion proof:** 48/48 MGR, 46/46 MGD, 65/65 findings, 31/31 opportunities mapped to valid evidence/status; zero blocking risks; all mandatory role sign-offs.
  **Rollback/containment:** Capability disable/read-only/Recovery_Mode or clean data reset only; never legacy restoration.
  **IDs:** All MGR-001–048 except optional GO behavior of MGR-030; all MGD-001–046 except optional GO; all MG finding/opportunity IDs.

  - [x] 5.1 F5.1 — Run 100k correctness, latency, query-plan, and resource hardening

    **Objective:** Validate and optimize measured hotspots without changing frozen semantics/policy or hiding correctness failures.
    **Targets (subject to discovery):** `kria-eval` memory graph suite, SQLite indexes/queries, bounded caches/workers, UI scene/resource harness.
    **Prerequisites:** Frozen `mg-release-v2`, release build, reference hardware/power protocol; no concurrent heavy job.
    **Invariants/non-negotiables:** ≥30 warm samples plus separate cold; bootstrap 95% CI; correctness assertions same run; plans reject corpus adjacency scans; optimization stays behind stable ports/contracts.
    **Implementation steps:** Execute 5.1.1–5.1.7.
    **Failure/degraded behavior:** Missed budget blocks release or disables optional strategy/Canvas; only after indexed SQLite optimization may architecture review evaluate rebuildable analytics via existing port.
    **Focused validation:** V-PERF-01, V-RESOURCE-01, V-GRAPH-01, V-RET-03, V-POLICY-02.
    **Evidence:** `evidence/F5/<run-id>/performance/{samples,summary,query-plans,resource-trace}.json`.
    **Completion proof:** core retrieval ≤120ms, search ≤250ms, one-hop ≤500ms, prediction ≤750ms p95 with correctness; no >50ms async blocking; frame/idle/heap/queue budgets pass under competing model load.
    **Rollback/containment:** Revert only unsafe optimization; disable optional expensive strategy/analysis; retain bounded indexed SQLite.
    **IDs:** MGR-006–009, MGR-022–023, MGR-032, MGR-036, MGR-045; MGD-015, MGD-024–025; MG-H01, MG-H03–H04, MG-H09, MG-H14.

    - [x] 5.1.1 Generate/verify 100k authority fixture once, record membership hashes/planted answers/storage size, and reuse immutable fixture across campaigns.
    - [x] 5.1.2 Run exact correctness for search, five strategies, graph depths/paths, time, goals, traces, totals/cursors, lifecycle exclusions, and policy-paired queries before accepting samples.
    - [x] 5.1.3 Capture `EXPLAIN QUERY PLAN` for hot SQL and fail corpus-wide adjacency scans, missing policy/selectivity indexes, N+1 endpoint/evidence reads, or unbounded temp sorts.
    - [x] 5.1.4 Tune indexes, batching, prepared statements, chunking, revision caches, worker concurrency, and payload fields within immutable hard limits; rerun only affected slices then full gate.
    - [x] 5.1.5 Measure cold and ≥30 warm p50/p95/p99/bootstrap CI under idle and competing local-model load with hardware/power/thermal/build/model facts.
    - [x] 5.1.6 Measure async blocking, foreground preemption, queue memory, cache/heap steady band after 20 cycles, frame p95, idle CPU/GPU, and quality-ladder transitions.
    - [x] 5.1.7 If SQLite still misses accepted analytical budget after evidence-backed tuning, conduct architecture review through GraphAnalyticsPort; do not add distributed/authoritative backend or ANN scope without replacement decision.

  - [x] 5.2 F5.2 — Prove multi-window ownership and revision convergence

    **Objective:** Ensure detached and simultaneous Memory windows share only immutable compatible cache data while retaining independent intent/focus/pending/camera/subscriptions.
    **Targets (subject to discovery):** UI session/cache/patch reducers, Tauri window lifecycle/events, E2E harness.
    **Prerequisites:** F4 session implementation; F5 stable release fixture.
    **Invariants/non-negotiables:** One window cannot reset/leak another; shared cache only equal schema/revision/policy/query; close cancels owned work only; delete/policy invalidation reaches all affected windows; no mixed revision.
    **Implementation steps:** Execute 5.2.1–5.2.6.
    **Failure/degraded behavior:** Lag/gap preserves per-window stale snapshot and bounded refetch; incompatible detached restore discards cache; detached-window capability can be disabled.
    **Focused validation:** V-UI-UNIT-01, V-E2E-01, V-LIFE-01, V-FAULT-01 multi-window sequences.
    **Evidence:** `evidence/F5/<run-id>/{traces/multi-window/,reports/cache-ownership.json,videos/multi-window/}`.
    **Completion proof:** randomized open/focus/write/lag/scope/delete/close sequences converge each active query with zero cross-window mutation/leak.
    **Rollback/containment:** Disable detached windows and retain single-window complete product.
    **IDs:** MGR-008, MGR-013, MGR-021, MGR-040; MGD-014, MGD-035; MG-H06, MG-H17, MG-M19.

    - [x] 5.2.1 Test two+ windows with distinct destinations/queries/policies/selections/cameras/quality and assert reducer ownership isolation.
    - [x] 5.2.2 Test exact compatible shared cache reuse and rejection after revision/schema/policy/query mismatch.
    - [x] 5.2.3 Test simultaneous focus and writes, pending confirmation, duplicate/reordered/missing patches, lag, reconnect, and bounded per-window refetch.
    - [x] 5.2.4 Test policy/capability identity change discards incompatible in-flight responses/caches/pending writes/traces across affected windows only.
    - [x] 5.2.5 Test Forget/Delete/source cascade invalidates all affected snapshots/inspectors/traces without exposing deleted content or resetting unrelated windows.
    - [x] 5.2.6 Test detach restore, close cancellation/subscription cleanup, focus return, heap recovery, and no orphan event listeners/workers.

  - [x] 5.3 F5.3 — Complete deletion, rebuild, corruption, model-migration, and recovery campaigns

    **Objective:** Demonstrate crash convergence and zero residue across every authority/derived/client surface at release scale.
    **Targets (subject to discovery):** authority/lifecycle/outbox/rebuild/integrity/recovery modules, caches/UI, fault injector/eval.
    **Prerequisites:** F5.1 fixture; F1 mechanisms and F3 indexes.
    **Invariants/non-negotiables:** Authority hashes unchanged by projection rebuild; Deleted wins over stale outbox; authority damage → Recovery; derived damage → Partial/rebuild; model partition compatibility strict.
    **Implementation steps:** Execute 5.3.1–5.3.7.
    **Failure/degraded behavior:** Campaign failure blocks release; system remains read-only/Partial with diagnosable cursor, not legacy fallback.
    **Focused validation:** V-LIFE-01, V-CRYPTO-01, V-REC-01, V-REBUILD-01, V-FAULT-01, V-VECTOR-01.
    **Evidence:** `evidence/F5/<run-id>/{traces/fault-campaign/,reports/deletion-residue.json,reports/rebuild-model-migration.json,reviews/crypto-recovery.json}`.
    **Completion proof:** every interruption converges, every deleted payload is absent from all supported paths, and every corruption class triggers exact safe state.
    **Rollback/containment:** Keep affected feature disabled/Recovery_Mode; reset/reimport verified KRIA data.
    **IDs:** MGR-017, MGR-040–042, MGR-045; MGD-027, MGD-036, MGD-041; MG-H16, MG-L02, MG-L11.

    - [x] 5.3.1 Inject failure at every AuthorityTx step, commit/publication boundary, DB busy state, outbox lease/apply, and verify all-or-none or post-commit convergence.
    - [x] 5.3.2 Run forget/restore/expiry/immediate-delete/source/session/namespace/tool/MCP/OpenClaw/subject cascades with independent-evidence choices.
    - [x] 5.3.3 Assert zero deleted plaintext/content through authority default/history policy, FTS, vector, graph, trace, inspector, cache, export, logs, snapshots, and stale windows after reconciliation.
    - [x] 5.3.4 Delete each derived projection and rebuild 1k/10k/100k; interrupt/resume/discard; compare count/hash/version and authority/event/revision hashes.
    - [x] 5.3.5 Run compatible/incompatible model migration, dimension/hash/tokenizer mismatch, dual partition transition, old-generation deletion, and remaining-strategy Partial behavior.
    - [x] 5.3.6 Corrupt authority page/schema/event checksum/order/revision and prove read-only Recovery_Mode; corrupt FTS/vector manifests/rows and prove authority stays usable with isolated Partial/rebuild.
    - [x] 5.3.7 Re-run crypto truth threat review: either prove subject-key plaintext denial everywhere or prove all surfaces consistently state Hard Delete/pending crypto without overclaim.

  - [x] 5.4 F5.4 — Complete interchange portability and schema evolution campaign

    **Objective:** Prove secret-free open export, atomic empty-store import, optional-field preservation, migration fixtures, and deterministic semantic round trip.
    **Targets (subject to discovery):** interchange modules, schema fixtures for every released version, independent parser, local desktop UI/action.
    **Prerequisites:** F2 interchange base; F5 release semantics frozen.
    **Invariants/non-negotiables:** Local-only current release; configured disk quota/streaming; whole package validates before commit; no unauthorized secrets; unknown required zero writes; unknown optional preserved; no compatibility write scaffolding.
    **Implementation steps:** Execute 5.4.1–5.4.6.
    **Failure/degraded behavior:** Tamper/quota/version/policy failure returns report and zero writes; interrupted export/import cleans temp data or resumes explicit job.
    **Focused validation:** V-IO-01, V-SCHEMA-01, V-FAULT-01.
    **Evidence:** `evidence/F5/<run-id>/{reports/interchange-roundtrip.json,reports/schema-evolution.json,artifacts/export-package/,reviews/portability.json}`.
    **Completion proof:** independent parse succeeds; export→empty import→export semantic/checksum comparison passes, including unknown optional fields; every prior released schema fixture upgrades.
    **Rollback/containment:** Disable import/export while retaining authority; clean failed empty target and retry.
    **IDs:** MGR-032, MGR-034, MGR-040, MGR-046; MGD-019, MGD-032, MGD-044; MG-O29.

    - [x] 5.4.1 Export authorized selected events/records/links/provenance/truth/lifecycle/order/version metadata and content files with canonical JSON/checksums and no out-of-scope secret.
    - [x] 5.4.2 Validate package with an independent parser and compare declared counts/hashes/order/required semantics before KRIA import.
    - [x] 5.4.3 Import into empty authority through one idempotent AuthorityTx after full validation; test replay, tamper, unknown required, unknown optional, quota, cancellation, and crash.
    - [x] 5.4.4 Re-export and compare semantic IDs/order/links/provenance/state/content/checksums and preserved optional extensions.
    - [x] 5.4.5 Retain deterministic fresh-create and upgrade fixture from every released schema; test upgrade→export→empty import→rebuild.
    - [x] 5.4.6 Delete migration compatibility shims and obsolete import/export formats after current round-trip evidence.

  - [x] 5.5 F5.5 — Harden observability, pressure/offline, threat, and fault behavior

    **Objective:** Make failures diagnosable without content leakage or material overhead and verify exact capability preservation under resource/network/model loss.
    **Targets (subject to discovery):** core/server/UI observability, Health, scheduler, fault harness, redaction filters.
    **Prerequisites:** F5.1–F5.3 stable behavior.
    **Invariants/non-negotiables:** Metrics/logs contain correlation and aggregates only—no content/embedding/secret/private label/hidden ID; overhead ≤1% CPU and ≤1% interactive latency or sample less; paired worlds non-interfering including timing.
    **Implementation steps:** Execute 5.5.1–5.5.6.
    **Failure/degraded behavior:** Telemetry failure never affects authority; sampling reduces on overhead; offline/pressure preserves declared floor and exact Health state.
    **Focused validation:** V-POLICY-02, V-FAULT-01, V-RESOURCE-01, V-RET-01, V-XPORT-01, threat review.
    **Evidence:** `evidence/F5/<run-id>/{security/redaction-timing.json,performance/observability-overhead.json,traces/offline-pressure/,reviews/threat.json}`.
    **Completion proof:** token/embedding/hidden-ID scanners find zero protected artifacts; overhead and paired timing bounds pass; all fault states match contracts.
    **Rollback/containment:** Reduce/disable telemetry; disable optional work/remote while retaining local core.
    **IDs:** MGR-003–004, MGR-009, MGR-017, MGR-028, MGR-043, MGR-045; MGD-020, MGD-034, MGD-037; MG-C05–C06, MG-H03, MG-O24.

    - [x] 5.5.1 Inventory every log/metric/trace/crash/UI diagnostic field and apply structured allowlist/redaction before emission/storage/export.
    - [x] 5.5.2 Run seeded protected-token/embedding/hidden-ID scans and paired-world output/timing/cache/log comparisons across Tauri/server/search/graph/path/predict/export/trace/patch.
    - [x] 5.5.3 Measure telemetry CPU/latency/storage overhead and implement adaptive sampling below the 1% budgets without dropping security/recovery events.
    - [x] 5.5.4 Campaign network/embedder/LLM/model/server/worker disconnect, DB busy, deadline/cancel, malformed/oversized row/DTO, cursor expiry, patch disorder, and optional context loss.
    - [x] 5.5.5 Campaign battery/power saver, high memory, thermal/CPU/GPU/model pressure, burst queue overflow, eventual drain, and foreground correction/search under load.
    - [x] 5.5.6 Complete threat review of server boundary, policy non-interference, prompt-injection fencing, lifecycle/crypto truth, caches/cursors/logs, and local recovery ownership.

  - [x] 5.6 F5.6 — Complete visual, device, accessibility, usability, and resource matrices

    **Objective:** Validate the production list/Canvas/inspector/destination composition across every required viewport, scale, locale, theme, input, AT, state, and prolonged interaction cycle.
    **Targets (subject to discovery):** Playwright/WebKitGTK visual/performance/a11y suites, Orca harness, screenshot semantic parser, review templates.
    **Prerequisites:** F4 evidence; F5 fixes frozen; serialize browser/Orca runs.
    **Invariants/non-negotiables:** Semantic assertions accompany pixels; no accepted diff without rationale; complete tasks remain available list-first; no clipping/focus loss/hidden state/invented encoding; resource evidence uses real release scene.
    **Implementation steps:** Execute 5.6.1–5.6.7.
    **Failure/degraded behavior:** Failed Canvas/device tier falls to minimal/list; accessibility or core workflow failure blocks release.
    **Focused validation:** V-DT-01, V-E2E-01, V-A11Y-01, V-VIS-01, V-RESOURCE-01, usability protocol.
    **Evidence:** `evidence/F5/<run-id>/{screenshots/,accessibility/,performance/,reviews/{visual-truth,accessibility,product-ux,usability}.json}`.
    **Completion proof:** matrix complete with no serious/critical axe findings, successful keyboard/Orca tasks, accepted semantic screenshots, and resource budgets.
    **Rollback/containment:** Ship list-first/minimal quality; disable divergent Canvas/animation/input mode.
    **IDs:** MGR-013–016, MGR-021–023, MGR-026–027, MGR-031; MGD-013–016, MGD-026, MGD-030, MGD-046; MG-H10–H15, MG-M24–M26.

    - [x] 5.6.1 Capture deterministic actual/reference/diff/semantic artifacts at 640×480, 800×600, 1176×775, 1440×900, 1920×1080, ultrawide and mixed DPI.
    - [x] 5.6.2 Cover 100/125/150/200%, light/dark/forced colors, LTR/RTL/CJK/long labels, mouse/keyboard/coarse touch, reduced motion, and screen reader.
    - [x] 5.6.3 Cover empty/loading/ready/partial/stale/offline/unauthorized/timeout/malformed/pending/conflict/deleted/worker failure/renderer failure/recovery states in every applicable destination.
    - [x] 5.6.4 Assert semantic JSON: no invented topology/state/score/use, exact authority/truth text, no clipped action/hidden focus/map-list mismatch, and present-only legend.
    - [x] 5.6.5 Run complete keyboard and Orca tasks for search, list/map navigation, inspect, trace, correct, relate/path, goal/resume, source consent/delete, forget/restore/delete, and focus return.
    - [x] 5.6.6 Profile frame/idle/CPU/GPU/heap/GC/worker/queue through 20 navigation/inspect/write/delete cycles and competing local-model load; verify steady recovery and quality ladder.
    - [x] 5.6.7 Conduct task-based human review/usability study for find, explain, correct, forget, source consent, and recovery; record confusion/errors without substituting opinion for executable evidence.

  - [x] 5.7 F5.7 — Produce exact supply-chain, license, SBOM, vulnerability, and project-license evidence

    **Objective:** Ensure every shipped Rust/npm/Python/model/asset component is pinned, checksummed, reachable, FOSS-reviewed, and represented in release SBOMs.
    **Targets (subject to discovery):** Cargo/npm/Python locks, model/asset manifests, root/project license declarations, release scripts/CI, new pinned `CMD-MG-SBOM` implementation.
    **Prerequisites:** Release closure frozen; no F6 dependency included.
    **Invariants/non-negotiables:** Exact versions/checksums; SPDX and CycloneDX; application/sidecar/runtime/model/assets; unknown/incompatible blocks; vulnerability has severity/reachability/owner/expiry; project license conflict resolved explicitly.
    **Implementation steps:** Execute 5.7.1–5.7.6.
    **Failure/degraded behavior:** Missing metadata/license/checksum or unowned reachable vulnerability blocks release inclusion; remove component/capability rather than guess.
    **Focused validation:** V-SBOM-01, clean reproducible release closure, SBOM schema/checksum/license-policy tests.
    **Evidence:** `evidence/F5/<run-id>/supply-chain/{sbom.spdx.json,sbom.cdx.json,licenses.json,vulnerabilities.json,model-assets.json,project-license-review.json}`.
    **Completion proof:** scanners and human Supply Chain/owner review account for 100% shipped closure with zero unknown disposition.
    **Rollback/containment:** Remove offending dependency/asset/model/capability and regenerate; do not waive unknown/incompatible license.
    **IDs:** MGR-027, MGR-036, MGR-047–048; MGD-029, MGD-039, MGD-045; MG-C01, MG-M21, MG-M27.

    - [x] 5.7.1 Pin and verify direct/transitive Cargo/npm/Python packages, FastEmbed/runtime/model/tokenizer artifacts, UI assets/fonts/icons, and build tools used in shipped closure.
    - [x] 5.7.2 Reconcile Cargo metadata/root LICENSE/docs and record approved project-license disposition without inferring legal facts.
    - [x] 5.7.3 Implement reproducible SBOM command and emit valid SPDX plus CycloneDX including application, sidecars, model runtime/artifacts, assets, and platform-specific closure.
    - [x] 5.7.4 Apply reviewed FOSS allow/deny policy and record license text/source/checksum/reachability/disposition for every component.
    - [x] 5.7.5 Generate vulnerability report with severity, affected version, runtime reachability, mitigation, owner, due/expiry, and release-block disposition.
    - [x] 5.7.6 Tamper/missing-component test the SBOM gate, obtain independent Supply Chain plus owner/legal review, and checksum the bundle.

  - [x] 5.8 F5.8 — Run regression/release review, delete dead paths, and sign F5

    **Objective:** Finish hard cutover, documentation/runtime parity, risk closure, independent sign-offs, and the machine-checkable public-ready manifest.
    **Targets (subject to discovery):** all superseded schemas/stores/routes/adapters/UI/renderers/tests/dependencies/docs; release evidence/CI command; no application feature expansion.
    **Prerequisites:** F5.1–F5.7 pass on frozen inputs.
    **Invariants/non-negotiables:** No blanket golden updates; no dead compatibility scaffolding; no false Current/crypto/3D/semantic/confidence/community claims; zero blocking risk; task boxes are not release proof.
    **Implementation steps:** Execute 5.8.1–5.8.8.
    **Failure/degraded behavior:** Any regression, orphan, stale live registration, missing review, dirty unrecorded tree, or blocking risk prevents manifest Pass.
    **Focused validation:** V-REG-01 plus all F0–F5 suites, coverage/orphan linter, dead-code/dependency/runtime-doc checks.
    **Evidence:** complete immutable `evidence/F5/<run-id>/` and signed release reviews.
    **Completion proof:** F5 manifest chain validates and production-ready authoritative 2D/list product stands without F6.
    **Rollback/containment:** Independent safe capability disables/read-only/reset only; no legacy revival.
    **IDs:** MGR-001, MGR-027–029, MGR-032, MGR-047–048 and all inherited mappings; MGD-018–022, MGD-042, MGD-045–046; MG-M27–M28.

    - [x] 5.8.1 Run targeted then complete Rust core/desktop/server/cognition, TypeScript unit, integration/E2E/adversarial, schema/contract/security/retrieval/fault/resource/a11y/visual/supply-chain suites on frozen release commit.
    - [x] 5.8.2 Review every golden change against a requirement/decision and reject blanket updates or mocked success that weaken semantics.
    - [x] 5.8.3 Delete superseded schema objects/migrations assumptions, stores, free-text/ANN paths, routes/commands, adapter business logic, client models/global state, SVG/renderer business logic, dead 3D controls, tests, assets, dependencies, and flags without owner/removal condition.
    - [x] 5.8.4 Re-run registration/write/read/dependency/dead-code inventories and prove one authority, one public API, one scene/action model, one complete list path, and conditional Canvas only.
    - [x] 5.8.5 Reconcile developer/product/runtime docs and capability copy with actual current behavior, exact limits, degradation, crypto state, evidence links, and no 3D launch promise.
    - [x] 5.8.6 Run machine coverage: 48/48 MGR, 46/46 MGD, 65/65 findings, 31/31 opportunities; zero orphan suite/risk/workstream/artifact/command/fixture; every claimed Pass points to checksummed evidence, not a box.
    - [x] 5.8.7 Close or explicitly block on all Critical/High truth/privacy/security/lifecycle/integrity/accessibility/supply-chain risks; no waiver may override P0/unknown license/policy leak/false erasure/corruption.
    - [x] 5.8.8 Obtain independent Backend, Security/Privacy, Data Integrity, Retrieval, Cognition, API, Product/UX, Accessibility, Visual Truth, Performance, Supply Chain, and Release sign-offs; validate F0→F5 predecessor hashes and sign F5 manifest.

- [x] 6. F6 — Optional true-3D preregistered GO or complete deletion

  **Objective:** After signed F5 only, determine whether one authority-backed semantic z-axis improves one real user task; either ship one scene/action-compatible optional renderer or delete every 3D artifact and claim.
  **Targets (subject to discovery):** current dormant `graph/{GraphCanvas3D,GraphScene,layout.worker,layoutSettle,lensController}.*`, optional dependencies/assets/tests/docs; do not alter F5 authority/API semantics.
  **Prerequisites:** signed F5 manifest, zero blocking risk, preregistration completed before implementation, separate isolated branch/workstream and serialized study/profile runs.
  **Invariants/non-negotiables:** Same Semantic Scene/actions/list fallback; no duplicate semantics; one authority-backed z-axis; ≥10% median task-time or error improvement, ≥30 FPS real scene, idle/resource/a11y/license parity; F5 product remains default/complete.
  **Implementation steps:** Complete F6.1 before F6.2; then execute exactly one F6.3 GO or NO-GO result.
  **Failure/degraded behavior:** Any failed/ambiguous threshold is NO-GO and requires complete deletion; context loss falls to 2D/list; no experimental shipped-looking state.
  **Focused validation:** V-3D-01 plus V-A11Y-01, V-RESOURCE-01, V-SBOM-01 and predecessor check.
  **Evidence:** `evidence/F6/<run-id>/{manifest.json,study/,performance/,accessibility/,supply-chain/,reviews/,reports/deletion-diff.json}`.
  **Completion proof:** Either all GO thresholds/sign-offs pass with identical actions, or repository/dependency/asset/test/control/claim inventory proves zero 3D residue.
  **Rollback/containment:** Mandatory complete deletion on failure/ambiguity; F5 product unchanged.
  **IDs:** MGR-012, MGR-014, MGR-022, MGR-027, MGR-030, MGR-047–048; MGD-002, MGD-016, MGD-021, MGD-028–029, MGD-042, MGD-045–046; MG-C01, MG-H02, MG-M20–M23, MG-L12, MG-O20.

  - [x] 6.1 F6.1 — Preregister task, semantic z-axis, cohort, measures, thresholds, and deletion rule

    **Objective:** Prevent novelty bias and post-hoc success criteria before any technical implementation.
    **Targets (subject to discovery):** F6 study protocol/evidence schema only; no app code.
    **Prerequisites:** F5 signed manifest and real F5 scene fixture.
    **Invariants/non-negotiables:** One user task; one authority-backed z-axis; 2D baseline; participant/cohort/exclusions; task-time/error primary outcome; all GO/a11y/perf/resource/license thresholds frozen; ambiguity = delete.
    **Implementation steps:** Execute 6.1.1–6.1.5.
    **Failure/degraded behavior:** Missing authority meaning/sample protocol/reviewer approval prevents spike.
    **Focused validation:** predecessor/registry/manifest protocol lint and Product/A11y/Performance preregistration review.
    **Evidence:** `evidence/F6/<run-id>/study/preregistration.json` with F5 hash and signatures.
    **Completion proof:** immutable preregistration hash exists before first 3D implementation commit.
    **Rollback/containment:** Stop F6 with NO-GO/no code change.
    **IDs:** MGR-027, MGR-030, MGR-048; MGD-016, MGD-028, MGD-042, MGD-046; MG-C01.

    - [x] 6.1.1 Select one current F5-supported user task and state why depth could improve it versus complete 2D/list baseline.
    - [x] 6.1.2 Define one z-axis derived only from authority-backed semantics; reject decorative depth, centrality/confidence inference, or generated topology as meaning.
    - [x] 6.1.3 Freeze participants/cohort, fixtures, training, counterbalancing, exclusions, task-time/error measures, analysis, and context-loss/a11y observation protocol.
    - [x] 6.1.4 Freeze GO thresholds: ≥10% median time or error benefit, ≥30 FPS target WebKitGTK real scene, idle quiet, resource/a11y/core-task parity, approved FOSS closure.
    - [x] 6.1.5 Freeze mandatory deletion inventory and rule that any miss, ambiguity, maintenance/supply-chain failure, or semantic divergence is NO-GO.

  - [x] 6.2 F6.2 — Build isolated technical spike using identical scene/actions

    **Objective:** Measure real integrated 3D rendering/interactions without creating a second domain or changing F5 contracts.
    **Targets (subject to discovery):** adapt current dormant 3D files only; optional worker/dependency behind non-shipping spike registration; same `SemanticScene`/action controller/list.
    **Prerequisites:** immutable F6.1 preregistration.
    **Invariants/non-negotiables:** Packed transferable buffers; integrated LOD/culling; bounded labels/collision; camera fit/focus/presets; keyboard/touch/comfort; static reduced motion; context recovery; idle freeze; 2D/list fallback.
    **Implementation steps:** Execute 6.2.1–6.2.7.
    **Failure/degraded behavior:** Capability/context failure immediately restores same query/focus/action state in 2D/list; no lost correction/pending state.
    **Focused validation:** V-3D-01 technical slices, scene/action hash parity, real WebKitGTK profiles, context loss, V-A11Y/V-RESOURCE/V-SBOM.
    **Evidence:** `evidence/F6/<run-id>/{performance/real-scene.json,accessibility/parity.json,traces/context-loss/,supply-chain/}`.
    **Completion proof:** spike is study-ready, scene/action-identical, bounded, and removable as one isolated diff.
    **Rollback/containment:** Remove spike registration/dependency immediately; F5 untouched.
    **IDs:** MGR-012, MGR-014, MGR-022, MGR-030, MGR-047; MGD-002, MGD-016, MGD-029, MGD-046; MG-H02, MG-M20–M23, MG-L12.

    - [x] 6.2.1 Feed the exact F5 Semantic Scene and authorized actions into 3D; assert semantic collection/action hashes equal 2D/list for the same snapshot/session/capabilities.
    - [x] 6.2.2 Implement authority-backed z mapping, deterministic positions, packed reusable transferable buffers, bounded worker lifecycle, and no renderer-owned truth/policy/layout persistence.
    - [x] 6.2.3 Implement integrated node/edge/label LOD, frustum/cap culling, bounded dirty label/collision updates, and exact scene/truncation caps.
    - [x] 6.2.4 Implement camera fit/focus/presets/back/forward, keyboard actions, touch policy, depth comfort, focus indication, selected/offscreen continuity, and list synchronization.
    - [x] 6.2.5 Implement reduced-motion static mode, quality ladder, idle render stop, device/context loss recovery to identical 2D query/focus/pending state.
    - [x] 6.2.6 Profile real nodes/edges/labels/layout/compositing/interaction/context loss under local-model contention for FPS/frame/CPU/GPU/RAM/VRAM/heap/GC/idle.
    - [x] 6.2.7 Inventory exact optional code/controls/dependencies/assets/tests/docs so NO-GO deletion can be machine-verified.

  - [x] 6.3 F6.3 — Run preregistered study and execute GO or mandatory NO-GO deletion

    **Objective:** Produce one clean evidence-backed outcome with no indefinite experimental state.
    **Targets (subject to discovery):** study/evidence artifacts; on GO, capability-gated optional renderer; on NO-GO, all inventory from 6.2.7.
    **Prerequisites:** F6.2 technical evidence passes enough to run study without changing thresholds.
    **Invariants/non-negotiables:** Analyze exactly preregistered measures; all GO thresholds conjunctive; 2D remains default/complete; failed or ambiguous result deletes everything optional.
    **Implementation steps:** Execute 6.3.1–6.3.6 and one branch only.
    **Failure/degraded behavior:** Study interruption/bias/data insufficiency/threshold miss = NO-GO deletion.
    **Focused validation:** complete V-3D-01 and final coverage/dependency/claim inventory.
    **Evidence:** signed F6 manifest, study, profiles, reviews, and GO integration diff or NO-GO deletion diff.
    **Completion proof:** GO has Product/A11y/Performance/Supply Chain sign-offs and all thresholds; NO-GO has zero code/control/dependency/asset/test/doc/marketing residue.
    **Rollback/containment:** Delete 3D completely and return unchanged F5 product.
    **IDs:** MGR-027, MGR-030, MGR-047–048; MGD-016, MGD-028–029, MGD-045–046; MG-C01, MG-O20.

    - [x] 6.3.1 Run counterbalanced 2D/list versus 3D study on preregistered cohort/fixtures and preserve anonymized raw task-time/error/context/a11y observations.
    - [x] 6.3.2 Analyze only preregistered metrics and report median effect, uncertainty, exclusions, errors, learning/order effects, and ambiguity without post-hoc threshold changes.
    - [x] 6.3.3 Re-run target WebKitGTK real-scene ≥30 FPS, idle/resource/context-loss, keyboard/Orca/core-task parity, and optional dependency license/SBOM checks on study commit.
    - [x] 6.3.4 GO branch only if every threshold passes: capability-gate optional renderer, retain 2D default/list completeness, document semantic z-axis, and sign all required reviews.
    - [x] 6.3.5 NO-GO branch on any miss/ambiguity: delete renderer, worker, controls, flags, dependencies, assets, tests, docs/marketing claims, and any graph-only abstraction; regenerate locks/SBOM.
    - [x] 6.3.6 Run machine inventory and F0→F6 manifest validation proving either qualified optional view or complete deletion, with F5 public readiness unchanged.

## Final Definition of Done — Evidence, Not Checkboxes

The feature is Done only when the release verifier reads manifests and confirms every item below. Checkbox state is ignored.

- [x] DOD.1 The manifest predecessor chain is valid and immutable: F0→F1→F2→F3→F4→F5; F6 is absent, verified GO, or verified complete NO-GO deletion.
- [x] DOD.2 Every applicable suite in `validation.md` has exact command/cwd/exit code, commit/dirty digest, fixtures/seeds/hashes, assertion totals, artifacts/checksums, metrics, and mandatory independent reviews.
- [x] DOD.3 Machine coverage reports exactly requirements `48/48`, decisions `46/46`, findings `65/65`, opportunities `31/31`, with zero missing, duplicate, out-of-range, forward orphan, reverse orphan, invalid gate, or checklist-only Pass.
- [x] DOD.4 SQLite v2 is the only writable authority; all durable sources cross WritePolicyEngine/AuthorityTx; events/audits/revisions are immutable; idempotency/atomicity/policy/Recovery properties pass.
- [x] DOD.5 Canonical typed records, provenance, Memory Links, truth/time, entity/source lifecycle, interchange, retrieval traces, and deletion/rebuild semantics pass independent round-trip and fault oracles.
- [x] DOD.6 Five-strategy retrieval passes exact vector/FTS/graph/time/goal properties, ≥200 judged thresholds, forbidden/deletion exclusions, trace injected-order equality, and 100k correctness/performance budgets.
- [x] DOD.7 Canonical v2 operation/error/limit/cursor/patch contracts have Tauri/Axum parity; old APIs/full-refresh/direct SQL/parallel stores are absent from live registrations.
- [x] DOD.8 All seven destinations and find/inspect/explain/correct/merge/split/relate/path/goal/source/forget/restore/delete/offline/recovery workflows pass without Canvas; any enabled Canvas has exact list/action parity.
- [x] DOD.9 Visual matrix, semantic assertions, axe, keyboard, Orca, focus/input/zoom/forced-color/RTL/CJK reviews, frame/idle/heap/resource budgets, and human Visual Truth/Accessibility/Product reviews pass.
- [x] DOD.10 Deletion residue, corruption, rebuild/model migration, pressure/offline, multi-window, interchange, regression, threat, observability, and release campaigns pass with zero blocking risk.
- [x] DOD.11 SPDX/CycloneDX SBOMs, exact locks/checksums, model/asset manifests, project-license resolution, FOSS dispositions, and reachable-vulnerability ownership are complete with zero unknown release component.
- [x] DOD.12 Dead schemas/stores/routes/adapters/UI models/renderers/tests/dependencies/flags and false documentation claims are deleted; runtime, docs, requirements, traceability, and evidence status agree.
- [x] DOD.13 The release manifest—not this list—contains final Backend, Security/Privacy, Data Integrity, Retrieval, Cognition, API, Product/UX, Accessibility, Visual Truth, Performance, Supply Chain, and Release verdicts over reviewed artifact hashes.

**Machine-checkable completion command requirement:** the implemented coverage/release command must exit nonzero unless DOD.1–DOD.13 are derivable from valid manifest data and artifact hashes. It must not parse checked boxes as proof, and it must report the exact missing edge/artifact/review/threshold on failure.
