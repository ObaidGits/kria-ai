# Memory Graph Production Redesign — Validation and Evidence Contract

**Status:** Planned/Unverified. This document defines executable and reviewed evidence required for MGR-001–MGR-048 and MGD-001–MGD-046; it records no pass and marks no implementation complete.
**Order:** F0 → F1 → F2 → F3 → F4 → F5; optional F6 starts only after F5. A later artifact cannot waive an earlier P0 failure.

## 1. Evidence Rules

1. A checklist, task checkbox, code review, screenshot alone, dormant code, comment, type signature, or successful broad build is **not proof** of behavior. Proof is a reproducible command result plus assertions and artifacts; visual, accessibility, recovery, crypto, license, and usability claims also require named human review.
2. Every result is `Planned`, `Pass`, `Fail`, `Blocked`, or `NotApplicable`. This file remains Planned/Unverified; status changes belong in generated evidence and require linked commit-specific artifacts.
3. Security, policy, deletion, corruption, and migration fixes are forward-only. Validation may disable an unsafe capability or reset pre-production KRIA data; it must never certify rollback to permissive auth, direct writes, dual authority, client-only filtering, false erasure, or stale schema.
4. Latency evidence is invalid unless correctness assertions pass in the same run. Pixel diffs are invalid as semantic proof without reviewer confirmation. Aggregated counts are invalid if hidden records can change observable output.
5. Target commands below are contracts to implement where absent. Only commands labeled **existing** are known repository targets; their existence does not imply the named suite exists or passes.

## 2. Deterministic Fixture Contract

| Fixture ID | Seed | Size/purpose | Required planted cases |
|---|---:|---|---|
| `mg-unit-v2` | `0x4D475201` | 100 records; properties and schema | every record/link kind, all truth/memory modes, policy lattice, invalid rows, duplicate idempotency keys |
| `mg-small-v2` | `0x4D475202` | 1,000 records; API/UI | seven destinations, long/RTL/CJK labels, empty/partial/stale/offline/recovery states, traces and corrections |
| `mg-medium-v2` | `0x4D475203` | 10,000 records; faults/rebuild | outbox backlog, model partitions, corruption sentinels, import extensions, source cancellation |
| `mg-release-v2` | `0x4D475204` | 100,000 authority records; release scale | fixed degree distribution, cycles, hidden intermediaries, temporal boundaries, 1/2/3/4-hop paths, exact expected memberships |
| `mg-policy-pairs-v2` | `0x4D475205` | paired worlds | worlds differ only by unauthorized content, labels, IDs, counts, topology, timings, cache/log inputs |
| `mg-vector-oracle-v2` | `0x4D475206` | numeric oracle | normalized/non-normalized vectors, ties, zero norm, NaN/Inf, wrong 1536-byte length/model/dimension |
| `mg-retrieval-judged-v2` | `0x4D475207` | ≥200 judged queries | identifier, phrase, semantic, entity/relation, temporal, goal, contradiction, source, forbidden/adversarial strata |
| `mg-interchange-v2` | `0x4D475208` | portable package | all required semantics, unknown optional fields, unknown required field, checksums, no secrets, empty-store round trip |
| `mg-visual-v2` | `0x4D475209` | semantic scene | deterministic revisions/states at all matrix viewports; no random layout, clock, network, animation, font drift |

Generators live at target path `tests/fixtures/memory-graph/generators/`; generated packages live under `tests/fixtures/memory-graph/generated/<fixture-id>/<generator-version>/`. Each emits `fixture-manifest.json` with generator commit/version, seed, record counts by kind/policy/truth, expected membership hash, expected relation/path answers, schema/model/ontology versions, and file SHA-256. Property suites run at least 100 generated cases, persist the exact seed and minimized counterexample, and never regenerate golden expectations from the system under test.

## 3. Evidence Artifact and Manifest Schema

Canonical run root: `.kiro/specs/memory-graph-production-redesign/evidence/<gate>/<run-id>/`. Large generated artifacts may be stored elsewhere only when the manifest contains an immutable URI and checksum.

```text
manifest.json
commands/<suite-id>.json
junit/<suite-id>.xml
logs/<suite-id>.jsonl
reports/<suite-id>.json
traces/<suite-id>/
fixtures/<fixture-id>.json
screenshots/<suite-id>/<case-id>/{actual,reference,diff,semantic}.png|json
accessibility/<suite-id>/{axe,keyboard,orca}.json|md
performance/<suite-id>/{samples,summary,query-plans,resource-trace}.json
security/<suite-id>/{paired-worlds,redaction,timing}.json
supply-chain/{sbom.spdx.json,sbom.cdx.json,licenses.json,vulnerabilities.json}
reviews/<role>.json
```

`manifest.json` SHALL contain: `schemaVersion`, `runId`, `gate`, `status`, UTC start/end, commit, branch, dirty-state digest, actor, exact command/working directory/exit code, requirement IDs, decision IDs, suite IDs, fixture IDs/seeds/generator hashes, authority schema/ontology/model/RRF/scene versions, lockfile and binary hashes, OS/kernel/WebKitGTK/runtime/build profile, reference-hardware ID (CPU/RAM/GPU/storage/display/DPI), power/thermal/network state, warm/cold protocol, locale/theme/input/AT, artifact `{path,mediaType,sha256,size}`, assertion totals, counterexamples, metric samples/intervals, reviewer records, waivers, and predecessor-manifest hashes. A manifest fails validation if IDs are unknown, artifacts are missing/checksum-invalid, required fields are null, the tree is dirty without a recorded digest, or a claimed Pass lacks required sign-off.

## 4. Command Catalog

| Command ID | Status | Command / working directory | Intended use |
|---|---|---|---|
| `CMD-RUST-UNIT` | existing target | `just test` / repository root | workspace library regression; not sufficient alone |
| `CMD-COGNITION` | existing target | `just test-cognition` / repository root | cognition regression; not sufficient alone |
| `CMD-GUI-E2E` | existing target | `just test-e2e` / repository root | sandboxed GUI E2E |
| `CMD-ADVERSARIAL` | existing target | `just test-adversarial` / repository root | adversarial GUI cases |
| `CMD-UI-UNIT` | existing target | `npm run test:run` / `ui/` | frontend unit/component tests |
| `CMD-UI-E2E` | existing target | `npm run e2e` / `ui/` | Playwright E2E |
| `CMD-UI-A11Y` | existing target | `npm run e2e:a11y` / `ui/` | current accessibility target; suite content must be extended |
| `CMD-UI-PERF` | existing target | `npm run e2e:performance` / `ui/` | current performance target; suite content must be extended |
| `CMD-MG-CORE` | planned target | `cargo test -p kria-core --test memory_graph_v2 -- --nocapture` | authority/schema/policy/semantic suites |
| `CMD-MG-EVAL` | planned target | `cargo run -p kria-eval -- memory-graph --manifest <run-root>/manifest.json` | fixtures, retrieval, performance, artifact emission |
| `CMD-MG-CONTRACT` | planned target | `cargo test -p kria-desktop --test memory_v2_contract` and `cargo test -p kria-server --test memory_v2_contract` | normalized transport parity |
| `CMD-MG-VISUAL` | planned target | `npm run e2e -- memory-control-center.visual.spec.ts` / `ui/` | deterministic visual-semantic matrix |
| `CMD-MG-ORCA` | planned target | `npm run e2e -- memory-control-center.orca.spec.ts` / `ui/` | Orca transcript and keyboard tasks |
| `CMD-MG-SBOM` | planned target | repository release evidence command, to be added and pinned | SBOM/license/vulnerability production |

## 5. Executable Evidence Matrix

| Suite | Exact behavior/assertions | Fixtures | Target command | Artifact class | Gate / reviewer |
|---|---|---|---|---|---|
| `V-AUTH-01` | Inject failure before/after semantic row, immutable Event, Audit, outbox, idempotency result and revision writes; assert all-or-none, exactly one revision for graph-visible commit, no revision for rejection, publication recovery after commit | unit + small | CMD-MG-CORE | junit, SQL state hashes, crash trace | F1; Backend + Data Integrity |
| `V-AUTH-02` | Attempt Event/graph revision/audit UPDATE and DELETE through SQL and public ports; all reject; checksum/HLC ordering remains unchanged | unit | CMD-MG-CORE | junit, trigger/error transcript | F1; Data Integrity |
| `V-AUTH-03` | Same partition/key/hash returns byte-equivalent original result and no second semantic row/event/revision; same key/different hash conflicts; concurrent replay converges once | unit, ≥100 schedules | CMD-MG-CORE | property counterexamples, DB hashes | F1; Backend |
| `V-SCHEMA-01` | Fresh create and every released schema migration/reset: checksums, PK/FK/CHECK/UNIQUE/index/trigger inventory, pragmas, canonical UUID/time/JSON, unknown version rejection, no dual authority | all schema fixtures | CMD-MG-CORE | schema diff, migration report | F1/F5; Data Integrity |
| `V-POLICY-01` | Most-restrictive meet is associative/commutative/idempotent; empty capability/scope intersection denies; no derivation declassifies absent audited command; every durable source uses Write Policy and memory-mode semantics | policy pairs | CMD-MG-CORE | property report, write-path inventory | F1/F2; Security |
| `V-POLICY-02` | Paired-world non-interference across planning, labels, IDs, counts, ranks, topology, cursor/cache keys, DTOs, logs and deny responses; timing distributions must remain within preregistered equivalence bound and never encode hidden cardinality | policy pairs at 100/1k/10k | CMD-MG-EVAL + CMD-MG-CONTRACT | paired outputs, statistical report | F1/F5; Security + Privacy |
| `V-GRAPH-01` | BFS/path over cycles terminates; depth 0–3 exact; depth 4 rejected/truncated; no repeated path node; visited/edge/deadline caps; hidden intermediary removes whole path; frontier has no protected identifier/count/topology | release cycles | CMD-MG-CORE | property report, query plans | F2/F3; Backend + Security |
| `V-VECTOR-01` | Decode exactly 384 finite `f32le` (1536 bytes), reject wrong model/hash/dimension/zero norm/NaN/Inf; exact search equals independent scalar f64 cosine oracle and stable score-desc/ID tie order for every candidate | vector oracle + 100k | CMD-MG-EVAL | per-vector comparison, manifest | F3; Retrieval |
| `V-RET-01` | Independently exercise FTS5, exact vector, ≤3-hop graph, temporal and active-goal strategies; apply policy/truth/version gates before fusion; unavailable strategy reports Partial without redistributing weight | judged + policy | CMD-MG-EVAL | trace corpus, ablations | F3; Retrieval + Security |
| `V-RET-02` | Recompute weighted RRF from stored one-based ranks, availability, profile weights and k; deterministic tie break; diversity/token packing caps; injected order equals `Used` trace exactly | judged | CMD-MG-EVAL | replayable RRF worksheet, traces | F3; Retrieval |
| `V-RET-03` | ≥200 adjudicated queries, two judges or recorded adjudication; report Recall@10 ≥0.85, nDCG@10 ≥0.80, identifier/phrase ≥0.95, forbidden=100%, Deleted/Forgotten/default-Superseded exclusion=100%, per-class/ablation/95% bootstrap CI, no >0.03 absolute accepted-profile regression outside CI | judged v2 | CMD-MG-EVAL | judgments, metrics, CIs, profile activation record | F3/F5; Retrieval + Product |
| `V-SEM-01` | Typed records/links/provenance serialize and round-trip; canonical link types only; registry direction/reflexivity/endpoints/identity/evidence/Valid Time enforced; duplicate observation appends evidence, not edge | unit/small | CMD-MG-CORE | property/golden report | F2; Domain |
| `V-TRUTH-01` | Current/history predicates cover open/exact boundaries/timezones; contradiction and supersession preserve sources; correction creates governed lineage; absent authority becomes Unavailable, never inferred presentation | unit/small | CMD-MG-CORE | temporal/truth goldens | F2/F3; Domain + Truth |
| `V-ENTITY-01` | Strong identifiers may resolve; names/fuzzy/vector only propose; merge/split preview uses base revision, preserves mentions/provenance/policy, reversal restores exact partition and links; stale preview fails | small | CMD-MG-CORE + CMD-UI-E2E | round-trip hashes, E2E video | F2/F4; Domain + UX |
| `V-CONS-01` | Episode→Summary→Skill→Rule identity is sorted-parent hash + algorithm/version; replay/crash resume is idempotent; immediate lineage inspectable; restrictive policy propagates; insufficient/self-only evidence never promotes | medium | CMD-COGNITION + CMD-MG-EVAL | run ledger, lineage graph | F3/F5; Cognition + Security |
| `V-TOOL-01` | Native/MCP/OpenClaw/sidecar invocation start/completion correlate once; failure/success remain observations; n<20 is inert; n≥20 only affects named allowed policy; cannot grant capability, broaden scope, promote/delete, or mutate RRF online | medium/policy pairs | CMD-COGNITION + CMD-MG-EVAL | observation ledger, negative assertions | F3/F5; Cognition + Security |
| `V-LIFE-01` | Forget excludes default reads and restores same ID within window; delete preview dependencies are revision-bound; after reconciliation zero content through FTS/vector/graph/trace/inspector/cache/export; interrupted purge resumes | medium/release | CMD-MG-EVAL + CMD-UI-E2E | residue matrix, outbox trace | F1/F5; Privacy + Data Integrity |
| `V-CRYPTO-01` | If plaintext events/snapshots/caches remain, UI/API must say Deleted/Reconciled or pending crypto, never Crypto-Shredded; capability may pass only after subject-key destruction makes all current/history/snapshot/index/cache paths return no plaintext under threat review | crypto fixture | CMD-MG-EVAL | plaintext-denial report, threat review | F1/F5; Security + Privacy |
| `V-REC-01` | Seed authority page/schema/event checksum/order corruption: startup enters read-only Recovery_Mode, no durable writes, policy-safe diagnostic only; only fully verified restore/import exits. Seed FTS/vector corruption: authority remains available, capability Partial, affected generation rebuilt | medium corruptions | CMD-MG-EVAL | fault log, mode transitions | F1/F5; Recovery + Security |
| `V-REBUILD-01` | Delete each derived projection; rebuild revision-ordered into temp generation, interrupt/resume/discard, compare member count/hash/version, atomic activate; authority/event/revision hashes unchanged | 1k/10k/100k | CMD-MG-EVAL | before/after manifests, cursor trace | F1/F5; Data Integrity |
| `V-IO-01` | Policy-selected secret-free export parses independently; empty-store import validates whole manifest then commits once; export→import→export preserves authority semantics/checksums and unknown optional fields; unknown required/tampered checksum fails with zero writes | interchange | CMD-MG-EVAL | package, parser report, round-trip diff | F2/F5; Data Portability + Security |
| `V-XPORT-01` | For each supported operation, Tauri/Axum normalized status/error/capability/DTO/limits/revision are equal; auth context differs only at adapter boundary; unsupported operation explicit; remote negative matrix covers anonymous/origin/scope/replay/oversize | small/policy | CMD-MG-CONTRACT | host golden diff | F3/F5; API + Security |
| `V-UI-UNIT-01` | Runtime DTO rejection, generation/policy/revision discard, duplicate/reordered/missing patch convergence, pending rollback, per-window ownership, scene purity, map/list item/action hash parity, camera/culling/hit-test reducers | visual/small | CMD-UI-UNIT | Vitest/JUnit, reducer traces | F4; Frontend |
| `V-DT-01` | Overview, Recall, Knowledge, Timeline, Goals, Sources and Health each render real capability/state and one-revision DTOs; unavailable capability omitted/labeled; list-first find/inspect/explain/correct/merge/split/relate/path/goal/source/lifecycle workflows complete without Canvas | small | CMD-UI-E2E | videos, snapshots, action transcript | F4; Product + UX |
| `V-E2E-01` | End-to-end authority write→revision→patch/refetch→scene/list/inspector plus offline, partial, stale, conflict, malformed, timeout, renderer/worker failure, delete and Recovery_Mode; no simulated success | small/medium | CMD-GUI-E2E + CMD-UI-E2E | Playwright trace/video, DB hashes | F4/F5; QA + Domain |
| `V-REG-01` | Existing memory, cognition, server, desktop and UI suites remain green; changes in goldens are requirement-linked and reviewed, never blanket-updated | repository fixtures | CMD-RUST-UNIT + CMD-COGNITION + CMD-UI-UNIT | regression report | every gate; owning leads |
| `V-FAULT-01` | Failure at every authority step, commit/publication boundary, DB busy, outbox/rebuild/model migration, cursor expiry, patch loss/reorder/duplicate, scope change, model/LLM loss, worker/server disconnect, malformed/oversized row and optional context loss yields specified rollback/Partial/stale/refetch/Recovery behavior | medium | CMD-MG-EVAL + CMD-ADVERSARIAL | fault matrix, invariant hashes | F1–F5; Reliability |
| `V-A11Y-01` | Axe has no serious/critical violations; complete keyboard and Orca scripts for search, list/map navigation, inspect, trace, correct, relate, forget/restore, path and focus return; one composite tab stop, 44px targets, 200%, forced colors, reduced motion; map/list/table semantics/actions match | visual matrix | CMD-UI-A11Y + CMD-MG-ORCA | axe JSON, key log, Orca transcript/video | F4/F5; Accessibility reviewer |
| `V-VIS-01` | Deterministic screenshots at 640×480, 800×600, 1176×775, 1440×900, 1920×1080, ultrawide; 100/125/150/200%, light/dark/forced colors, LTR/RTL/CJK, all states. Semantic JSON asserts no invented topology/state/score, no clipped action/hidden focus/map-list mismatch; reviewer explains each accepted diff | visual v2 | CMD-MG-VISUAL | image triplets + semantic JSON | F4/F5; Visual + Truth + A11y |
| `V-PERF-01` | Same correctness run measures ≥30 warm iterations plus separate cold: core retrieval ≤120ms p95, Control Center search ≤250ms, one-hop ≤500ms, prediction ≤750ms; bootstrap 95% CI; query plans reject corpus adjacency scans | release 100k | CMD-MG-EVAL | samples/CIs/plans | F3/F5; Performance |
| `V-RESOURCE-01` | No async blocking span >50ms; foreground preempts/defer cognition ≤100ms; frame p95 ≤33.3ms; animation/render loop stops ≤2s idle; 60s idle CPU delta ≤2pp; bounded queues; heap returns to declared steady band after 20 cycles; quality ladder preserves truth/list/actions | release/visual | CMD-UI-PERF + CMD-MG-EVAL | CPU/GPU/heap/frame/queue traces | F4/F5; Performance + UX |
| `V-SBOM-01` | Exact Cargo/npm/Python/model/asset locks and checksums; SPDX and CycloneDX include release closure; each FOSS disposition reviewed; project-license conflict resolved; vulnerability report has severity, reachability, owner, expiry; missing/unknown blocks | release closure | CMD-MG-SBOM | SBOM/license/vuln bundle | F1/F5/F6; Supply Chain + owner/legal |
| `V-3D-01` | Only after F5: preregister task/z-axis; same scene/actions/list fallback, ≥10% median task-time/error gain, ≥30 FPS on reference WebKitGTK, idle/resource/a11y parity and approved supply chain. Any miss proves NO-GO and complete code/control/dependency/asset/test/claim deletion | real F5 scene | planned F6 target | study, profiles, deletion diff | F6; Product + A11y + Performance |

## 6. Phase Gates and Required Sign-off

| Gate | Required suites/artifacts | Mandatory sign-off |
|---|---|---|
| F0 Evidence reset | manifest/fixture validators; MGR/MGD/audit orphan check; current-vs-planned claim inventory; baseline limitations | Spec owner, QA/evidence owner |
| F1 Authority/security/lifecycle/recovery | V-AUTH-01..03, V-SCHEMA-01, V-POLICY-01..02, V-LIFE-01, V-CRYPTO-01, V-REC-01, initial V-REBUILD/V-FAULT/V-SBOM | Backend, Security/Privacy, Data Integrity |
| F2 Semantics/truth/entities/sources | V-GRAPH-01, V-SEM-01, V-TRUTH-01, V-ENTITY-01, source/consent cases in V-IO/V-E2E | Domain, Security/Privacy |
| F3 Retrieval/cognition/API | V-VECTOR-01, V-RET-01..03, V-CONS-01, V-TOOL-01, V-XPORT-01, V-PERF-01 | Retrieval, Cognition, API, Security |
| F4 Human Digital Twin/list-first | V-UI-UNIT-01, V-DT-01, V-E2E-01, V-A11Y-01, V-VIS-01, initial V-RESOURCE-01 | Product/UX, Accessibility, Visual Truth, Frontend |
| F5 Production release | all F0–F5 suites, full 100k/regression/fault/rebuild/interchange/multi-window/resource/SBOM evidence; zero open Critical/High truth/privacy/security/lifecycle/accessibility/integrity risk | Release owner plus every prior mandatory role |
| F6 Optional 3D | V-3D-01 and predecessor F5 manifest hash | Product, Accessibility, Performance, Supply Chain |

Reviewer JSON contains reviewer identity/role, UTC timestamp, manifest hash, reviewed artifact hashes, verdict, findings, and signature method. The author of an implementation may not be the sole Security, Accessibility, Visual Truth, Retrieval-quality, crypto, or license approver. A waiver cannot override P0 acceptance criteria, unknown license, policy leak, false erasure, authority corruption, or an earlier gate.

## 7. Coverage and Orphan Enforcement

A planned manifest linter SHALL fail unless every MGR-001–MGR-048 and MGD-001–MGD-046 maps to at least one suite, risk, gate, and artifact class; every suite references valid IDs; all 65 finding IDs and 31 opportunity IDs occur exactly once in the audit ledger; and no Pass points only to a checklist. Coverage totals expected: requirements `48/48`, decisions `46/46`, findings `65/65`, opportunities `31/31`; current evidence status remains `0 verified` until actual linked run manifests exist.
