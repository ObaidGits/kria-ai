# Requirements Document

## Memory Graph Production Redesign — Cognitive Memory System and Memory Control Center

**Feature:** `memory-graph-production-redesign`  
**Spec type:** Feature  
**Workflow:** Design-first  
**Phase:** Specification complete; implementation not begun  
**Status:** Planned target; no criterion in this document is implementation evidence

## Introduction

This specification upgrades KRIA’s fragmented memory and graph surfaces into one production-grade **Cognitive Memory System** with a human-authoritative **Memory Control Center**. The backend foundation is the release-critical product; the interface exposes that foundation without inventing knowledge. SQLite remains the sole transactional authority, immutable events preserve audit history, derived indexes remain rebuildable, and all durable changes pass through one governed write boundary.

The document reconciles rather than discards the existing graph requirements, the 65 findings and 31 opportunities in `docs/design/memory-graph-comprehensive-audit.md`, the memory laws and verified hardening record in `.kiro/specs/memory-upgrade/`, and binding decisions MGD-001–MGD-046; MGD-001–MGD-022 are the preserved original decisions. Existing design and task files remain historical planning inputs. Checked task boxes, dormant code, comments, screenshots, or old readiness claims do not satisfy these requirements.

## Authority and Precedence

1. Shipped code plus executable evidence defines current behavior.
2. This document defines required future behavior for this feature.
3. Memory laws L1–L12 and current single-user, single-process, single-laptop constraints remain binding.
4. Decisions MGD-001–MGD-046 remain binding; MGD-001–MGD-022 are the preserved original decisions, and later decisions extend or explicitly clarify them.
5. Graph-specific claims in `kria-ui-redesign` remain superseded where the decision register says so.
6. Public readiness ends with a complete authoritative 2D product; optional 3D is a separate evidence-gated outcome.

## Product Outcomes

1. A user can find, understand, verify, correct, forget, and explain what KRIA knows.
2. A user can distinguish current truth, historical truth, contradiction, inference, retrieval use, and unavailable data.
3. KRIA can recall through vector, FTS5, graph, time, and active-goal evidence without bypassing policy.
4. KRIA can consolidate episodes into summaries, skills, and rules without losing source lineage.
5. Native tools, MCP servers, OpenClaw skills, sidecars, workspaces, sessions, and sensitivity classes remain isolated.
6. Core storage, retrieval, lifecycle, security, and fault behavior are proven before visual-polish readiness.
7. The Control Center remains useful, accessible, responsive, and truthful when graph effects, embeddings, models, network, or optional renderers are unavailable.

## Product Principles

1. **One authority:** one SQLite database owns transactional truth; vectors, FTS, analytics, caches, and render scenes are disposable projections.
2. **Provenance before presentation:** no UI, model, algorithm, or renderer may invent facts, topology, confidence, causality, recency, or answer use.
3. **Human authority:** users can inspect, correct, merge, split, supersede, forget, restore, and permanently erase authorized memory through governed operations.
4. **Local and offline core:** storage, policy, FTS5 recall, graph traversal, lifecycle, and correction remain useful without network, LLM, or embedder availability.
5. **Deterministic derivation:** every derived record is versioned, idempotent, source-linked, policy-propagating, and rebuildable where it is not authority.
6. **Bounded laptop operation:** foreground interaction wins over cognition; all queues, traversals, payloads, caches, scenes, workers, and animations have explicit limits.
7. **Semantic parity:** 2D map, semantic list/table, inspector, and any optional 3D renderer share one scene, action, policy, and revision contract.
8. **Backend evidence before UI claims:** no visual control or status may imply a capability before its backend contract, failure behavior, and executable evidence exist.
9. **Complete 2D before conditional 3D:** public readiness requires a production-grade accessible 2D Digital Twin; true 3D is optional and must prove task value.
10. **Long-lived seams, current-stage simplicity:** schemas and ports are versioned for evolution, while the current release remains one user, one process, one laptop, and one SQLite authority.

## Non-Goals

- Multi-device synchronization, distributed consensus, or enterprise tenancy in the current release.
- A second authoritative graph database or a renderer-owned knowledge model.
- Global visualization of every entity or memory at once.
- Automatic person merges based only on names or embedding proximity.
- Treating raw retrieval scores, centrality, generated layout, or model prose as verified truth.
- Local model training or autonomous rule promotion without governed evidence.
- Backup ceremony, compatibility shims, or dual-write migration paths solely to protect pre-production data.
- 3D as a launch dependency, branding promise, or substitute for measurable task value.

## Glossary

- **Cognitive_Memory_System:** The complete KRIA memory domain, including authority, policy, retrieval, truth maintenance, lifecycle, cognition, and user controls.
- **Authority_Store:** The one SQLite database that is the sole transactional source of truth.
- **Authority_Transaction:** One ACID transaction that atomically records authority changes, audit data, graph-visible revision, and outbox work.
- **Event_Log:** The immutable append-only event history used for provenance, audit, recovery cursors, and erasure references.
- **Write_Policy_Engine:** The mandatory admission and governance boundary for every durable memory, entity, relation, goal, correction, feedback, and cognitive write.
- **Derived_Index:** A disposable, rebuildable search or analytics structure, including the vector index, FTS5 projection, and optional analytical cache.
- **Cognitive_Record:** A typed durable record: Event, Memory, Entity, Alias, Mention, Relationship, Evidence, Goal, Episode, Summary, Skill, Rule, Retrieval_Trace, Feedback, or Audit_Record.
- **Memory:** A mutable durable knowledge unit derived from one or more Events while preserving provenance.
- **Episode:** A bounded sequence of related Events or Memories associated with a session or task.
- **Summary:** A compressed representation derived from Episodes or Memories.
- **Skill:** A reusable procedural representation derived from repeated evidence.
- **Rule:** A governed high-compression representation supported by independent evidence and false-promotion checks.
- **Provenance:** Source, actor, method, time, locator, model or algorithm version, and derivation links that explain a record.
- **Truth_State:** Current, Unverified, Stale, Contradicted, Superseded, Inferred, Confirmed, Forgotten, Deleted, or Unavailable.
- **Valid_Time:** The time interval during which a claim applies in the represented world.
- **Transaction_Time:** The authority revision at which KRIA stored or changed a record.
- **Graph_Revision:** A monotonic transaction-time revision covering graph-visible authority changes.
- **Retrieval_Engine:** The read-only orchestrator that performs policy-filtered vector, FTS5, graph, temporal, and active-goal retrieval and fuses results.
- **Retrieval_Trace:** A policy-safe record of candidates, scores, filters, exclusions, token allocation, and items actually injected into model context.
- **Used_Item:** A record proven by a Retrieval_Trace to have entered context for an identified response or task.
- **Retrieved_Filtered_Item:** A candidate excluded before context injection with a policy-safe reason.
- **Entity_Resolution_Engine:** The conservative service that proposes, accepts, rejects, merges, splits, and reverses canonical identity decisions.
- **Relation_Registry:** The versioned ontology defining relation identity, direction, inverse, reflexivity, endpoint kinds, evidence, and validity rules.
- **Effective_Policy:** Namespace, owner, scope, sensitivity, and capability restrictions after applying the most restrictive contributing evidence.
- **Memory_Mode:** Permanent, Temporary, Session_Only, Read_Only, or Disabled behavior selected by policy.
- **Cognitive_Scheduler:** The priority-aware coordinator for enrichment, consolidation, verification, extraction, analytics, and maintenance.
- **Memory_Control_Center:** The user-facing Memory space containing Overview, Recall, Knowledge, Timeline, Goals, Sources, and Health workflows.
- **Semantic_Scene:** Renderer-neutral graph/list content and typed actions derived from policy-safe DTOs.
- **Authoritative_2D_View:** The complete graph and table experience that carries every required task without optional 3D.
- **Navigation_Group:** A generated container used for browsing; Navigation_Groups are not authority edges.
- **Reference_Hardware:** The owner’s documented laptop configuration used for repeatable release measurements.
- **Evidence_Artifact:** A versioned test result, report, fixture, screenshot, trace, SBOM, or review record linked to commit, environment, and requirement IDs.
- **SQLiteVectorStore:** The current exact vector implementation: a versioned SQLite projection that performs policy-filtered brute-force cosine search over model-compatible vectors; it is rebuildable and is not authority.
- **VectorStore_Port:** The stable derived-index interface behind SQLiteVectorStore; an ANN or external implementation may replace it only after measured evidence and must never become transactional authority.
- **Adaptive_RRF_Profile:** A named, versioned weighted Reciprocal Rank Fusion configuration containing query class, strategy availability, per-strategy weights, RRF constant `k`, calibration/evaluation corpus, and activation rules.
- **Memory_Link:** A versioned directed semantic link with canonical type, endpoint kinds, provenance, Effective_Policy, Truth_State, and optional Valid_Time; required types include `derived_from`, `supports`, `contradicts`, `mentions_entity`, and `superseded_by`.
- **Digital_Twin:** The revision-synchronized, policy-safe, human-understandable model of KRIA’s knowledge state across map, list, inspector, goals, recall, health, and timeline; it is not a claim of personhood, sentience, consciousness, emotion, or a literal brain.
- **Quality_Ladder:** The ordered resource-degradation policy that disables decoration and optional analysis before reducing bounded scene size, while preserving policy, truth, retrieval, correction, lifecycle, and accessible list workflows.
- **Interchange_Export:** A self-describing, checksummed, open-format package containing selected authorized records, events, links, provenance, lifecycle/truth state, schema/ontology/model versions, and no secrets outside the selected export policy.
- **Recovery_Mode:** A fail-closed read-only operating state entered after authority-integrity failure; it exposes diagnostics and verified recovery actions but permits no invented or unsafe authority writes.

## Requirement Metadata

Priorities: **P0** release blocker, **P1** required production capability, **P2** scale/evolution capability, **P3** conditional enhancement. Gates: **F0** evidence reset, **F1** authority/security, **F2** semantic model, **F3** retrieval/cognition, **F4** Control Center, **F5** production release, **F6** optional 3D. A later gate cannot pass while an earlier P0 criterion lacks evidence.

## Requirements Index

| No. | ID | Title | Priority | Gate |
|---:|---|---|---|---|
| 1 | MGR-001 | Epistemic truth contract | P0 | F0–F5 |
| 2 | MGR-002 | Canonical mixed graph projection | P0 | F2 |
| 3 | MGR-003 | Server threat boundary | P0 | F1 |
| 4 | MGR-004 | Scope and sensitivity isolation | P0 | F1–F5 |
| 5 | MGR-005 | Governed relationship writes | P0 | F1–F2 |
| 6 | MGR-006 | Full-corpus ranked search | P0 | F3–F4 |
| 7 | MGR-007 | Versioned bounded graph API | P1 | F2–F3 |
| 8 | MGR-008 | Revision and patch consistency | P1 | F3–F5 |
| 9 | MGR-009 | Bounded backend execution | P0 | F1–F5 |
| 10 | MGR-010 | Temporal graph correctness | P0 | F2–F4 |
| 11 | MGR-011 | Honest analytics vocabulary | P0 | F0–F5 |
| 12 | MGR-012 | Renderer-neutral scene and actions | P0 | F2–F4 |
| 13 | MGR-013 | Focus concurrency correctness | P0 | F4 |
| 14 | MGR-014 | Accessible graph composite | P0 | F4–F5 |
| 15 | MGR-015 | Authoritative adaptive 2D view | P0 | F4–F5 |
| 16 | MGR-016 | Responsive input model | P1 | F4–F5 |
| 17 | MGR-017 | Fault containment and recovery | P0 | F1–F5 |
| 18 | MGR-018 | Relation ontology and evidence | P1 | F2–F4 |
| 19 | MGR-019 | Entity provenance and resolution | P1 | F2–F4 |
| 20 | MGR-020 | Transport capability parity | P1 | F3–F5 |
| 21 | MGR-021 | Multi-window ownership | P2 | F5 |
| 22 | MGR-022 | Idle and interaction budgets | P0 | F4–F5 |
| 23 | MGR-023 | Scale-aware subgraph navigation | P1 | F4–F5 |
| 24 | MGR-024 | Explain and correct workflows | P1 | F4–F5 |
| 25 | MGR-025 | Retrieval-use trace integration | P0 | F3–F4 |
| 26 | MGR-026 | Visual authority encoding | P0 | F4–F5 |
| 27 | MGR-027 | Testing and evidence gates | P0 | F0–F6 |
| 28 | MGR-028 | Privacy-safe observability | P1 | F1–F5 |
| 29 | MGR-029 | Documentation authority and audit continuity | P0 | F0–F5 |
| 30 | MGR-030 | Optional true-3D decision | P3 | F6 |
| 31 | MGR-031 | Control Center information and interaction integrity | P0 | F4–F5 |
| 32 | MGR-032 | Decades-long analytical evolution seam | P2 | F3–F5 |
| 33 | MGR-033 | Single SQLite authority and append-only events | P0 | F1 |
| 34 | MGR-034 | Typed cognitive records and provenance | P0 | F1–F2 |
| 35 | MGR-035 | Mandatory write policy and memory modes | P0 | F1 |
| 36 | MGR-036 | Five-strategy hybrid retrieval | P0 | F3 |
| 37 | MGR-037 | Truth maintenance and supersession | P0 | F2–F3 |
| 38 | MGR-038 | Active goals and goal-aware recall | P1 | F3–F4 |
| 39 | MGR-039 | Cognitive consolidation with source preservation | P1 | F3–F5 |
| 40 | MGR-040 | Memory lifecycle and user-controlled erasure | P0 | F1–F4 |
| 41 | MGR-041 | Cryptographic shredding truth | P0 | F1–F5 |
| 42 | MGR-042 | Derived-index convergence and model migration | P0 | F1–F3 |
| 43 | MGR-043 | Native, MCP, OpenClaw, sidecar, and tool isolation | P0 | F1–F3 |
| 44 | MGR-044 | Tool success and failure learning | P1 | F3–F5 |
| 45 | MGR-045 | Offline degradation and resource-aware cognition | P0 | F1–F5 |
| 46 | MGR-046 | Consent-gated ingestion and source lifecycle | P1 | F2–F4 |
| 47 | MGR-047 | Open-source licensing and SBOM | P0 | F1–F6 |
| 48 | MGR-048 | Backend-first release order and evolution discipline | P0 | F0–F6 |

## Requirements

### Requirement 1: MGR-001 — Epistemic Truth Contract

**User Story:** As a user, I want every memory claim to disclose authority and state, so that presentation never becomes fabricated knowledge.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL assign every visible semantic claim one or more defined Provenance and Truth_State values.
2. WHEN a required authority field is absent, THE Memory_Control_Center SHALL display `Unavailable` with the missing field category or omit the claim.
3. IF a score lacks calibration evidence, THEN THE Memory_Control_Center SHALL label it `Relative score`, show algorithm/profile version, and SHALL NOT render it as a probability or percentage; a value MAY be called confidence only when bounded to `[0.0, 1.0]` and accompanied by calibration method, versioned evaluation corpus, calibration date, and measured error.
4. WHEN a Navigation_Group is displayed, THE Memory_Control_Center SHALL label the group as generated navigation and exclude the group from authority topology.
5. WHEN the label `Used` is displayed, THE Memory_Control_Center SHALL provide a link to the Retrieval_Trace that proves context injection.
6. IF current behavior differs from planned behavior, THEN THE Cognitive_Memory_System SHALL describe the current behavior as current and the planned behavior as planned in product and developer surfaces.

**Traceability:** MGD-001, MGD-004, MGD-005, MGD-017, MGD-018; MG-C01–C04, MG-H05, MG-M07, MG-M28, MG-L01, MG-L07.

### Requirement 2: MGR-002 — Canonical Mixed Graph Projection

**User Story:** As an engineer, I want one typed policy-safe graph projection, so that storage, APIs, lists, inspectors, and renderers agree on meaning.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL project explicit node kinds `entity`, `memory`, `evidence`, `source`, and `aggregate` from Cognitive_Records.
2. THE Cognitive_Memory_System SHALL project explicit edge authority classes `stored`, `derived`, `inferred`, and `navigation`.
3. THE Cognitive_Memory_System SHALL include stable identifier, Graph_Revision, Effective_Policy, Truth_State, Valid_Time, Provenance summary, and typed metadata on every projected item.
4. WHEN a relationship endpoint is projected, THE Cognitive_Memory_System SHALL include a policy-safe canonical entity summary.
5. WHEN a graph projection is requested without explicit expansion, THE Cognitive_Memory_System SHALL return entity-primary bounded content and defer memory, evidence, and source expansion.
6. IF a projected edge references an unavailable endpoint, THEN THE Cognitive_Memory_System SHALL omit the edge or represent a typed aggregate frontier without exposing a hidden identifier.

**Traceability:** MGD-003, MGD-004, MGD-007; MG-C03, MG-H13, MG-M04, MG-M05, MG-M11, MG-M14.

### Requirement 3: MGR-003 — Server Threat Boundary

**User Story:** As a user, I want local memory inaccessible to unauthorized network clients, so that optional server capability does not weaken local privacy.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL bind server mode to loopback by default.
2. WHEN a non-loopback bind is configured, THE Cognitive_Memory_System SHALL require explicit remote enablement, authenticated identity, operation authorization, restrictive origins, transport protection, request limits, and audit logging.
3. IF credentials are missing, malformed, expired, replayed, or unauthorized, THEN THE Cognitive_Memory_System SHALL return a deny response that reveals no protected label, identifier, count, topology, or reason detail.
4. WHEN remote mode starts, THE Cognitive_Memory_System SHALL verify required security configuration before accepting requests.
5. IF remote security configuration is incomplete, THEN THE Cognitive_Memory_System SHALL refuse remote startup while preserving local Tauri operation.
6. WHEN graph or memory endpoints are tested, THE Cognitive_Memory_System SHALL produce negative evidence for anonymous, wrong-origin, wrong-scope, oversized, replayed, and cross-namespace requests.

**Traceability:** MGD-006, MGD-020; MG-C05.

### Requirement 4: MGR-004 — Scope and Sensitivity Isolation

**User Story:** As a privacy-conscious user, I want source boundaries preserved through every derivation, so that memory cannot leak through entities, retrieval, analytics, or presentation.

#### Acceptance Criteria

1. WHEN a Cognitive_Record is created or derived, THE Cognitive_Memory_System SHALL assign namespace, owner, scope, sensitivity, and source identity before commit.
2. WHEN multiple records contribute to a derived record, THE Cognitive_Memory_System SHALL calculate Effective_Policy as the most restrictive contributing policy.
3. WHEN an authorized declassification occurs, THE Write_Policy_Engine SHALL create a new audited provenance record rather than mutate source policy.
4. WHEN any read executes, THE Cognitive_Memory_System SHALL enforce Effective_Policy before query planning, result counts, ranking, serialization, caching, and rendering.
5. IF a hidden record contributes to an aggregate, THEN THE Cognitive_Memory_System SHALL expose only caller-authorized counts and labels.
6. WHEN identity or scope changes during an in-flight request, THE Cognitive_Memory_System SHALL discard the response and invalidate incompatible cache entries.
7. WHEN cross-scope tests run across Tauri, server, search, graph, path, prediction, export, trace, inspector, and patch flows, THE Cognitive_Memory_System SHALL report zero protected-data leaks.

**Traceability:** MGD-007; MG-C06, MG-O27, MG-O28; memory law L7 and D-20.

### Requirement 5: MGR-005 — Governed Relationship Writes

**User Story:** As a user, I want relationship changes validated, atomic, auditable, and reversible, so that graph correction cannot silently corrupt memory.

#### Acceptance Criteria

1. WHEN a durable relationship is created, edited, confirmed, expired, merged, split, restored, or deleted, THE Write_Policy_Engine SHALL process the command in one Authority_Transaction.
2. WHEN a relationship command is evaluated, THE Write_Policy_Engine SHALL validate caller capability, canonical endpoints, relation registry version, direction, reflexivity, Valid_Time, Effective_Policy, and Evidence.
3. WHEN repeated commands use the same idempotency key, THE Write_Policy_Engine SHALL return the original committed result without creating a second semantic relationship.
4. WHEN additional observations support an existing relationship identity, THE Cognitive_Memory_System SHALL append Evidence without duplicating the active semantic edge.
5. IF any relationship write step fails, THEN THE Authority_Store SHALL preserve the pre-command relationship, audit, outbox, and Graph_Revision state.
6. WHEN undo is available, THE Write_Policy_Engine SHALL create a compensating governed record that preserves the original audit history.

**Traceability:** MGD-008–MGD-010; MG-C07, MG-H08, MG-M12, MG-M13.

### Requirement 6: MGR-006 — Full-Corpus Ranked Search

**User Story:** As a user, I want one honest full-corpus search, so that absence from the visual window never implies absence from memory.

#### Acceptance Criteria

1. WHEN a search query is submitted, THE Retrieval_Engine SHALL search authorized memory content, entities, aliases, source metadata, goals, and relation labels through backend ranking.
2. WHEN a search result is returned, THE Retrieval_Engine SHALL include result kind, matched field, rank rationale, policy summary, Truth_State, Graph_Revision, and navigation target.
3. WHEN results are truncated or estimated, THE Memory_Control_Center SHALL display `showing N of M`, `at least M`, or `estimate M` according to the response semantics.
4. IF no authorized result matches, THEN THE Memory_Control_Center SHALL show a no-result state that preserves active filters and offers filter revision without claiming an empty Authority_Store.
5. IF a search strategy is unavailable, THEN THE Memory_Control_Center SHALL label the result set partial and identify the unavailable strategy.
6. WHEN the 100,000-record release fixture is queried on Reference_Hardware, THE Retrieval_Engine SHALL complete Control Center search within 250 ms p95 after declared warm-up.
7. WHERE local visible filtering is offered, THE Memory_Control_Center SHALL label the operation `Filter this view` and keep the operation separate from full-corpus search.

**Traceability:** MG-H01, MG-H04, MG-L05; MG-O05, MG-O06.

### Requirement 7: MGR-007 — Versioned Bounded Graph API

**User Story:** As a client developer, I want bounded revisioned graph queries, so that common interactions scale with visible work rather than corpus size.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL expose versioned contracts for search, neighborhood, path, trace, aggregate, prediction, temporal diff, and revision patch operations.
2. WHEN a graph response is returned, THE Cognitive_Memory_System SHALL include schema version, Graph_Revision, query hash, policy-safe scope, filters, window metadata, total semantics, truncation, and recovery cursor.
3. WHEN cursor pages are requested, THE Cognitive_Memory_System SHALL preserve one snapshot revision and return each authorized item at most once.
4. IF a cursor is expired or revision-incompatible, THEN THE Cognitive_Memory_System SHALL return a typed bounded-refetch instruction.
5. WHEN a request is validated, THE Cognitive_Memory_System SHALL enforce configured maximums for depth, item count, payload bytes, label length, filter complexity, and deadline; relationship traversal SHALL be cycle-safe, policy-filtered before every expansion, and limited to three hops.
6. WHEN a traversal visits a stable identifier, THE Cognitive_Memory_System SHALL prevent revisiting it within the same path and SHALL bound revisits across the query; every truncated result SHALL return frontier and truncation-reason metadata without hidden identifiers.
7. WHEN deterministic cyclic and cross-scope fixtures run, THE Cognitive_Memory_System SHALL terminate every traversal, return no repeated path node, and reveal no unauthorized intermediary, endpoint, count, or topology.
8. WHEN the 100,000-record release fixture is queried on Reference_Hardware, THE Cognitive_Memory_System SHALL complete one-hop neighborhood within 500 ms p95 and prediction within 750 ms p95.

**Traceability:** MG-H09, MG-H16, MG-H17; MG-O06–MG-O08, MG-O21, MG-O22.

### Requirement 8: MGR-008 — Revision and Patch Consistency

**User Story:** As a user, I want every visible state to represent one authority revision, so that I never inspect mixed old and new knowledge.

#### Acceptance Criteria

1. WHEN graph-visible authority state changes, THE Authority_Store SHALL advance exactly one monotonic Graph_Revision for the Authority_Transaction.
2. WHEN a patch is emitted, THE Cognitive_Memory_System SHALL include base revision, target revision, bounded changes, and invalidation metadata.
3. WHEN a client revision equals the patch base revision, THE Memory_Control_Center SHALL apply the patch atomically.
4. IF a client revision does not equal the patch base revision, THEN THE Memory_Control_Center SHALL preserve the prior snapshot as stale and perform a bounded active-query refetch.
5. WHEN duplicate, reordered, missing, delayed, or replayed patches occur, THE Memory_Control_Center SHALL converge to Authority_Store state without a full-corpus reload.
6. WHILE a write awaits matching revision confirmation, THE Memory_Control_Center SHALL display a pending state and withhold confirmed styling.
7. IF a write receives a typed failure, THEN THE Memory_Control_Center SHALL roll back optimistic presentation and preserve the failure beside the initiating action.

**Traceability:** MGD-011; MG-H17, MG-L07; MG-O09, MG-O22.

### Requirement 9: MGR-009 — Bounded Backend Execution

**User Story:** As a laptop user, I want memory work to remain bounded and cancellable, so that foreground interaction and other KRIA workloads stay responsive.

#### Acceptance Criteria

1. WHEN synchronous SQLite, parsing, embedding, graph, or CPU work can exceed 50 ms, THE Cognitive_Memory_System SHALL execute the work outside asynchronous executor threads.
2. WHEN foreground work arrives, THE Cognitive_Scheduler SHALL preempt or defer lower-priority cognition within 100 ms.
3. WHEN a query traverses relationships, THE Cognitive_Memory_System SHALL batch endpoint and Evidence reads and enforce traversal limits.
4. WHEN analytics exceed an interactive budget, THE Cognitive_Memory_System SHALL use a revision-keyed cached result or a cancellable background job.
5. IF cancellation, deadline, memory pressure, power policy, or worker failure occurs, THEN THE Cognitive_Memory_System SHALL stop or degrade the affected work without corrupting authority or caches.
6. WHEN production performance tests run, THE Cognitive_Memory_System SHALL report zero graph-originated asynchronous executor blocking spans longer than 50 ms.

**Traceability:** MG-H03, MG-M15–MG-M17; memory hardening H4, R1, R2.

### Requirement 10: MGR-010 — Temporal Graph Correctness

**User Story:** As a user, I want KRIA to distinguish current, historical, and superseded truth, so that time controls do not misrepresent present knowledge.

#### Acceptance Criteria

1. WHEN a current relationship query executes, THE Cognitive_Memory_System SHALL apply one centralized active-validity predicate.
2. WHEN a historical query executes, THE Cognitive_Memory_System SHALL evaluate the requested Valid_Time independently from Transaction_Time.
3. WHEN temporal results are returned, THE Cognitive_Memory_System SHALL include requested instant or range, Graph_Revision, validity intervals, source times, and timezone metadata.
4. IF temporal snapshot or diff capability is unavailable, THEN THE Memory_Control_Center SHALL omit the timeline control and disclose the capability state in Health.
5. WHEN a temporal diff is displayed, THE Memory_Control_Center SHALL distinguish additions, expirations, contradictions, supersessions, and corrections by state and text.
6. WHEN temporal tests run, THE Cognitive_Memory_System SHALL cover open-ended intervals, exact boundaries, timezone conversion, expiry, supersession, and revision interaction.

**Traceability:** MG-H07, MG-M08, MG-O09, MG-O30.

### Requirement 11: MGR-011 — Honest Analytics Vocabulary

**User Story:** As a user, I want metrics named by what they calculate, so that analytical output is not presented as cognition or certainty.

#### Acceptance Criteria

1. WHEN connected components are produced, THE Cognitive_Memory_System SHALL name the output `component` in code, contracts, tests, documentation, and UI.
2. WHEN a community is produced, THE Cognitive_Memory_System SHALL include the named algorithm, version, parameters, graph predicate, Graph_Revision, and quality metadata.
3. WHEN centrality is produced, THE Cognitive_Memory_System SHALL include the named algorithm and evaluated scope.
4. IF an analytical value lacks a grounded interpretation, THEN THE Memory_Control_Center SHALL display the technical metric name or omit the value.
5. WHEN an algorithm or parameter version changes, THE Cognitive_Memory_System SHALL invalidate affected caches, result comparisons, and visual baselines.
6. WHEN generated facets organize content, THE Memory_Control_Center SHALL present the facets as navigation rather than ontology.

**Traceability:** MGD-005, MGD-017; MG-H05, MG-M11, MG-O14–MG-O16.

### Requirement 12: MGR-012 — Renderer-Neutral Scene and Actions

**User Story:** As a maintainer, I want one semantic scene and action model, so that visual representations cannot diverge from memory semantics.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL transform versioned policy-safe DTOs into one deterministic Semantic_Scene.
2. THE Memory_Control_Center SHALL make Authoritative_2D_View, synchronized list or table, and optional 3D consume the same Semantic_Scene.
3. WHEN a user action is available, THE Memory_Control_Center SHALL dispatch one typed action independent of renderer choice.
4. WHEN representation capabilities differ, THE Memory_Control_Center SHALL declare the difference in a versioned capability object.
5. IF a representation lacks a required action, THEN THE Memory_Control_Center SHALL keep the action available through the Authoritative_2D_View or synchronized semantic representation.
6. WHEN canonical renderer migration passes parity tests, THE codebase SHALL remove disconnected renderer business logic and duplicate controls.

**Traceability:** MGD-002, MGD-012, MGD-021; MG-H02, MG-M09, MG-M10, MG-M20, MG-M21.

### Requirement 13: MGR-013 — Focus Concurrency Correctness

**User Story:** As a fast explorer, I want results and inspector content to match current selection, so that stale asynchronous responses cannot mislead me.

#### Acceptance Criteria

1. WHEN a focus request starts, THE Memory_Control_Center SHALL bind lens instance, request generation, query parameters, policy hash, and base Graph_Revision to the request.
2. WHEN focus changes, THE Memory_Control_Center SHALL cancel prior cancellable work and increment the request generation.
3. IF a response generation, policy hash, or base revision differs from current state, THEN THE Memory_Control_Center SHALL discard the response.
4. WHEN single and double activation are supported, THE Memory_Control_Center SHALL assign disjoint actions to the gestures.
5. WHEN focus data is unavailable, THE Memory_Control_Center SHALL expose independent loading, partial, stale, offline, error, retry, and ready states.
6. IF a refresh removes the selected item, THEN THE Memory_Control_Center SHALL close or policy-safely re-resolve the selection and announce the reason.

**Traceability:** MG-H06, MG-M03, MG-M06, MG-O18.

### Requirement 14: MGR-014 — Accessible Graph Composite

**User Story:** As a keyboard or assistive-technology user, I want complete memory workflows without traversing hundreds of controls, so that visual exploration is not an accessibility barrier.

#### Acceptance Criteria

1. THE Memory_Control_Center SHALL conform to WCAG 2.2 AA for required workflows and states.
2. WHEN keyboard focus enters the graph, THE Memory_Control_Center SHALL use one composite tab stop with roving or spatial navigation.
3. THE Memory_Control_Center SHALL provide a synchronized semantic list or table for every graph-visible item and action.
4. WHEN a dialog, drawer, or inspector opens, THE Memory_Control_Center SHALL provide initial focus, focus containment, Escape behavior, inert background, concise announcement, and focus restoration.
5. WHEN state, authority, risk, direction, or selection is encoded, THE Memory_Control_Center SHALL provide text, shape, icon, pattern, or semantic-state redundancy in addition to color.
6. WHILE reduced motion, forced colors, high contrast, 200% zoom, font scaling, or screen reader mode is active, THE Memory_Control_Center SHALL preserve every core task.
7. WHEN accessibility release evidence is collected, THE Memory_Control_Center SHALL demonstrate search, inspect, trace, correct, relate, forget, restore, path, and focus-return workflows with keyboard and Orca.

**Traceability:** MG-H10–MG-H12, MG-M24, MG-M25, MG-L04, MG-L10, MG-O25.

### Requirement 15: MGR-015 — Authoritative Adaptive 2D View

**User Story:** As a user, I want a precise and calm 2D knowledge view, so that memory remains complete and trustworthy without optional graphics.

#### Acceptance Criteria

1. THE Authoritative_2D_View SHALL complete search, inspect, explain, correct, lifecycle, path, history, and Health workflows without optional 3D.
2. WHEN graph content is displayed, THE Authoritative_2D_View SHALL render only a query-defined bounded Semantic_Scene with honest totals and truncation.
3. WHEN density increases, THE Authoritative_2D_View SHALL apply label priority, collision handling, semantic aggregation, and measured culling without removing selected or focused semantics.
4. WHEN camera controls are used, THE Authoritative_2D_View SHALL provide fit-visible, fit-selection, fit-neighborhood, bounded pan, pointer-centroid zoom, pinch-centroid zoom, and navigation history.
5. WHEN the inspector changes available viewport, THE Authoritative_2D_View SHALL preserve or visibly mark the selected item.
6. WHILE decorative effects are disabled, THE Authoritative_2D_View SHALL retain complete meaning and action parity.
7. WHEN rendering tests run on Reference_Hardware, THE Authoritative_2D_View SHALL meet a 33.3 ms p95 interaction-frame budget at the declared scene cap.

**Traceability:** MGD-002, MGD-015; MG-H14, MG-H15, MG-M01, MG-M02, MG-L09, MG-L13.

### Requirement 16: MGR-016 — Responsive Input Model

**User Story:** As a mouse, keyboard, touch, or small-window user, I want controls and composition matched to context, so that memory tasks remain operable everywhere KRIA supports.

#### Acceptance Criteria

1. WHEN available width is at least 1200 CSS pixels, THE Memory_Control_Center SHALL present a 240-pixel navigation region, flexible workspace with minimum 560-pixel usable width, and reserved 360-pixel inspector without covering selected content; regions MAY collapse only through an explicit user action.
2. WHEN available width is between 800 and 1199 CSS pixels, THE Memory_Control_Center SHALL present a 72-pixel navigation rail, flexible workspace, and 320-pixel focus-managed overlay inspector that reframes or marks selected graph content.
3. WHEN available width is below 800 CSS pixels or available content height is below 600 CSS pixels, THE Memory_Control_Center SHALL use a single-column search-first layout, mutually exclusive map/list segment, and focus-managed full-height inspector sheet.
4. WHILE a coarse pointer is active, THE Memory_Control_Center SHALL provide targets of at least 44 by 44 CSS pixels, pinch-centroid zoom, two-finger pan, and non-hover alternatives.
5. WHILE keyboard input is active, THE Memory_Control_Center SHALL expose implemented spatial, fit, zoom, help, and action shortcuts with platform-correct labels.
6. WHEN 640×480 through ultrawide viewports, 100% through 200% scale, mixed DPI, RTL, CJK, and long labels are tested, THE Memory_Control_Center SHALL preserve readable, non-overlapping core actions.

**Traceability:** MGD-013; MG-H15, MG-M01, MG-M02, MG-M24, MG-L05, MG-O26.

### Requirement 17: MGR-017 — Fault Containment and Recovery

**User Story:** As a user, I want failures isolated and explained, so that a broken strategy or renderer cannot become a false empty memory.

#### Acceptance Criteria

1. WHEN data crosses a boundary, THE Cognitive_Memory_System SHALL validate identifiers, enums, labels, numbers, collections, bytes, revisions, cursors, and relation types.
2. IF one optional section fails, THEN THE Memory_Control_Center SHALL preserve valid sections and label the result Partial.
3. IF no usable current result is available, THEN THE Memory_Control_Center SHALL distinguish Empty, Offline, Unauthorized, Timeout, Malformed_Data, Worker_Failure, Renderer_Failure, and Error.
4. WHEN a recoverable failure occurs, THE Memory_Control_Center SHALL present retry, preserved intent, and a diagnostic correlation identifier.
5. IF an optional renderer fails, THEN THE Memory_Control_Center SHALL restore the same query, selection, and pending correction state in Authoritative_2D_View or list form.
6. WHEN startup or on-demand integrity verification runs, THE Cognitive_Memory_System SHALL verify SQLite integrity, schema/version compatibility, event ordering/checksums, outbox cursors, and derived-index manifests.
7. IF authority corruption is detected, THEN THE Cognitive_Memory_System SHALL enter Recovery_Mode, stop durable writes, identify the affected class without exposing content, and SHALL NOT synthesize authority; recovery MAY use only a verified local snapshot or Interchange_Export, otherwise the system SHALL fail closed.
8. IF FTS5, vectors, analytics, or another Derived_Index is corrupt, THEN THE Cognitive_Memory_System SHALL permit deletion and deterministic rebuild from authorized authority records while keeping unrelated authority available.
9. WHEN fault-injection tests run, THE Cognitive_Memory_System SHALL cover malformed rows, oversized labels, duplicate identifiers, worker crash, cursor expiry, patch gaps, database busy, embedder loss, event lag, WebGL context loss, seeded byte damage, event-checksum mismatch, interrupted rebuild, and failed verified recovery.

**Traceability:** MG-H16, MG-L02, MG-L11, MG-L12, MG-O23, MG-O24.

### Requirement 18: MGR-018 — Relation Ontology and Evidence

**User Story:** As a user, I want every relationship to state what the relationship means and why the relationship exists, so that edges are inspectable claims rather than decoration.

#### Acceptance Criteria

1. THE Relation_Registry SHALL define canonical name, display labels, aliases, direction class, inverse, reflexivity, endpoint kinds, validity policy, Evidence policy, and version for every writable relation.
2. WHEN a relationship is stored, THE Cognitive_Memory_System SHALL support multiple supporting and contradicting Evidence records.
3. WHEN relationship Evidence is displayed, THE Memory_Control_Center SHALL show policy-safe source, actor, method, time, polarity, and score semantics.
4. IF relationship strength lacks a named versioned derivation, THEN THE Cognitive_Memory_System SHALL omit strength.
5. WHEN an inferred relationship is displayed, THE Memory_Control_Center SHALL show algorithm version, rationale, Evidence basis, relative score semantics, and materialization status.
6. WHEN relation type, direction, Evidence, or validity changes, THE Cognitive_Memory_System SHALL preserve the previous version in history.
7. THE Relation_Registry SHALL define canonical Memory_Link semantics for `derived_from`, `supports`, `contradicts`, `mentions_entity`, and `superseded_by`, including direction, allowed endpoint kinds, provenance requirements, Effective_Policy propagation, Truth_State effects, and Valid_Time behavior.
8. WHEN mention extraction, consolidation, evidence attachment, contradiction handling, or supersession occurs, THE Cognitive_Memory_System SHALL use the canonical Memory_Link type and SHALL NOT create an untyped parallel link.

**Traceability:** MGD-008, MGD-009, MGD-017; MG-H13, MG-M05, MG-M07, MG-M12, MG-L03.

### Requirement 19: MGR-019 — Entity Provenance and Resolution

**User Story:** As a user, I want to understand and reverse canonical identity decisions, so that mistaken merges do not contaminate memory.

#### Acceptance Criteria

1. WHEN an entity mention is recorded, THE Cognitive_Memory_System SHALL preserve source record, span or structured locator, extractor or actor, model or rule version, role, time, and available score semantics.
2. WHEN the Entity_Resolution_Engine proposes a merge, THE Cognitive_Memory_System SHALL preserve the proposal rationale and keep source entities unchanged until governed acceptance.
3. IF a person merge relies only on name or embedding similarity, THEN THE Entity_Resolution_Engine SHALL classify the proposal as unresolved and prevent automatic merge.
4. WHEN a user accepts, rejects, splits, renames, retypes, adds an alias, or removes an alias, THE Write_Policy_Engine SHALL create a reversible audited correction.
5. WHEN a merge preview is requested, THE Entity_Resolution_Engine SHALL report surviving identity, affected aliases, relationships, Evidence, scopes, contradictions, and reversal behavior at one Graph_Revision.
6. WHEN source authorization or lifecycle changes, THE Cognitive_Memory_System SHALL recalculate entity and relationship visibility without revealing orphaned hidden contributors.

**Traceability:** MG-M13, MG-M14, MG-O03, MG-O12; risks R-WRONG-MERGE, R-POLICY-LEAK.

### Requirement 20: MGR-020 — Transport Capability Parity

**User Story:** As a client developer, I want one memory contract across supported hosts, so that Tauri and server adapters cannot invent different truth or policy.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL define DTOs, errors, limits, revisions, operations, and score semantics once in canonical contract modules.
2. WHEN a capability is supported by Tauri and server, THE Cognitive_Memory_System SHALL return normalized equivalent results for the same caller, revision, and request.
3. WHEN a capability is unsupported by a host, THE Cognitive_Memory_System SHALL publish the unsupported disposition and reason in a versioned capability matrix.
4. IF a capability is unavailable, THEN THE Memory_Control_Center SHALL omit or disable the control with an exact reason.
5. WHEN a public operation is added, THE Cognitive_Memory_System SHALL require canonical contract, host dispositions, policy tests, documentation, and traceability in the same change.
6. WHEN transport contract tests run, THE Cognitive_Memory_System SHALL compare security, pagination, temporal, error, revision, and lifecycle semantics across supported hosts.

**Traceability:** MGD-012, MGD-018, MGD-022; MG-M18; memory hardening API-1 and UI-1.

### Requirement 21: MGR-021 — Multi-Window Ownership

**User Story:** As a multi-window user, I want each Memory view to retain independent intent, so that one window cannot reset or leak another.

#### Acceptance Criteria

1. THE Memory_Control_Center SHALL assign each window or lens an independent query, selection, camera, focus generation, pending action, error, and navigation history.
2. WHEN caches are shared, THE Memory_Control_Center SHALL key immutable entries by schema, Graph_Revision, caller policy hash, and query hash.
3. WHEN a window closes or resets, THE Memory_Control_Center SHALL release only resources owned by that window.
4. WHEN patches arrive, THE Memory_Control_Center SHALL let each window independently apply or bounded-refetch the active query.
5. WHEN a detached view is restored, THE Memory_Control_Center SHALL validate saved state against current policy, schema, and Graph_Revision.
6. WHEN multi-window tests run, THE Memory_Control_Center SHALL cover simultaneous focus, write confirmation, lag, close, detach, restore, and scope change without cross-window mutation.

**Traceability:** MGD-014; MG-M19.

### Requirement 22: MGR-022 — Idle and Interaction Budgets

**User Story:** As a laptop user, I want memory visuals to become quiet when idle, so that the Control Center does not waste battery or compete with local AI.

#### Acceptance Criteria

1. WHEN interaction, layout, and finite transitions stop for two seconds, THE Memory_Control_Center SHALL stop graph-originated continuous animation and render loops.
2. WHILE reduced motion is active, THE Memory_Control_Center SHALL disable nonessential motion immediately.
3. WHEN motion is used, THE Memory_Control_Center SHALL bind the motion to focus, selection, state transition, or navigation continuity and complete the motion within 400 ms.
4. WHEN the graph is idle for 60 seconds on Reference_Hardware, THE Memory_Control_Center SHALL remain within two CPU percentage points of the blank Memory view.
5. WHEN 20 query, inspect, and close cycles complete, THE Memory_Control_Center SHALL return retained heap and scene resources to the documented steady-state bound.
6. WHILE local model pressure or power-saving policy is active, THE Memory_Control_Center SHALL reduce effects, labels, analytics, and scene size before removing core tasks.

**Traceability:** MG-H14, MG-M22, MG-M23, MG-M26; MG-O23.

### Requirement 23: MGR-023 — Scale-Aware Subgraph Navigation

**User Story:** As a user with years of memory, I want bounded task-relevant views, so that corpus growth does not create a hairball or hide result limits.

#### Acceptance Criteria

1. WHEN the Control Center opens, THE Cognitive_Memory_System SHALL load a bounded overview or recent task context rather than full adjacency.
2. WHEN a user expands an entity, aggregate, path, trace, time, or Health issue, THE Cognitive_Memory_System SHALL return a query-defined subgraph with authorized totals and frontier metadata.
3. WHEN semantic zoom changes level, THE Memory_Control_Center SHALL preserve selected identity and display the aggregation rule.
4. IF a window reaches its node, edge, label, or byte cap, THEN THE Memory_Control_Center SHALL display truncation and offer a narrower query or explicit expansion cursor.
5. WHEN 100, 1,000, 10,000, and 100,000-record fixtures run, THE Cognitive_Memory_System SHALL prove common query cost is bounded by requested window and indexed selectivity rather than full corpus adjacency.
6. WHERE community, bridge, hub, orphan, or gap analysis is available, THE Cognitive_Memory_System SHALL expose named algorithm metadata and policy-safe drill-down.

**Traceability:** MGD-015; MG-H09, MG-H14, MG-O06–MG-O08, MG-O14, MG-O15, MG-O21.

### Requirement 24: MGR-024 — Explain and Correct Workflows

**User Story:** As a user, I want one path from claim to evidence to correction, so that memory control is operational rather than decorative.

#### Acceptance Criteria

1. WHEN a memory, entity, relationship, goal, or trace item is selected, THE Memory_Control_Center SHALL open a structured inspector with Identity, Truth, Evidence, Relationships, Use, History, and Actions sections.
2. WHEN an inspector section loads independently, THE Memory_Control_Center SHALL expose section-specific loading, empty, partial, stale, offline, error, and retry states.
3. WHEN a correction is initiated, THE Memory_Control_Center SHALL show current value, proposed value, Evidence, affected scope, impact count, reversibility, and base revision before commit.
4. IF authority revision changes before correction confirmation, THEN THE Memory_Control_Center SHALL require a refreshed preview.
5. WHILE a correction is pending, THE Memory_Control_Center SHALL preserve the initiating context and distinguish pending from committed state.
6. WHEN a correction commits, THE Memory_Control_Center SHALL show resulting revision, audit entry, affected records, and available undo.
7. WHEN a contradiction remains unresolved, THE Memory_Control_Center SHALL show competing beliefs and offer evidence-aware confirm, supersede, or keep-both actions according to capability.

**Traceability:** MGD-001; MG-C02, MG-C04, MG-C07, MG-O01–MG-O04, MG-O08, MG-O10–MG-O13, MG-O31.

### Requirement 25: MGR-025 — Retrieval-Use Trace Integration

**User Story:** As a user, I want to know why KRIA recalled or ignored memory for an answer, so that answer influence is verifiable.

#### Acceptance Criteria

1. WHEN retrieval runs for a response or task, THE Retrieval_Engine SHALL persist or return a policy-safe Retrieval_Trace with strategy candidates, fusion contributions, gating decisions, token allocation, model versions, and context-injected identifiers.
2. WHEN a user opens `Why this answer`, THE Memory_Control_Center SHALL distinguish Used_Item, Retrieved_Filtered_Item, and merely available memory.
3. WHEN a Used_Item is displayed, THE Memory_Control_Center SHALL link the item to the response or task, Retrieval_Trace, source Evidence, and relevant revision.
4. IF a filtered item is unauthorized, THEN THE Retrieval_Engine SHALL expose a policy-safe reason code without label, content, hidden count, or topology.
5. WHEN trace data is incomplete, THE Memory_Control_Center SHALL label the trace Partial and avoid reconstructing model influence from graph proximity.
6. WHEN retrieval trace tests run, THE Cognitive_Memory_System SHALL prove that every displayed Used_Item belongs to the recorded injected set.
7. WHEN the system explains a record, THE Memory_Control_Center SHALL distinguish `Why stored` (write-decision record), `Why recalled` (retrieval strategy/fusion), and `How used` (context-injection proof) as three separate explanations.
8. THE Write_Policy_Engine SHALL persist a policy-safe write-decision record containing accepted, rejected, or deferred disposition; policy version; source event; actor; and non-secret reason codes.
9. THE Memory_Control_Center SHALL NOT infer answer use from retrieval candidacy, graph distance, focus, visibility, or storage rationale.

**Traceability:** MG-C04, MG-O01; memory law L6 and CP-9.

### Requirement 26: MGR-026 — Visual Authority Encoding

**User Story:** As a user, I want visual appearance to reflect actual memory semantics, so that color, shape, depth, and motion never fabricate confidence or topology.

#### Acceptance Criteria

1. THE Memory_Control_Center SHALL encode record kind with text plus shape or icon.
2. THE Memory_Control_Center SHALL encode stored, derived, inferred, and navigation edges with distinct text-accessible patterns.
3. THE Memory_Control_Center SHALL encode Current, Unverified, Stale, Contradicted, Superseded, Forgotten, and Used states without relying on opacity alone.
4. WHEN a legend is displayed, THE Memory_Control_Center SHALL generate legend entries only for semantic encodings present in the current scene.
5. IF confidence, strength, importance, freshness, or direction is unavailable, THEN THE Memory_Control_Center SHALL omit the corresponding visual channel.
6. WHEN hidden policy classes exist, THE Memory_Control_Center SHALL avoid colors, gaps, counts, or placeholders that reveal protected categories.
7. THE Memory_Control_Center SHALL centralize typography, spacing, radius, elevation, focus, motion, state, and authority tokens; body text SHALL be at least 14 CSS pixels, graph labels at least 12 CSS pixels at readable LOD, and visible focus at least 2 CSS pixels with WCAG 2.2 AA contrast.
8. THE Memory_Control_Center SHALL achieve futuristic quality through semantic hierarchy, provenance-linked depth cues, precise geometry, restrained material layers, deterministic polish, and finite state transitions; ambient particles, gratuitous glow, fake holograms, and perpetual GPU loops SHALL NOT carry meaning.
9. WHEN visual regression evidence is reviewed, THE reviewer SHALL verify semantic accuracy in addition to pixel stability.

**Traceability:** MG-C03, MG-H13, MG-M24, MG-M25, MG-L03, MG-L08, MG-L10, MG-O27.

### Requirement 27: MGR-027 — Testing and Evidence Gates

**User Story:** As a release owner, I want every capability proven at the correct layer, so that planned architecture and polished screenshots cannot substitute for production evidence.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL maintain deterministic 100, 1,000, 10,000, and 100,000-record fixtures with planted truth, contradictions, aliases, relationships, paths, scopes, deletion states, goals, traces, and failures.
2. WHEN a release gate runs, THE test system SHALL execute applicable unit, property, schema, migration, contract, integration, E2E, lifecycle, retrieval-quality, visual, accessibility, performance, security, and fault tests.
3. WHEN a property test runs, THE test system SHALL execute at least 100 generated cases and annotate the test with `Feature: memory-graph-production-redesign, Property N`.
4. WHEN performance evidence is captured, THE test system SHALL record commit, fixture seed, hardware, OS, power mode, build profile, dependency lock, model versions, warm or cold state, and p50, p95, and p99.
5. WHEN visual evidence is captured, THE test system SHALL use deterministic same-seed fixtures and record reference, before, and after captures for every changed state across dark, light, forced colors, reduced motion, 640×480, 800×600, 1176×775, 1440×900, 1920×1080, ultrawide, 100%–200% scale, long labels, RTL, CJK, and all applicable truth/error/loading/empty/partial states.
6. WHEN visual evidence is reviewed, a human semantic reviewer SHALL verify no invented topology, clipping, overlap, hidden focus, misleading score/provenance, inaccessible contrast, map/list mismatch, or action whose consequence is visually unclear; automated pixel diff SHALL NOT substitute for this sign-off.
7. IF a P0 gate lacks an Evidence_Artifact, THEN THE release system SHALL keep the feature status Planned or Unverified.
8. IF any Critical or High audit finding lacks verified evidence, THEN THE release system SHALL block public readiness.

**Traceability:** MG-M27; all MG-C/H/M/L and MG-O rows; memory CP-1–CP-14 and AUD-01/AUD-02.

### Requirement 28: MGR-028 — Privacy-Safe Observability

**User Story:** As a maintainer, I want actionable memory diagnostics without private content, so that production failures can be found without creating a second leak surface.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL emit structured metrics for query latency, strategy use, cache hit, Graph_Revision lag, outbox lag, enrichment depth, scheduler work, frame time, fallback, and data-quality counts.
2. WHEN logs or metrics are emitted, THE Cognitive_Memory_System SHALL use correlation identifiers and aggregate dimensions without memory content, secret values, private labels, embeddings, or hidden identifiers.
3. WHEN a user opens Health, THE Memory_Control_Center SHALL show authority status, index status, model availability, backlog, last verified times, degradation, and remediation actions.
4. WHERE developer diagnostics are enabled, THE Memory_Control_Center SHALL gate detailed query plans, scene counts, cache keys, and fault data to local developer mode.
5. IF telemetry overhead exceeds one percent CPU at idle or one percent of measured interactive latency, THEN THE Cognitive_Memory_System SHALL reduce sampling or disable nonessential diagnostics.
6. WHEN observability security tests run, THE test system SHALL inspect logs, traces, crash reports, and metrics for protected content.

**Traceability:** MG-H03, MG-H14, MG-M27, MG-O24; hardening M5, L3, R2.

### Requirement 29: MGR-029 — Documentation Authority and Audit Continuity

**User Story:** As an implementation model, I want one traceable requirements authority, so that old claims and checkboxes cannot cause goal drift.

#### Acceptance Criteria

1. THE documentation set SHALL distinguish current-state architecture, current executable contracts, host capabilities, future requirements, design decisions, implementation tasks, and verification evidence.
2. WHEN shipped behavior changes, THE documentation set SHALL update the current-state authority and executable contract evidence in the same change.
3. WHEN a requirement changes, THE documentation set SHALL preserve replaced text or rationale through version control and update affected decision, audit, test, and evidence mappings.
4. IF a task checkbox lacks linked implementation and validation evidence, THEN THE documentation set SHALL treat the checkbox as historical planning state rather than completion proof.
5. THE documentation set SHALL preserve traceability for MGD-001–MGD-022, MGR-001–MGR-048, MG-C01–MG-C07, MG-H01–MG-H17, MG-M01–MG-M28, MG-L01–MG-L13, MG-O01–MG-O31, memory laws L1–L12, and the hardening identifiers cited by this document.
6. WHEN a capability is removed after a failed gate, THE documentation set SHALL retain the decision evidence and remove claims that the capability ships.

**Traceability:** MGD-018, MGD-022; MG-M28; hardening DOC-1.

### Requirement 30: MGR-030 — Optional True-3D Decision

**User Story:** As a user, I want spatial depth only when depth improves a real memory task, so that optional graphics do not create cost or false meaning.

#### Acceptance Criteria

1. WHERE optional 3D is evaluated, THE Memory_Control_Center SHALL use only dependencies with approved free and open-source licenses.
2. WHERE optional 3D is evaluated, THE Memory_Control_Center SHALL map the z-axis to one documented semantic variable backed by authority data.
3. WHERE optional 3D is evaluated, THE Memory_Control_Center SHALL consume the same Semantic_Scene and typed actions as Authoritative_2D_View.
4. WHEN the preregistered task study runs, THE optional 3D representation SHALL improve median completion time or error rate by at least 10 percent over Authoritative_2D_View for the selected task.
5. WHEN the real-scene target-hardware profile runs, THE optional 3D representation SHALL sustain at least 30 frames per second, reach idle quiet, recover from context loss, and preserve reduced-motion fallback.
6. IF any semantic, task-benefit, accessibility, resource, licensing, or maintainability gate fails, THEN THE codebase SHALL delete optional 3D controls, graph-only dependencies, dormant renderer code, and shipping claims.
7. IF every optional 3D gate passes, THEN THE Memory_Control_Center SHALL keep Authoritative_2D_View complete, default-capable, and available as immediate fallback.

**Traceability:** MGD-002, MGD-016, MGD-021; MG-C01, MG-H02, MG-M20–MG-M23, MG-L12, MG-O20.

### Requirement 31: MGR-031 — Control Center Information and Interaction Integrity

**User Story:** As a user, I want a coherent Memory Control Center organized around decisions, so that I can understand and control memory without learning storage internals.

#### Acceptance Criteria

1. THE Memory_Control_Center SHALL provide primary destinations Overview, Recall, Knowledge, Timeline, Goals, Sources, and Health.
2. WHEN Overview loads, THE Memory_Control_Center SHALL show exact authority state, current degradation, recent changes, unresolved contradictions, active goals, pending cognition, and safe next actions.
3. WHEN Recall loads, THE Memory_Control_Center SHALL prioritize full-corpus search, retrieval rationale, saved filters, and answer-trace entry points.
4. WHEN Knowledge loads, THE Memory_Control_Center SHALL provide Authoritative_2D_View, synchronized list, bounded exploration, and structured inspector.
5. WHEN Timeline loads, THE Memory_Control_Center SHALL provide valid-time and transaction-time filters only for supported snapshots and diffs.
6. WHEN Goals loads, THE Memory_Control_Center SHALL show active, paused, completed, conflicted, and stale goals with Evidence and resumption context.
7. WHEN Sources loads, THE Memory_Control_Center SHALL show library items, native tools, MCP servers, OpenClaw skills, sidecars, imports, policy, derived records, and lifecycle actions.
8. WHEN Health loads, THE Memory_Control_Center SHALL show trustworthy readiness, degradation, backlog, resource pressure, index state, and Evidence_Artifact links.
9. WHEN a control is displayed, THE Memory_Control_Center SHALL provide an implemented action, accurate label, semantic state, accessible name, loading behavior, failure behavior, and help text where the consequence is not obvious.
10. IF the Authority_Store contains no authorized memory, THEN THE Memory_Control_Center SHALL provide goal-led onboarding without claiming extraction, models, or scans already occurred.
11. THE Memory_Control_Center SHALL present Overview, Recall, Knowledge, Timeline, Goals, Sources, and Health as one Digital_Twin synchronized to one Graph_Revision and one caller-policy context across map, list, inspector, and status surfaces.
12. WHEN Digital_Twin state is displayed, THE Memory_Control_Center SHALL explain it in human language and SHALL NOT claim consciousness, sentience, emotions, autonomous desires, or a literal brain.
13. IF one destination cannot represent the current revision or policy context, THEN THE Memory_Control_Center SHALL mark that destination stale or unavailable rather than combine revisions.

**Traceability:** MGD-001; MG-H12, MG-M09, MG-M10, MG-L02, MG-L05, MG-L06, MG-L11, MG-L13.

### Requirement 32: MGR-032 — Decades-Long Analytical Evolution Seam

**User Story:** As an architect, I want replaceable derived analytics behind stable contracts, so that years of corpus growth do not force premature distributed architecture.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL keep Authority_Store independent from Graph_Query and Graph_Analytics implementation choices.
2. THE Cognitive_Memory_System SHALL implement the current release with indexed SQLite reads, bounded workers, revision caches, and rebuildable derived indexes.
3. IF measured 100,000-record evidence misses accepted budgets after query and index optimization, THEN THE architecture review SHALL evaluate a rebuildable analytical backend through the existing port.
4. WHERE a derived analytical backend is introduced, THE Cognitive_Memory_System SHALL rebuild the backend from Authority_Store, consume ordered revisions, enforce Effective_Policy before results, and preserve canonical contracts.
5. THE Cognitive_Memory_System SHALL reserve versioned seams for model changes, schema changes, new record kinds, and Interchange_Export without implementing multi-device synchronization or consensus in the current release.
6. WHEN an Interchange_Export is created, THE Cognitive_Memory_System SHALL produce a self-describing open-format manifest with schema, ontology, relation-registry, algorithm, and model versions; checksums; selected events/records/Memory_Links; provenance; truth/lifecycle state; ordering; and explicit scope, while excluding secrets outside export authorization.
7. WHEN an export is imported into an empty compatible Authority_Store, THE Cognitive_Memory_System SHALL preserve semantic identifiers, ordering, links, provenance, truth/lifecycle state, and authorized content under deterministic round-trip comparison.
8. WHEN import runs, THE Write_Policy_Engine SHALL validate the whole manifest before commit, apply records idempotently, reject unknown required semantics atomically, and preserve unknown optional fields for re-export.
9. WHEN a schema version is released, THE codebase SHALL retain a deterministic migration fixture from every prior released schema version to the current version and SHALL test fresh create, upgrade, export, import, and rebuild.
10. WHEN dead legacy or duplicate paths remain after a hard cutover, THE codebase SHALL delete the paths rather than maintain compatibility scaffolding.

**Traceability:** MGD-015, MGD-019; MG-M15, MG-O21, MG-O29; dev-context single-laptop constraint.

### Requirement 33: MGR-033 — Single SQLite Authority and Append-Only Events

**User Story:** As a KRIA engineer, I want one transactional authority and immutable event history, so that every derived view can be explained and rebuilt without split truth.

#### Acceptance Criteria

1. THE Authority_Store SHALL be the sole transactional authority for immutable interaction and tool Events, Memories, Entities, Aliases, Mentions, Relationships, Goals, Provenance, Retrieval_Traces, Audit_Records, outbox items, and Graph_Revisions; no alternate durable store SHALL accept authoritative writes.
2. WHEN an interaction or tool invocation begins and completes, THE Cognitive_Memory_System SHALL append separate start and completion Events linked by invocation identity, including typed success, partial, failure, cancellation, or timeout outcome.
3. WHEN durable state changes, THE Cognitive_Memory_System SHALL commit authority rows, immutable Event_Log entry, Audit_Record, Graph_Revision when applicable, and Derived_Index outbox work in one Authority_Transaction.
4. WHEN an Event is appended, THE Authority_Store SHALL assign stable event identifier, sortable logical time, UTC time, source timezone offset, source identity, payload checksum, and optional shred-key reference.
5. IF an Event update or delete is attempted, THEN THE Authority_Store SHALL abort the operation through database-enforced immutability.
6. WHEN the same source event identifier or idempotency key is replayed, THE Authority_Store SHALL create one semantic ingestion result and return the original committed result.
7. WHEN Event_Log retention moves content to cold segments, THE Cognitive_Memory_System SHALL preserve immutable checksums, ordered cursors, queryability, and erasure-key references.
8. IF a legacy memory store conflicts with Authority_Store during hard cutover, THEN THE migration process SHALL select the declared authority, produce a reconciliation report, and remove the competing write path.

**Traceability:** Memory laws L1–L4, ADR-004, ADR-005; hardening H2, R1; existing schema `0001_init.sql`.

### Requirement 34: MGR-034 — Typed Cognitive Records and Provenance

**User Story:** As a user, I want every memory object typed and source-linked, so that KRIA can explain origin, transformation, and authority.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL represent Event, Memory, Entity, Alias, Mention, Relationship, Evidence, Goal, Episode, Summary, Skill, Rule, Retrieval_Trace, Feedback, and Audit_Record as explicit versioned types.
2. WHEN a Cognitive_Record is created, THE Cognitive_Memory_System SHALL assign stable identifier, schema version, source, actor, creation time, Effective_Policy, Truth_State, Valid_Time when applicable, and provenance links.
3. WHEN a record is derived, THE Cognitive_Memory_System SHALL preserve all immediate parent identifiers, derivation method, method version, and derivation time.
4. WHEN source content has a structured location, THE Cognitive_Memory_System SHALL preserve a policy-safe locator such as event identifier, library item and chunk, tool invocation, MCP server and tool, OpenClaw skill, file span, or response turn.
5. IF a forward-compatible enum or record version is unknown, THEN THE Cognitive_Memory_System SHALL preserve the raw value for read diagnostics and deny unsafe writes using the unknown value.
6. WHEN a record is serialized and deserialized through a supported contract version, THE Cognitive_Memory_System SHALL preserve all semantically significant fields.

**Traceability:** Memory design R1–R20, API-1; MG-C02, MG-H13, MG-M14.

### Requirement 35: MGR-035 — Mandatory Write Policy and Memory Modes

**User Story:** As a user, I want every durable learning decision governed consistently, so that tools, models, and interfaces cannot bypass consent or quality rules.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL expose no durable write path outside the Write_Policy_Engine.
2. WHEN a write candidate arrives, THE Write_Policy_Engine SHALL perform deterministic mode, identity, namespace, scope, sensitivity, security, idempotency, and quality admission before asynchronous enrichment.
3. WHEN deterministic admission is measured on Reference_Hardware, THE Write_Policy_Engine SHALL complete the policy evaluation portion within 2 ms p95 excluding Authority_Store commit latency.
4. WHILE Permanent mode is active, THE Write_Policy_Engine SHALL admit approved durable writes according to policy.
5. WHILE Temporary or Session_Only mode is active, THE Write_Policy_Engine SHALL bind admitted records to the current session and make the records unavailable after session purge.
6. WHILE Read_Only mode is active, THE Write_Policy_Engine SHALL reject durable writes with a typed mode response and preserve retrieval.
7. WHILE Disabled mode is active, THE Cognitive_Memory_System SHALL avoid durable memory reads and writes and expose an honest degraded tool surface.
8. IF a candidate proposes a Rule from insufficient, correlated, self-reflective, or single-source Evidence, THEN THE Write_Policy_Engine SHALL reject promotion and record the reason.

**Traceability:** Memory law L3, D-19; hardening M2, M5, AUD-01, AUD-02; MGD-010.

### Requirement 36: MGR-036 — Five-Strategy Hybrid Retrieval

**User Story:** As a user, I want recall to combine meaning, exact text, relationships, time, and goals, so that KRIA retrieves useful context without opaque shortcuts.

#### Acceptance Criteria

1. WHEN retrieval runs, THE Retrieval_Engine SHALL evaluate policy-authorized FTS5, local FastEmbed `all-MiniLM-L6-v2` 384-dimensional vector, maximum-three-hop graph, temporal, and active-goal strategies that are available for the query.
2. THE current vector implementation SHALL be SQLiteVectorStore behind VectorStore_Port, using exact policy-filtered brute-force cosine over model-compatible versioned SQLite vectors; LanceDB, Qdrant, and approximate-nearest-neighbor backends SHALL NOT be current-release dependencies or authorities.
3. THE build and model manifest SHALL pin the embedding model identity, source, license, checksum, dimensions, tokenizer/runtime compatibility, and output normalization contract.
4. WHEN strategies return candidates, THE Retrieval_Engine SHALL fuse them using an Adaptive_RRF_Profile and retain profile ID, RRF `k`, query class, strategy availability, per-strategy ranks, weights, and final contributions.
5. WHEN query intent is classified, THE Retrieval_Engine SHALL record the deterministic query class, classifier version, and selected Adaptive_RRF_Profile in Retrieval_Trace.
6. WHEN a token budget is supplied, THE Retrieval_Engine SHALL select context by fused relevance, diversity, policy, Truth_State, active-goal contribution, and token cost rather than fixed top-K alone.
7. WHEN a candidate is stale, contradicted, superseded, forgotten, deleted, or unverified, THE Retrieval_Engine SHALL apply the declared truth-state policy before context injection.
8. IF the embedder or vector partition is unavailable, THEN THE Retrieval_Engine SHALL continue with FTS5 plus available graph, temporal, and goal strategies, mark the result Partial, and record the omission in Retrieval_Trace.
9. IF graph, temporal, or goal strategy is unavailable, THEN THE Retrieval_Engine SHALL continue with remaining strategies and identify the omitted strategy in Retrieval_Trace.
10. WHEN retrieval is evaluated on a versioned corpus of at least 200 human-judged queries, THE Retrieval_Engine SHALL achieve Recall@10 ≥0.85, nDCG@10 ≥0.80, exact phrase/identifier success ≥0.95, forbidden-item exclusion 100%, and superseded/deleted exclusion 100%.
11. IF a retrieval change causes more than 0.03 absolute regression in Recall@10, nDCG@10, or exact-match rate outside the 95% bootstrap confidence interval, THEN the release system SHALL block release unless the approved evaluation corpus and requirement are versioned with explicit rationale.
12. WHEN retrieval is evaluated on the 100,000-record fixture, THE Retrieval_Engine SHALL meet 120 ms p95 core retrieval latency on Reference_Hardware after declared warm-up.

**Traceability:** Memory laws L8, L10, L12; current `retriever.rs` vector+FTS baseline; memory Requirement 13; MG-H01.

### Requirement 37: MGR-037 — Truth Maintenance and Supersession

**User Story:** As a user, I want KRIA to preserve disagreement and history without presenting stale beliefs as current, so that recall remains correctable over time.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL assign each claim a staleness class and Truth_State.
2. WHEN new Evidence contradicts current memory, THE Cognitive_Memory_System SHALL evaluate deterministic precedence in the order user-confirmed source, verification recency, independent Evidence quality, and statistically significant Memory Worth.
3. IF no contradiction candidate dominates, THEN THE Cognitive_Memory_System SHALL preserve competing beliefs and mark the conflict unresolved.
4. WHEN one record supersedes another, THE Cognitive_Memory_System SHALL preserve the superseded record, link the successor, close applicable Valid_Time, and exclude the superseded record from default current recall.
5. WHEN a verifiable volatile record becomes due, THE Cognitive_Memory_System SHALL execute or queue the declared verification predicate before presenting the record as current.
6. IF verification cannot run, THEN THE Cognitive_Memory_System SHALL mark the record Unverified or Stale and preserve the last verified value with time.
7. WHEN a user resolves a contradiction, THE Write_Policy_Engine SHALL preserve the prior beliefs, Evidence, user decision, and reversal information.
8. WHEN relationship Evidence contradicts an edge, THE Cognitive_Memory_System SHALL apply the same explicit contradiction and supersession semantics used for Memory claims.

**Traceability:** ADR-009; current `truth.rs`; memory Requirement 6 and CP-14; MG-O11.

### Requirement 38: MGR-038 — Active Goals and Goal-Aware Recall

**User Story:** As a user, I want KRIA to remember active goals and resumption context, so that recall supports current work without converting guesses into commitments.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL represent goal kind, title, status, priority, confidence semantics, owner, scope, creation time, progress time, Evidence, and resumption context.
2. WHEN goal-aware retrieval runs, THE Retrieval_Engine SHALL consider only goals authorized for the caller and active for the task context.
3. WHEN an active goal contributes to ranking, THE Retrieval_Engine SHALL record the goal identifier and contribution in the Retrieval_Trace.
4. IF a goal is inferred rather than user-confirmed, THEN THE Cognitive_Memory_System SHALL label the goal Candidate and prevent autonomous promotion to Active without policy evidence.
5. WHEN a goal is paused, completed, contradicted, superseded, or deleted, THE Retrieval_Engine SHALL stop applying the goal as active ranking context.
6. WHEN the Goals destination displays a goal, THE Memory_Control_Center SHALL show status, provenance, linked memory, progress history, conflicts, and resume action.

**Traceability:** Existing `goals` schema; memory design reasoning context; current hardening finding M1.

### Requirement 39: MGR-039 — Cognitive Consolidation with Source Preservation

**User Story:** As a user, I want experience compressed into useful summaries, skills, and rules while retaining sources, so that KRIA improves without rewriting history.

#### Acceptance Criteria

1. WHEN consolidation runs, THE Cognitive_Scheduler SHALL process bounded candidate sets through Episode to Summary to Skill to Rule levels according to declared policies.
2. WHEN a Summary, Skill, or Rule is produced, THE Cognitive_Memory_System SHALL preserve complete `derived_from` links to source records and derivation method version.
3. WHEN consolidation archives a source record, THE Cognitive_Memory_System SHALL keep the source retrievable through history and Evidence workflows unless lifecycle policy deletes the source.
4. WHEN self-reflection produces a candidate, THE Write_Policy_Engine SHALL classify the source as untrusted self-reflection and cap confidence at 0.6 before independent verification.
5. IF a Skill or Rule lacks the configured minimum independent Evidence count, source diversity, success observations, or contradiction check, THEN THE Write_Policy_Engine SHALL keep the candidate below the proposed compression level.
6. WHEN consolidation is replayed with unchanged inputs and algorithm version, THE Cognitive_Memory_System SHALL produce the same semantic output identity without duplicate records.
7. IF consolidation stops or crashes, THEN THE Cognitive_Memory_System SHALL resume from a durable cursor without losing source lineage or duplicating output.
8. WHEN a consolidated record is corrected, THE Cognitive_Memory_System SHALL mark dependent summaries, skills, and rules stale and queue bounded reevaluation.

**Traceability:** Memory Requirement 21, compression levels 0–3, L11; hardening R1/R2.

### Requirement 40: MGR-040 — Memory Lifecycle and User-Controlled Erasure

**User Story:** As a user, I want reversible forgetting and explicit permanent deletion, so that I control memory without confusing concealment with erasure.

#### Acceptance Criteria

1. WHEN a user forgets an authorized record, THE Cognitive_Memory_System SHALL set Truth_State to Forgotten, exclude the record from default retrieval, preserve audit history, and provide a 30-day restore window.
2. WHEN a user restores a Forgotten record within the restore window, THE Cognitive_Memory_System SHALL restore the same stable identity and governed active state.
3. WHEN the restore window expires or immediate hard delete is confirmed, THE Cognitive_Memory_System SHALL mark authority content Deleted, remove FTS5 entries, purge vectors, remove graph projections, close dependent relations, and queue reconciliation.
4. WHEN a source item, session, namespace, tool source, MCP source, OpenClaw source, or subject is deleted, THE Cognitive_Memory_System SHALL calculate and display dependent records before executing the authorized cascade.
5. IF dependent records retain independent Evidence, THEN THE Cognitive_Memory_System SHALL offer policy-valid keep-with-source-deleted or cascade choices and record the decision.
6. WHEN hard deletion completes, THE Cognitive_Memory_System SHALL return zero deleted content through retrieval, graph, trace, inspector, export, cache, and Derived_Index reads.
7. IF deletion fails after authority commit but before Derived_Index purge, THEN THE Cognitive_Memory_System SHALL preserve Deleted authority state and complete purge through idempotent reconciliation.
8. WHEN lifecycle controls are displayed, THE Memory_Control_Center SHALL distinguish Forget, Restore, Hard Delete, and Crypto-Shred consequences in plain language.

**Traceability:** Memory Requirement 10, CP-10; current `lifecycle.rs`; MG-O10, MG-O11.

### Requirement 41: MGR-041 — Cryptographic Shredding Truth

**User Story:** As a user, I want permanent-erasure claims backed by cryptographic evidence, so that a database status flag is not presented as unreadability.

#### Acceptance Criteria

1. THE Cognitive_Memory_System SHALL describe Crypto-Shredding as available only when protected payloads are encrypted with subject-bound key material outside the payload and key destruction makes decryption fail.
2. WHEN a subject is assigned cryptographic erasure protection, THE Cognitive_Memory_System SHALL associate eligible Event and Memory payloads with a stable shred-key identifier and key version.
3. WHEN Crypto-Shredding executes, THE Cognitive_Memory_System SHALL destroy the recoverable subject key material, record destruction time and method, and retain only non-secret audit proof.
4. WHEN a destroyed key is used in a decryption test, THE Cognitive_Memory_System SHALL fail closed and return no plaintext through current, historical, backup, cache, or index paths.
5. IF only a shred-key status row is updated while recoverable key material or plaintext remains, THEN THE Memory_Control_Center SHALL label the operation Hard Delete Pending Cryptographic Erasure rather than Crypto-Shredded.
6. WHERE application-level encryption is deferred for the current laptop stage, THE documentation set SHALL state the reliance on OS disk encryption and shall not claim application-level cryptographic unreadability.
7. WHEN cryptographic implementation is introduced, THE release system SHALL require threat review, key-lifecycle tests, backup interaction tests, and dependency-license approval.

**Traceability:** Memory law L9, ADR-006; current `lifecycle.rs` MVP limitation; dev-context encryption deferral.

### Requirement 42: MGR-042 — Derived-Index Convergence and Model Migration

**User Story:** As a user, I want search indexes to converge with authority across crashes and model changes, so that recall cannot silently drift.

#### Acceptance Criteria

1. WHEN an authority change affects a Derived_Index, THE Authority_Store SHALL enqueue target, operation, content hash, model version, and record identity inside the same Authority_Transaction.
2. WHEN a Derived_Index relay applies an outbox item, THE Cognitive_Memory_System SHALL make application idempotent by semantic target and content version.
3. IF relay retries exceed the configured budget, THEN THE Cognitive_Memory_System SHALL move the item to a diagnosable dead-letter state and preserve reconciliation eligibility.
4. WHEN reconciliation runs, THE Cognitive_Memory_System SHALL add missing entries, remove orphan entries, repair version mismatches, and report counts without modifying semantic authority.
5. WHEN an embedding model changes, THE Cognitive_Memory_System SHALL maintain explicit model versions, query compatible partitions, and migrate incrementally under resource policy.
6. IF an embedding dimension or model hash mismatches the declared partition, THEN THE Retrieval_Engine SHALL reject the partition and continue with available retrieval strategies.
7. WHEN a Derived_Index is deleted and rebuilt from Authority_Store, THE Cognitive_Memory_System SHALL reproduce equivalent authorized record membership and version semantics.
8. WHEN crash recovery tests interrupt authority commit, outbox relay, model migration, and reconciliation, THE Cognitive_Memory_System SHALL converge without duplicate semantic records or lost deletions.

**Traceability:** ADR-005, D-5, D-16; memory Requirement 12 and 22, CP-4; hardening H2, R1, R2.

### Requirement 43: MGR-043 — Native, MCP, OpenClaw, Sidecar, and Tool Isolation

**User Story:** As a user, I want every execution source confined to authorized memory, so that extensibility cannot read or poison unrelated knowledge.

#### Acceptance Criteria

1. WHEN a native tool, MCP server, OpenClaw skill, sidecar, import, or cloud result enters memory, THE Write_Policy_Engine SHALL assign a source-specific namespace, trust class, scope, sensitivity, invocation identifier, and capability context.
2. WHEN an MCP server or OpenClaw skill reads memory, THE Cognitive_Memory_System SHALL restrict the read to the source namespace plus explicitly authorized public-core records.
3. WHEN an MCP server or OpenClaw skill proposes a write, THE Cognitive_Memory_System SHALL require orchestrator mediation and Write_Policy_Engine admission.
4. IF a plugin-originated record requests promotion to core memory, THEN THE Write_Policy_Engine SHALL require explicit user approval or the versioned high-evidence policy.
5. WHEN untrusted external content is admitted, THE Cognitive_Memory_System SHALL preserve the content as data, apply injection scanning and contextual fencing, and prevent content text from invoking actions.
6. WHEN tool capability or identity changes, THE Cognitive_Memory_System SHALL invalidate incompatible Retrieval_Trace, caches, and pending writes.
7. WHEN isolation tests run, THE Cognitive_Memory_System SHALL prove zero cross-namespace reads, writes, counts, timing distinctions, graph paths, inferred endpoints, and trace details for unauthorized native, MCP, OpenClaw, sidecar, and server callers.
8. IF the memory backend is unavailable, THEN tool adapters SHALL return an explicit no-memory degraded response rather than write to an alternate store.

**Traceability:** Memory Requirement 19; MGD-007; current `Source` and `ScopeFilter`; OpenClaw capability grants; hardening S1, S2, DC2.

### Requirement 44: MGR-044 — Tool Success and Failure Learning

**User Story:** As a user, I want meaningful outcomes to improve future recall while trivial chatter stays out of durable memory, so that KRIA learns signal rather than volume.

#### Acceptance Criteria

1. WHEN a native tool, MCP tool, OpenClaw skill, or sidecar task completes, THE Cognitive_Memory_System SHALL classify the outcome as success, partial success, expected failure, unexpected failure, timeout, cancellation, correction, undo, or unknown.
2. WHEN an outcome is meaningful, THE Write_Policy_Engine SHALL store tool/server identity, capability and version, invocation identifier, goal context, environment class, input fingerprint, result summary, error class, latency, timeout/retry/recovery facts, affected records, user correction, and source policy.
3. WHEN an outcome is a failure, THE Cognitive_Memory_System SHALL preserve policy-safe failure Evidence and recovery result unless content is secret or unsafe.
4. WHEN an outcome is a trivial repeated success, THE Write_Policy_Engine SHALL count the outcome in bounded telemetry and avoid durable memory creation.
5. WHEN comparable observations number at least 20, THE Cognitive_Memory_System MAY show versioned success rate and latency quantiles with sample size, environment class, and observation window; below 20 it SHALL display `Insufficient evidence` and SHALL NOT extrapolate reliability.
6. WHEN a task outcome can be attributed to retrieved memory, THE Cognitive_Memory_System SHALL divide credit across the Used_Item set and record the attribution method.
7. IF Memory Worth has fewer than 20 observations, THEN THE Retrieval_Engine SHALL prevent Memory Worth from changing archival or ranking decisions.
8. WHEN capability learning or Memory Worth changes ranking, THE Retrieval_Engine SHALL do so only through a named versioned policy and SHALL record the contribution in Retrieval_Trace.
9. THE Cognitive_Memory_System SHALL prevent tool/MCP learning from granting capabilities, expanding scope, bypassing approval, promoting a Rule, mutating security policy, or deleting memory.
10. THE Cognitive_Memory_System SHALL prevent a learned outcome from overriding explicit user correction or a newer capability-version observation without preserving both and applying declared precedence.

**Traceability:** Memory Requirement 16, D-19, CP-13; hardening M5 and AUD-02.

### Requirement 45: MGR-045 — Offline Degradation and Resource-Aware Cognition

**User Story:** As a laptop user, I want memory to remain useful offline and respectful of resources, so that cognition does not compete with foreground work.

#### Acceptance Criteria

1. WHILE network services are unavailable, THE Cognitive_Memory_System SHALL preserve local Authority_Store writes, FTS5 retrieval, available vector retrieval, lifecycle controls, and policy enforcement.
2. WHILE the embedder is unavailable, THE Retrieval_Engine SHALL use FTS5 plus available graph, temporal, and goal strategies and queue bounded embedding work.
3. WHILE an LLM is unavailable, THE Cognitive_Memory_System SHALL use deterministic admission and extraction, queue model-dependent consolidation, and keep storage and retrieval functional.
4. WHILE battery or power-saver mode is active, THE Cognitive_Scheduler SHALL suspend P3 and P4 work and preserve P0 and required P1 work.
5. WHILE memory pressure is high, THE Cognitive_Scheduler SHALL shed rebuildable caches, reduce concurrency, and defer P3 and P4 work.
6. WHILE thermal, CPU, GPU, or local-model pressure exceeds configured thresholds, THE Cognitive_Scheduler SHALL pause or chunk nonessential work before affecting foreground recall or correction.
7. WHEN a burst exceeds the bounded enrichment wake queue, THE Cognitive_Memory_System SHALL drop or coalesce wakes while preserving durable Event_Log work and recovery cursors.
8. WHEN offline or pressure state changes, THE Memory_Control_Center SHALL show exact degradation, queued work, preserved capabilities, and recovery state.
9. WHEN resource tests run, THE Cognitive_Memory_System SHALL prove bounded queue memory, foreground preemption, no P3 or P4 execution on battery, and eventual catch-up after pressure clears.

**Traceability:** Memory laws L8, L12; memory Requirement 7 and 20; hardening R1, R2, L3, L4; AGENTS resource policy.

### Requirement 46: MGR-046 — Consent-Gated Ingestion and Source Lifecycle

**User Story:** As a user, I want source ingestion explicit, resumable, and deletable, so that documents and scans cannot silently become permanent memory.

#### Acceptance Criteria

1. WHEN KRIA first offers filesystem, repository, shell-history, or library scanning, THE Memory_Control_Center SHALL request source-specific consent before scanning.
2. IF scan consent is not granted, THEN THE Cognitive_Memory_System SHALL perform no scan and offer manual onboarding instead.
3. WHEN a consented scan produces candidates, THE Memory_Control_Center SHALL let the user preview, exclude, and approve candidates before durable admission.
4. WHEN a document is ingested, THE Cognitive_Memory_System SHALL stream content, create bounded chunks, compute a content hash, preserve item and version identity, and submit each derived write through the Write_Policy_Engine.
5. WHEN duplicate content is ingested, THE Cognitive_Memory_System SHALL reuse or version the existing source record according to the declared content and source identity policy.
6. IF ingestion is cancelled or interrupted, THEN THE Cognitive_Memory_System SHALL stop within the current bounded unit and preserve a resumable cursor without partial semantic records.
7. WHEN a source item is deleted, THE Cognitive_Memory_System SHALL apply the lifecycle preview and cascade semantics from MGR-040.
8. WHEN imported or library content contains secret or injection-shaped data, THE Write_Policy_Engine SHALL apply content-level sensitivity and injection checks before creating retrievable memory.

**Traceability:** Memory Requirement 8 and 9, CP-10; hardening M3, L4, S1, S2, M6.

### Requirement 47: MGR-047 — Open-Source Licensing and SBOM

**User Story:** As a product owner, I want dependency provenance and licenses known before release, so that the Memory system can ship and evolve without hidden legal or supply-chain risk.

#### Acceptance Criteria

1. THE build system SHALL pin direct Rust, TypeScript, Python, model, and optional renderer dependencies through committed lock or checksum records.
2. THE build system SHALL generate a machine-readable SPDX or CycloneDX SBOM covering application, sidecar, model runtime, assets, and optional renderer dependencies for every release candidate.
3. THE build system SHALL maintain an approved license policy that permits only reviewed free and open-source licenses for shipped Memory Control Center and optional 3D code.
4. IF a dependency has an unknown, source-available-only, noncommercial, field-of-use, incompatible copyleft, or missing license, THEN THE release system SHALL block inclusion until explicit legal disposition.
5. WHEN an optional 3D dependency is proposed, THE build system SHALL record package purpose, transitive licenses, maintenance status, bundle cost, security advisories, and deletion ownership before implementation.
6. WHEN dependency vulnerability or license scans run, THE build system SHALL produce Evidence_Artifacts linked to exact lockfiles and approved exceptions.
7. IF a failed optional capability leaves a dependency used only by that capability, THEN THE codebase SHALL remove the dependency, lockfile entries, assets, tests, and SBOM component.
8. THE documentation set SHALL distinguish KRIA’s MIT project license from the independent licenses of dependencies, models, fonts, icons, and assets.
9. THE current implementation SHALL keep authoritative domain and retrieval logic in Rust, desktop transport in Tauri v2, and Memory_Control_Center presentation in SolidJS and TypeScript; Python sidecars SHALL remain optional and SHALL NOT become required authority paths.
10. WHEN a 2D or optional 3D renderer is selected or replaced, THE architecture review SHALL compare semantic/action parity, accessibility, Linux WebKitGTK behavior, bundle size, CPU/GPU/RAM/battery measurements, maintenance health, and license/SBOM evidence; existing dormant code alone SHALL NOT justify selection.
11. EVERY shipped memory model, runtime, asset, font, icon, Rust crate, JavaScript package, and Python package SHALL have an exact lock or checksum and a reviewed free/open-source license disposition.

**Traceability:** Root MIT declaration and lockfiles; current repository SBOM gap; MGD-016.

### Requirement 48: MGR-048 — Backend-First Release Order and Evolution Discipline

**User Story:** As an implementation owner, I want foundation gates to precede polish, so that a premium interface cannot conceal incomplete memory behavior.

#### Acceptance Criteria

1. THE release process SHALL complete F0 evidence reset before accepting implementation-completion claims.
2. THE release process SHALL complete F1 authority, write-policy, security, isolation, lifecycle, and fault gates before F4 visual-polish acceptance.
3. THE release process SHALL complete F2 semantic model, provenance, relation, entity, and temporal gates before enabling correction controls.
4. THE release process SHALL complete F3 five-strategy retrieval, Retrieval_Trace, truth maintenance, goals, consolidation, and resource gates before declaring the Control Center production-ready.
5. IF a backend capability is incomplete, THEN THE Memory_Control_Center SHALL expose an honest unavailable or partial state rather than a simulated control or fixture.
6. WHEN F4 begins, THE implementation process SHALL preserve a complete list-first or minimal 2D workflow while visual quality evolves.
7. WHEN F5 release evidence is reviewed, THE release process SHALL require zero open Critical or High privacy, security, truth, lifecycle, accessibility, or data-integrity findings.
8. WHERE a future seam is reserved, THE implementation process SHALL add only the stable versioned boundary required by current behavior and avoid distributed or multi-user machinery without measured need.
9. WHEN a hard cutover succeeds, THE codebase SHALL delete superseded stores, adapters, renderers, migrations, tests, and claims that no longer serve current behavior.
10. WHERE optional 3D is attempted after F5, THE release process SHALL treat F6 as independent from public readiness and enforce the clean ship-or-delete outcome.

**Traceability:** MGD-001, MGD-002, MGD-015, MGD-016, MGD-019–MGD-022; dev-context and audit launch verdict.

## Normative Production Gates

| Gate | Required threshold |
|---|---|
| Authority atomicity | Zero partial authority/event/audit/outbox/revision commits under crash injection |
| Event immutability | 100% UPDATE and DELETE attempts rejected by database enforcement |
| Write governance | Zero durable write paths bypassing Write_Policy_Engine |
| Privacy isolation | Zero unauthorized content, identifier, count, topology, timing, cache, trace, or log leaks |
| Core retrieval | p95 ≤120 ms at 100,000 records on Reference_Hardware after declared warm-up |
| Control Center search | p95 ≤250 ms at 100,000 records on Reference_Hardware |
| One-hop graph | p95 ≤500 ms at 100,000 records on Reference_Hardware |
| Link prediction | p95 ≤750 ms at 100,000 records on Reference_Hardware |
| Async runtime | Zero graph or memory executor blocking spans >50 ms |
| Foreground preemption | Lower-priority cognition yields or defers within 100 ms |
| Interaction frame | p95 ≤33.3 ms at the declared 2D scene cap |
| Idle rendering | No graph-originated continuous render or animation loop after 2 seconds |
| Idle CPU | ≤2 percentage points above blank Memory view over 60 seconds |
| Accessibility | WCAG 2.2 AA plus complete keyboard and Orca task script |
| Lifecycle | Zero deleted content returned after hard-delete reconciliation |
| Retrieval quality | On ≥200 judged queries: Recall@10 ≥0.85, nDCG@10 ≥0.80, exact phrase/identifier ≥0.95, forbidden exclusion 100%, superseded/deleted exclusion 100%, and no >0.03 absolute regression outside 95% bootstrap CI |
| Graph traversal | 100% max-three-hop cyclic fixtures terminate, repeat no path node, respect bounds, and leak no hidden intermediary/topology |
| Derived rebuild | FTS5/vector/analytics rebuild reproduces authorized membership, model/version semantics, and deleted-record exclusion |
| Interchange | Deterministic export/import round trip preserves semantic IDs, order, Memory_Links, provenance, truth/lifecycle state, and checksums |
| Corruption | Authority corruption enters read-only Recovery_Mode and fails closed; derived corruption rebuilds without authority mutation |
| Optional 3D | ≥10% median task benefit, ≥30 FPS real scene, no task or accessibility regression, idle quiet |
| Release findings | Zero open Critical or High privacy, security, truth, lifecycle, accessibility, or integrity findings |

## Normative Correctness Obligations

These obligations are requirements-phase inputs for later design property mapping; this update does not modify `design.md`.

1. **Authority atomicity:** For any accepted write command, authority records, Event, Audit_Record, outbox work, and Graph_Revision either all commit once or all remain unchanged.
2. **Event immutability:** For any committed Event, later supported operations preserve identifier, payload, checksum, and ordering fields.
3. **Idempotent admission:** For any command replayed with the same idempotency key and caller context, the semantic authority result is identical and unique.
4. **Restrictive policy propagation:** For any derived record, Effective_Policy is at least as restrictive as every contributing record.
5. **Isolation non-interference:** For any two callers with disjoint authorization, adding hidden records does not change the first caller’s visible identifiers, labels, authorized counts, result membership, or error shape.
6. **Serialization round trip:** For any supported Cognitive_Record or API DTO, serialize then deserialize preserves semantically significant data.
7. **Relationship normalization:** For any symmetric relation, swapping endpoints preserves identity; for any directed relation, swapping endpoints preserves identity only when the Relation_Registry defines equivalence.
8. **Evidence aggregation:** For any repeated observation of one active relation identity, evidence cardinality may increase while semantic edge cardinality remains one.
9. **Current-validity safety:** For any current query instant, no returned relationship has a closed Valid_Time ending at or before the instant.
10. **Snapshot pagination:** For any fixed authorized query and Graph_Revision, concatenating all cursor pages equals the bounded snapshot result without duplicates.
11. **Patch convergence:** For any valid patch sequence with duplicates or reordered delivery, apply-or-refetch converges to the same active-query projection as authority.
12. **Retrieval trace soundness:** For any response, every item labeled Used belongs to the recorded context-injected set.
13. **Hybrid degradation monotonicity:** For any unavailable optional retrieval strategy, remaining strategies return only policy-authorized candidates and the trace names the degradation.
14. **Truth supersession:** For any supersession, the successor is current, the predecessor remains historically inspectable, and default current retrieval excludes the predecessor.
15. **Consolidation provenance:** For any Summary, Skill, or Rule, traversing `derived_from` reaches at least one non-self-reflection source record and preserves every immediate parent.
16. **Consolidation idempotence:** For any unchanged consolidation input set and algorithm version, repeated consolidation produces no duplicate semantic output.
17. **Entity merge reversibility:** For any accepted reversible merge, applying the compensating split restores source canonical memberships and preserves audit history.
18. **Deletion exclusion:** For any hard-deleted scope after reconciliation, vector, FTS5, graph, trace, inspector, cache, and export queries return no deleted content.
19. **Crypto-shred denial:** For any destroyed subject key, every supported decryption path fails closed and yields no plaintext.
20. **Focus generation safety:** For any stale focus response, applying the response leaves the current lens state unchanged.
21. **Renderer parity:** For any Semantic_Scene action required by a core task, representation choice does not change domain authorization or outcome.
22. **Navigation exclusion:** For any topology, path, centrality, or community calculation, Navigation_Groups contribute no authority edges.
23. **Resource boundedness:** For any input burst above wake-channel capacity, durable Event work remains recoverable while in-memory queue usage stays within its configured bound.
24. **Tool-learning safety:** For any tool outcome, Memory Worth cannot directly cause hard deletion, core-namespace promotion, or Rule promotion.

## Required Verification and Evidence Artifacts

| Evidence class | Minimum artifact |
|---|---|
| Domain and property | Seeded property-test report with property annotation and minimized counterexample on failure |
| Schema and migration | Fresh-create, hard-cutover, invariant, corruption, and clean-rebuild report |
| Contract | Canonical schema fixtures plus normalized Tauri/server comparison |
| Retrieval | Versioned evaluation corpus, relevance judgments, Recall/nDCG/exact-match, latency, and trace-soundness report |
| Memory lifecycle | Forget, restore, hard-delete, source cascade, index purge, reconciliation, and crypto-denial report |
| E2E | Find, trace, inspect, correct, merge, split, relate, contradict, supersede, goal, forget, restore, delete, offline, and recovery runs |
| Regression | Audit finding map and old-bug reproduction coverage |
| Visual | Deterministic screenshots plus semantic review notes for required viewport/theme/state matrix |
| Accessibility | Automated checks plus keyboard and Orca task transcript |
| Performance | Same-hardware p50/p95/p99, CPU, GPU where available, RAM, heap, frame, idle, queue, and query-plan data |
| Security | Threat matrix, auth tests, namespace non-interference, poisoned-content, log-redaction, and deletion-leak results |
| Fault | Crash points, database busy, worker failure, model loss, patch disorder, cursor expiry, scope change, and context-loss results |
| Supply chain | Exact-lockfile SBOM, license report, vulnerability report, and approved exception record |
| Release | Requirement-to-test-to-artifact manifest with commit and environment metadata |

## Audit Reconciliation

- **Preserved binding decisions:** MGD-001–MGD-022 remain effective.
- **Preserved graph requirements:** MGR-001–MGR-032 retain identifiers and expanded scope; MGR-033–MGR-048 add missing whole-system obligations.
- **Preserved findings:** MG-C01–MG-C07, MG-H01–MG-H17, MG-M01–MG-M28, and MG-L01–MG-L13 remain Planned or Unverified until Evidence_Artifacts prove closure.
- **Preserved opportunities:** MG-O01–MG-O31 remain mapped to required or explicitly conditional outcomes.
- **Preserved memory foundations:** L1–L12, ADR-004/005/006/008/009/013, D-5/D-12/D-16/D-17/D-19/D-20, CP references, and hardening IDs retain their source meaning.
- **Corrected overclaims:** Current vector+FTS retrieval is a baseline, not proof of five-strategy retrieval; current shred-key status is not proof of cryptographic unreadability; existing Memory UI is not proof of the specified Control Center; dormant 3D is not a capability; old checkboxes are not completion evidence.

## Dependencies

| Dependency | Required disposition |
|---|---|
| SQLite with bundled FTS5 and integrity support | Sole authority plus exact lexical projection; schema and pragma contract pinned and tested. |
| FastEmbed `all-MiniLM-L6-v2` | Local 384-dimensional embedding model with source, license, checksum, tokenizer/runtime, and normalization pinned. |
| Rust / Tauri v2 / SolidJS contracts | Rust owns domain logic, Tauri adapters remain thin, SolidJS consumes canonical runtime-validated DTOs. |
| Local OS facilities | File permissions, keychain where used, disk encryption disclosure, power/pressure signals, and monotonic/UTC clocks have typed unavailable behavior. |
| Deterministic evidence tooling | Seeded fixtures, judged retrieval corpus, Playwright captures, Orca scripts, property tests, corruption injectors, benchmark manifests, SBOM/license scanners. |

## Principal Risks and Required Gates

| Risk | Governing requirements and blocking evidence |
|---|---|
| Split authority or bypass writes | MGR-005, MGR-033–035; direct-write search plus atomicity/idempotency crash suite. |
| Scope, sensitivity, count, timing, or cache leakage | MGR-003, MGR-004, MGR-043; zero-leak non-interference matrix. |
| False epistemic or Digital Twin claims | MGR-001, MGR-011, MGR-025, MGR-026, MGR-031; claim inventory and semantic visual review. |
| Incorrect person merge | MGR-019; name-only auto-merge rejection and merge/split round trip. |
| Retrieval quality regression or model drift | MGR-006, MGR-036, MGR-042; judged-corpus quality, model manifest, and rebuild equivalence. |
| Authority corruption or deletion residue | MGR-017, MGR-040–042; byte-damage Recovery_Mode and zero-residue reconciliation. |
| Resource contention and battery drain | MGR-009, MGR-022, MGR-045; scheduler preemption, bounded queues, frame/idle/pressure profiles. |
| Schema/model obsolescence | MGR-032, MGR-034, MGR-042; all-released-version migration fixtures and interchange round trip. |
| Renderer divergence or inaccessible spectacle | MGR-012–016, MGR-026, MGR-030; scene/action parity, visual matrix, Orca, and ship-or-delete gate. |
| Supply-chain or license incompatibility | MGR-047; exact locks/checksums, SBOM, vulnerability and license reports. |

## Repository Research Basis

The requirements were reconciled against the current schema and memory implementation (`crates/kria-core/src/memory/`), canonical contract, retrieval, truth maintenance, lifecycle, entity and graph code, OpenClaw capability model, current Memory Space, root dependency declarations and lockfiles, `MEMORY_ARCHITECTURE_FINAL.md`, `.kiro/specs/memory-upgrade/requirements.md`, `.kiro/specs/memory-upgrade/STABILIZATION_BIBLE.md`, the graph audit, and the existing decision, architecture, design, validation, risk, traceability, and task artifacts. Missing implementation evidence is expressed as a requirement or release gate rather than a research TODO.

## Complete Coverage and Definition of Ready

### Coverage Map

| Concern | Governing requirements |
|---|---|
| Authority, events, records, policy, provenance | MGR-033–MGR-035 |
| Search, retrieval, trace, goals, indexes | MGR-006, MGR-025, MGR-036, MGR-038, MGR-042 |
| Graph, relations, entities, time, truth | MGR-002, MGR-005, MGR-007, MGR-010, MGR-018, MGR-019, MGR-037, MGR-039 |
| Lifecycle, privacy, isolation, consent | MGR-003, MGR-004, MGR-040, MGR-041, MGR-043, MGR-046 |
| Resilience, evolution, recovery, resources | MGR-009, MGR-017, MGR-032, MGR-042, MGR-045 |
| Native/MCP/OpenClaw/tool learning | MGR-043, MGR-044 |
| UI, accessible Digital Twin, 2D/3D | MGR-012–MGR-016, MGR-021–MGR-024, MGR-026, MGR-030, MGR-031 |
| Supply chain and open-source posture | MGR-047 |
| Evidence, documentation, sequencing, release | MGR-027–MGR-029, MGR-048 |

### Requirements Ready for Design

The requirements phase is ready only when:

- every canonical term used by acceptance criteria is defined;
- every MGR requirement has priority, gate, dependencies, risks, and a named verification/evidence class;
- no `TBD`, placeholder, fake fixture behavior, or subjective term such as “futuristic,” “intelligent,” “fast,” or “high quality” remains without measurable semantics;
- fixture seeds, the ≥200-query judged retrieval corpus plan, pinned model assets/checksums, and reference-hardware manifest are identified;
- thresholds, degraded behavior, failure behavior, and rollback boundaries are fixed;
- a requirement → test → Evidence_Artifact manifest template exists;
- unchecked, checked, or partial task markers are explicitly treated as planning state, never proof.

### Public-Ready Definition

Public readiness requires all F0–F5 P0 and P1 obligations to have linked executable and manual evidence, zero open Critical or High truth/privacy/security/lifecycle/accessibility/integrity findings, complete authoritative 2D and semantic list action parity, deterministic rebuild/recovery/deletion evidence, retrieval-quality and resource gates, and documentation/runtime parity. F6 optional 3D is not a dependency. The only acceptable F6 outcomes are an evidence-qualified optional renderer using the same Digital_Twin scene/actions or complete removal of its code, controls, dependencies, and shipping claims.