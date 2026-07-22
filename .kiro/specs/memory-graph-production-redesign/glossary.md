# Memory Graph Production Redesign — Canonical Glossary

**Status:** Planned design vocabulary. Definitions constrain code, DTOs, UI copy, tests, and evidence but do not assert implementation.

| Term | Normative meaning |
|---|---|
| Authority Store | The one SQLite database owning transactional cognitive truth. |
| Authority Transaction | One ACID unit committing semantic rows, immutable Event, Audit Record, outbox work, idempotency result, and one Graph Revision when graph-visible. |
| Cognitive Record | Versioned Event, Memory, Entity, Alias, Mention, Relationship, Evidence, Goal, Episode, Summary, Skill, Rule, Retrieval Trace, Feedback, or Audit Record. |
| Event Log | Immutable, ordered interaction/tool history with checksums and optional shred-key references. |
| Effective Policy | Most restrictive namespace/owner/scope/sensitivity/capability policy over all contributors. |
| Truth State | Current, Unverified, Stale, Contradicted, Superseded, Inferred, Confirmed, Forgotten, Deleted, or Unavailable. |
| Valid Time | Interval when a claim applies in the represented world. |
| Transaction Time | Authority revision when KRIA stored or changed a record. |
| Graph Revision | Monotonic transaction-time revision for graph-visible authority changes. |
| Memory Link | Registry-governed directed semantic link, including `derived_from`, `supports`, `contradicts`, `mentions_entity`, and `superseded_by`. |
| Evidence | Source-linked support or contradiction with actor, method, time, polarity, locator, and score semantics. |
| Provenance | Source, actor, locator, method/model version, time, and immediate parent links explaining a record. |
| Navigation Group | Generated browsing container; never an authority edge or analytics input. |
| Semantic Scene | Pure renderer-neutral policy-safe nodes, edges, groups, status, ordered collection, and authorized typed actions. |
| Authoritative 2D View | Complete Canvas2D map plus synchronized semantic list, inspector, and actions; launch does not require 3D. |
| Digital Twin | Human-readable, revision/policy-synchronized memory state; never a claim of brain, consciousness, emotion, or sentience. |
| Retrieved Candidate | Strategy result before gates and packing; not proof of use. |
| Retrieved Filtered Item | Candidate excluded before context injection with a policy-safe reason. |
| Used Item | Identifier present in a Retrieval Trace’s recorded context-injected set. |
| Why stored / Why recalled / How used | Respectively write-policy decision, retrieval/fusion rationale, and context-injection proof. |
| Relative score | Uncalibrated ranking value; never shown as probability or percentage. |
| Component | Connected component under a declared graph predicate and revision. |
| Community | Named clustering algorithm output with version, parameters, predicate, quality, and revision. |
| Recovery Mode | Fail-closed read-only state after authority integrity failure; no durable writes or invented authority. |
| Derived Index | Disposable FTS5/vector/analytics projection rebuilt from authority. |
| SQLiteVectorStore | Current-release planned exact policy-filtered brute-force cosine over compatible SQLite vector partitions. |
| VectorStore Port | Stable derived-index seam; no ANN/external backend in current release. |
| Adaptive RRF Profile | Versioned query-class profile with strategy weights, availability rules, `k`, evaluation corpus, and activation evidence. |
| Interchange Export | Self-describing checksummed open-format authorized package with schemas, versions, records, links, provenance, and lifecycle/truth state. |
| Evidence Artifact | Versioned test/report/trace/screenshot/SBOM/review tied to commit, environment, requirement, and gate. |

## Record Terms

| Term | Meaning |
|---|---|
| Memory | Mutable durable knowledge unit preserving event/source provenance. |
| Episode | Bounded sequence of related events/records for a session or task. |
| Summary | Deterministic compressed record derived from episodes/memories. |
| Skill | Reusable procedural record supported by repeated independent evidence. |
| Rule | Highest-compression governed record; never promoted from insufficient or self-reflective-only evidence. |
| Entity | Canonical typed subject; not a synonym for memory. |
| Alias | Versioned source-linked alternate identifier for an entity. |
| Mention | Provenance-bearing link from a source span/locator to an entity. |
| Relationship | Registry-governed semantic claim between canonical endpoints with evidence and validity. |
| Goal | Candidate/active/paused/completed/conflicted/stale/superseded/deleted task intention with provenance and resumption context. |
| Tool Observation | Start/completion-linked outcome record for native/MCP/OpenClaw/sidecar execution; never an authorization grant. |
| Memory Worth | Evidence-limited outcome attribution; below 20 observations it cannot alter ranking or archival. |
| Consolidation | Bounded deterministic Episode→Summary→Skill→Rule derivation preserving all immediate parents. |
| Source | Consent/lifecycle/policy identity for native, MCP, OpenClaw, sidecar, import, library, or conversation input. |

## UI and State Terms

| Term | Required copy behavior |
|---|---|
| Empty | Authorized authority query succeeded and contains no matching record; never used for failure. |
| Partial | Some required/optional strategy or section failed; preserved results remain visible with omissions named. |
| Stale | Valid prior snapshot whose revision cannot currently be confirmed. |
| Offline | Network/host unavailable while declared local capabilities remain available. |
| Unavailable | Capability or field lacks authority/evidence; may be omitted with reason. |
| Pending | Command accepted locally or by adapter but matching committed revision is not yet applied. |
| Crypto-Shredded | Allowed only after subject-bound encryption and destroyed-key plaintext-denial evidence. |
| Hard Delete Pending Cryptographic Erasure | Required label when plaintext/recoverable keys may remain despite a key status update. |
| Showing N of M / at least M / estimate M | Exact, lower-bound, or estimated total semantics respectively. |
| Filter this view | Local visible filtering; never called full-corpus or semantic search. |

## Forbidden/Misleading Language

Use `entity in this view`, not “active memory” for entity counts; `component`, not community for connectivity; `structural link suggestion`, not AI reasoning; `relative score`, not confidence/probability absent calibration; `available`, not Used absent trace proof; exact revision/time, not “synced moments ago”; and `Digital Twin`, not brain/consciousness/emotion/sentience. Dormant source is not a capability.

## Additional Binding Terms

| Term | Normative meaning |
|---|---|
| Semantic Revision | The Graph Revision shared by user-visible cognitive authority DTOs; operational telemetry does not invent one. |
| Status Version | Monotonic version plus observation time for Health/model/index/resource state; it is explicitly separate from Semantic Revision. |
| Policy Meet | Fail-closed Effective Policy operation: same owner or deny, intersect namespace/scope/capability sets, take maximum sensitivity, and retain contributor provenance. |
| Deny Derivation | Result of an empty policy intersection; no derived record, count, cache entry, or placeholder may be produced. |
| Write Decision | Policy-safe accepted/rejected/deferred admission record with policy version, actor, source event, subject revision, and non-secret reason codes; distinct from security audit. |
| Provisional Trace | Read-pipeline explanation before prompt construction has supplied the exact context-injected set; it cannot prove Used. |
| Finalized Retrieval Trace | Governed authority record containing the exact injected order and token allocation for a response/task, linked to its source and committed revisions. |
| Source Revision | Semantic revision from which a derived query, trace, preview, or analysis read its inputs. |
| Half-Open Valid Time | Claim interval `[valid_from, valid_until)`; the start is included and the end is excluded. |
| Derived Generation | Rebuildable FTS/vector/analytics dataset built separately and activated only after manifest verification. |
| Deleted Reconciled | Terminal lifecycle state in which deleted content is absent from every supported read, cache, scene, export, and active derived generation; it does not claim physical unreadability. |
| Physical Erasure | Removal of recoverable bytes from storage media/snapshots; not promised by Hard Delete. |
| Semantic Zoom | Deterministic change among aggregate, entity, memory-link, and evidence/source levels using authorized scene data while preserving stable selection. |
| Authority Plane | SQLite semantic records, events, provenance, audit, write decisions, revisions, and outbox governed by Authority Transactions. |
| Operational Plane | Timestamped model/index/resource/scheduler observations exposed by Health and versioned independently from semantic authority. |
| Idle | Client state before any successful query; unlike Empty, it makes no corpus claim. |
| Empty Result | Successful authorized query with exactly no matching visible records; never a synonym for uninitialized, unavailable, or failed. |
| Remote Security Profile v1 | Minimum non-loopback configuration: protected transport, restrictive origins, signed short-lived identity and grants, replay defense, limits, key references, and redacted audit. |
| List-First Fallback | Complete synchronized semantic table and inspector workflow used when pixels, workers, effects, or optional renderers are unavailable. |

## Architecture, Governance, and Release Terms

| Term | Normative meaning |
|---|---|
| Planned Target | Approved binding architecture or behavior that remains future work until its governing executable and manual Evidence Artifacts pass. It must never be presented as shipped capability. |
| Shipped Current State | Behavior demonstrated by current code plus executable evidence. For this spec's present repository observation: `MemoryUniverse` is the active 2D SVG path, `GraphCanvas3D` is dormant, and vector/FTS foundations exist; this observation is not readiness evidence for the Planned Target. |
| Single SQLite Authority | The invariant that one SQLite Authority Store owns every durable Cognitive Record and Event. FTS5, vectors, analytics, caches, and renderer scenes are disposable projections and cannot accept authoritative writes. |
| Pinned Embedding Model | Planned current-release `all-MiniLM-L6-v2` identity at exactly 384 dimensions, bound to reviewed artifact revision, artifact/tokenizer checksums, runtime, pooling, normalization, and license disposition in a model manifest. A model name alone is not a pin. |
| Five-Strategy Hybrid Retrieval | Planned bounded retrieval over FTS5, exact vector, graph, temporal, and active-goal strategies, followed by policy/truth gates, deterministic fusion, diversity, and token packing. |
| Adaptive Weighted RRF | Weighted Reciprocal Rank Fusion using a versioned Adaptive RRF Profile. Adaptation means offline-evaluated profile activation; unavailable strategies contribute nothing, weights are not silently redistributed, and runtime feedback never mutates them online. |
| Canonical Memory Links (`Memory_Links`) | The sole governed semantic-link model, projected to SQLite as `memory_links`, with required versioned types `derived_from`, `supports`, `contradicts`, `mentions_entity`, and `superseded_by`. Every link carries validated endpoints, provenance, Effective Policy, Truth State, optional Valid Time, event, and revision; no parallel untyped link table is allowed. |
| Backend-First Gates | Ordered release evidence gates F0 evidence reset, F1 authority/security, F2 semantic model, F3 retrieval/cognition, F4 Control Center, F5 production release, and F6 optional 3D. A later gate cannot pass while an earlier P0 criterion lacks linked evidence. |
| Governed Consolidation | Bounded, deterministic, idempotent, versioned Episode→Summary→Skill→Rule derivation that preserves immediate-parent lineage, applies the Policy Meet, and crosses Write Policy before becoming durable authority. |
| Governed Tool Learning | Start/completion-linked Tool Observation analysis that may influence only a named, versioned policy after bounded independent evidence. It cannot grant capability, broaden scope, weaken policy, promote a Rule from insufficient/self-reflective evidence, override deletion, or mutate retrieval weights online. |
| Forget | Governed reversible lifecycle transition to `Forgotten`; default reads and active projections exclude content during the declared restore window, while provenance and audit remain. It is not deletion or erasure. |
| Hard Delete | Governed lifecycle operation that, after reconciliation, guarantees zero content through supported reads, active projections, caches, scenes, traces, and exports. It does not claim physical or cryptographic erasure. |
| Crypto-Shred | Governed destruction of subject-bound payload encryption keys. The result may be labeled `Crypto-Shredded` only after tests prove plaintext denial across current, history, snapshot, cache, and index paths. |
| Open Interchange | Documented, self-describing, checksummed, policy-selected import/export formats that preserve records, Events, canonical links, provenance, lifecycle/truth state, and schema/ontology/model versions without exporting secrets outside policy. |
| Schema Evolution | Versioned deterministic migration and round-trip discipline: validate the whole import before one commit, reject unknown required semantics atomically, preserve unknown optional fields on re-export, and never create dual authority. |
| FOSS Release Gate | Release-blocking review of project, model, asset, Rust, npm, and Python license dispositions against exact locked artifacts; license comments or inferred metadata are not approval. |
| SBOM | Machine-readable Software Bill of Materials covering exact release artifacts and dependencies, linked to checksums, license dispositions, vulnerability report, commit, environment, and release gate. |
| Canvas2D Authoritative Target | Planned pixel implementation of the Authoritative 2D View. It becomes authoritative only after Reference Hardware evidence passes F4/F5 correctness, accessibility, bounded-memory, responsiveness, culling, idle-quiet, and quality-ladder criteria. It is not the Shipped Current State. |
| Semantic DOM Table Fallback | Synchronized, accessible, complete table/list plus inspector and authorized actions using the same Semantic Scene and revision as the pixel map; it remains operable when canvas, workers, effects, embeddings, models, or optional renderers are unavailable. |
| Optional 3D Ship-or-Delete | F6 rule requiring an optional 3D renderer to prove its preregistered semantic z-axis, task benefit, performance, idle, accessibility, action, policy, and revision parity. If it fails, its controls, renderer, graph-only dependencies, assets, tests, and capability claims are removed. |

`Confirmed` is a persisted Truth State backed by declared confirmation evidence; `Current` means applicable under Valid Time and lifecycle without that stronger claim. `Unavailable` remains a capability/presentation term and is never persisted as claim truth. `Used` is a trace relation, not a Truth State despite sharing visual-state grammar.

Hard Delete copy must state that content is excluded from supported reads and projections after reconciliation while immutable plaintext Events or recoverable snapshots may still retain bytes. Only verified subject-key destruction may use `Crypto-Shredded`.
