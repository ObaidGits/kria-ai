# Memory Graph Production Redesign — Decision Register

**Status:** Approved design direction, not implementation or release evidence. All target behavior remains Planned.

## Precedence and Change Rule

MGR-001–MGR-048 are normative. MGD-001–MGD-022 are preserved below. A replacement decision supersedes only the named clause and must update design, validation, roadmap, risk, glossary, and traceability. Current code or checked tasks do not ratify a target decision.

## Preserved MGD-001–MGD-022

| ID | Preserved decision |
|---|---|
| MGD-001 | Primary job is trust, correction, and explanation of actual knowledge and answer use. |
| MGD-002 | 2D is the authoritative production representation; optional 3D cannot block public readiness. |
| MGD-003 | Typed mixed projection: entity-primary; memory/evidence/source expanded on demand. |
| MGD-004 | Generated groups are navigation containers, never authority topology. |
| MGD-005 | Connected components are `component`; `community` requires named algorithm/version/parameters/quality/revision. |
| MGD-006 | Server is loopback by default; remote is explicit, authenticated, authorized, origin-restricted, protected, and deny-by-default. |
| MGD-007 | Scope/sensitivity propagate through every derivation; Effective Policy is the most restrictive contributor absent audited declassification. |
| MGD-008 | Relationship identity is registry-defined; symmetric endpoints canonicalize; directed endpoints retain order. |
| MGD-009 | Relationships have multiple supporting/contradicting evidence records; strength is named/versioned or omitted. |
| MGD-010 | All durable graph writes use Memory API + Write Policy in one authority transaction with audit/outbox/revision. |
| MGD-011 | Graph revision is monotonic; one graph-visible commit advances once; responses and patches are revisioned. |
| MGD-012 | Canonical contract is transport-neutral; host support is explicit and supported public operations have parity. |
| MGD-013 | Touch supports scoped 2D tasks with ≥44px targets, pinch-centroid zoom, and two-finger pan. |
| MGD-014 | Each lens/window owns intent state; immutable revision/policy/query cache may be shared. |
| MGD-015 | Release scale fixture is 100k authority records; visual work remains query-scoped. |
| MGD-016 | Optional 3D requires one semantic z-axis, ≥10% task benefit, ≥30 FPS, idle quiet, parity, or clean deletion. |
| MGD-017 | Prediction scores are relative unless calibrated on a versioned corpus. |
| MGD-018 | Current-state docs and executable contracts describe shipped truth; intent docs mark future behavior Planned. |
| MGD-019 | Pre-production DB may hard-migrate; no compatibility shim or dual authority. |
| MGD-020 | Security rollback disables capability or uses read-only safe behavior; it never restores unsafe paths. |
| MGD-021 | Current shipped graph path is `MemoryUniverse` 2D SVG; `GraphCanvas3D` is dormant and not capability evidence. |
| MGD-022 | Documentation is split by current state, contract/capability, future design, task, and evidence purpose. |

## New and Replacement Decisions

| ID | Decision | Replaces/clarifies | Evidence/rationale | Planned consequence |
|---|---|---|---|---|
| MGD-023 | SQLite v2 remains sole authority; schema hard-cutover is preferred to compatibility scaffolding. | Clarifies MGD-019 | Single-owner pre-production posture; existing event/SQLite foundation | Fresh-create and deterministic reconciliation/reset; delete legacy write paths after parity |
| MGD-024 | Current-release vector backend is exact `SQLiteVectorStore` behind `VectorStorePort`; LanceDB, Qdrant, HNSW, and all ANN are excluded. | Replaces future-backend wording in current comments/docs, not MGD-015 | 100k evidence must precede extra infrastructure; exact results simplify truth and deletion | 384d model-compatible brute-force cosine; external/ANN review only after F5 evidence |
| MGD-025 | Retrieval uses five bounded strategies fused by versioned weighted RRF; adaptation is offline-evaluated profile activation, never unconstrained online weight mutation. | Extends MGD-017 | Reproducibility and traceability | Query class/profile/weights/contributions appear in every trace |
| MGD-026 | Authoritative graph pixels use Canvas2D with a synchronized DOM semantic list; no global force layout. | Refines MGD-002 | Audit found current SVG element/filter pressure and WebKitGTK risk; static evidence favors bounded immediate-mode drawing, subject to F4 profiles | Deterministic query-specific layouts, spatial-index hit testing, measured culling, list parity |
| MGD-027 | Application-level crypto-shred is an unavailable capability until encrypted payload/key-destruction denial evidence exists. | Clarifies MGD-020 and lifecycle claims | Current status-row update does not prove unreadability | UI says hard delete/pending crypto; OS disk encryption is disclosed without overclaim |
| MGD-028 | Optional 3D is F6 after complete F5 release, and its semantic z-axis must be preregistered before implementation. | Updates old phase numbering in MGD-016/021 | Requirements make 3D independent of readiness | Failure deletes controls, renderer, graph-only dependencies/assets/tests/claims |
| MGD-029 | Model and dependency licenses are unknown until exact artifact/license/SBOM evidence is reviewed; comments/manifests are not legal disposition. | New | Root MIT declaration conflicts with Apache-2.0 `LICENSE`; model facts are incomplete | F5 blocks until project license, model, assets, crates/npm/Python dispositions are reconciled |
| MGD-030 | The Control Center is one revision/policy-synchronized Digital Twin with Overview, Recall, Knowledge, Timeline, Goals, Sources, Health. | Extends MGD-001/003 | Prevents destination drift and brain/sentience claims | Unsupported destinations/actions are unavailable, never simulated |
| MGD-031 | Tool observations cannot escalate capability, scope, policy, rule promotion, or deletion. | New | Learning must not become authorization | Outcome learning only influences named retrieval policies after evidence thresholds |
| MGD-032 | Schema/model/interchange evolution is versioned and round-trip tested; unknown required semantics fail atomically and optional fields survive re-export. | Extends MGD-019 | Long-lived data without current-stage distributed machinery | Migration fixture for every released schema; deterministic empty-store import |
| MGD-033 | Core dependencies are one-way and concrete storage is composed only in `kria-core`; Retrieval remains read-only and durable traces cross Write Policy as explicit commands. | Clarifies MGD-010/012 | Prevents hidden SQL/write paths and resolves authoritative Used claims | Enforced module DAG; no Python/adapter/renderer authority; trace finalization before model invocation |
| MGD-034 | Effective Policy is an authoritative fail-closed meet over owner, namespace, scope, sensitivity, and capabilities; empty intersections deny derivation. | Clarifies MGD-007 | Column-only sensitivity was insufficient for isolation | Canonical policy rows/hashes key queries, cursors, caches, DTOs, and derivations |
| MGD-035 | Graph Revision versions semantic authority; operational Health uses a separately labeled status version and observation time. | Clarifies MGD-011/030 | Runtime model/index/resource status can change without semantic mutation | No mixed semantic revisions and no fake revision churn for telemetry |
| MGD-036 | Hard Delete guarantees zero content through supported reads after reconciliation, but is not physical or cryptographic erasure while immutable plaintext events or recoverable snapshots may remain. | Clarifies MGD-027 | Event immutability and honest erasure must coexist | UI distinguishes Forgotten, Deleted/Reconciled, pending crypto erasure, and proven Crypto-Shredded |
| MGD-037 | Remote profile v1 fails startup unless protected transport, restrictive origins, short-lived signed identity/grants, replay defense, limits, and redacted audit are complete. | Refines MGD-006/020 | P0 threat boundary requires one implementable minimum | Loopback remains default; unsafe fallback is prohibited |
| MGD-038 | One SQLite Authority Store owns every durable Cognitive Record and Event, including memories, entities, links, goals, episodes, summaries, skills, rules, traces, feedback, audit, and the immutable Event Log; FTS5, vectors, analytics, caches, and scenes are rebuildable non-authoritative projections. | Clarifies MGD-023/033 | A single atomic truth boundary prevents split-brain lifecycle, policy, revision, and recovery behavior. | Every accepted durable mutation crosses Write Policy and one Authority Transaction; no adapter, renderer, sidecar, Python service, vector store, or graph store may become a second authority. |
| MGD-039 | The planned current-release embedding identity is pinned `all-MiniLM-L6-v2` at 384 dimensions; artifact revision, artifact/tokenizer checksums, runtime, pooling, normalization, and reviewed license disposition are manifest-bound. | Clarifies MGD-024/029 | A model label alone cannot prove compatibility, reproducibility, or licensing. | Wrong-model, wrong-dimension, malformed, non-finite, or stale vectors are rejected and rebuilt; exact brute-force cosine remains the planned `SQLiteVectorStore` behavior. |
| MGD-040 | `Memory_Links` is the canonical governed semantic-link model and `memory_links` is its SQLite projection; required versioned types are `derived_from`, `supports`, `contradicts`, `mentions_entity`, and `superseded_by`. No parallel untyped link table is permitted. | Extends MGD-008–010/023 | Consolidation, evidence, mentions, contradiction, and supersession require one policy-safe lineage vocabulary. | Link writes validate registry type, mixed-kind endpoints, provenance, Effective Policy, Truth State, Valid Time, identity, audit, and revision in one Authority Transaction. |
| MGD-041 | Authority integrity, schema, or immutable-event verification failure enters `Recovery_Mode`: a fail-closed read-only state that exposes policy-safe diagnostics and only verified restore/import actions. Derived-index failure alone is Partial capability and triggers isolated rebuild, not Recovery Mode. | Clarifies MGD-020/023 | Authority corruption must never be hidden by fabricated state or unsafe writes, while disposable-index faults should not disable sound authority reads. | Recovery Mode permits no durable cognitive writes and exits only after complete verification succeeds. |
| MGD-042 | Release sequencing is backend-first and gate-ordered: F0 evidence reset, F1 authority/security, F2 semantic model, F3 retrieval/cognition, F4 Control Center, F5 production release, then optional F6 3D. A later gate cannot pass while an earlier P0 criterion lacks linked evidence. | Extends MGD-018/022 | UI polish cannot establish storage, policy, lifecycle, retrieval, or failure correctness. | Capability copy and controls remain unavailable until backend contracts, negative paths, performance bounds, and Evidence Artifacts pass their governing gate. |
| MGD-043 | Consolidation and tool learning are governed cognition, not autonomous authority: Episode→Summary→Skill→Rule outputs are deterministic, idempotent, versioned, source-linked, policy-propagating, and admitted through Write Policy; tool outcomes require start/completion linkage and bounded evidence before influencing a named policy. | Extends MGD-031/033 | Compression and outcome learning can otherwise launder provenance, permissions, or weak correlations into truth. | Immediate parents remain inspectable; self-reflection-only or insufficient evidence cannot promote Rules; learning cannot grant capability, broaden scope, weaken policy, override deletion, or mutate retrieval weights online. |
| MGD-044 | Interchange and evolution use self-describing, checksummed open formats with explicit schema, ontology, relation, policy, lifecycle, truth, provenance, event, link, and model versions. Imports validate the whole manifest before one atomic commit; unknown required semantics fail, while unknown optional fields survive round-trip re-export. | Extends MGD-032 | Long-lived local memory requires portable data and deterministic evolution without dual authority or compatibility scaffolding. | Every released schema has migration/round-trip fixtures; exports are policy-selected, secret-free, and independently parseable from documented schemas. |
| MGD-045 | FOSS and supply-chain evidence are release gates: project, model, asset, Rust, npm, and Python license dispositions must be reviewed against exact locked artifacts, and the release emits a machine-readable SBOM plus vulnerability report. | Extends MGD-029 | License comments and package metadata are not sufficient evidence, and the repository currently contains unresolved license signals. | F5/F6 block on unresolved or incompatible dispositions, missing checksums/locks, missing SBOM entries, or unreviewed release dependencies. |
| MGD-046 | Canvas2D is the planned authoritative pixel implementation only if Reference Hardware evidence proves F4/F5 correctness, accessibility, bounded memory, responsiveness, culling, idle quiet, and quality-ladder targets. The synchronized semantic DOM table/list and inspector are the complete fallback. Optional 3D must pass F6 and ship with parity or be deleted with its controls, dependencies, assets, tests, and claims. | Clarifies MGD-002/016/026/028 | A preferred renderer is a target, not shipped truth; accessible tasks cannot depend on GPU/canvas success or speculative 3D. | Failed Canvas2D evidence preserves the list-first product and requires a revised evidenced 2D implementation; failed 3D evidence triggers ship-or-delete, never dormant product code. |

### State Interpretation for MGD-023+

All MGD-023+ rows describe the **Planned Target/current-release design** unless a row explicitly says otherwise; they are binding architecture choices, not claims that code has shipped. The **Shipped Current State** remains the evidence-backed repository behavior identified by MGD-021 and current-state documentation: active `MemoryUniverse` 2D SVG, dormant `GraphCanvas3D`, and existing vector/FTS foundations. In particular, `SQLiteVectorStore` exact 384-dimensional brute-force cosine, five-strategy adaptive weighted RRF, canonical `Memory_Links`, Recovery Mode, Canvas2D, and the complete Control Center remain Planned until executable gate evidence proves them.

## Explicitly Rejected for Current Release

- A second authoritative graph/vector database, distributed synchronization, consensus, or enterprise tenancy.
- LanceDB, Qdrant, HNSW/ANN, or Python-required retrieval authority.
- Automatic person merge from name or embedding similarity; self-reflection-only Rule promotion.
- Raw ranking values displayed as probabilities; graph proximity used as answer-use proof.
- Renderer-owned semantics, global full-adjacency loading, perpetual animation, hidden client-side authorization.
- Compatibility shims, dual writes, and dormant optional-renderer code retained after a failed gate.
