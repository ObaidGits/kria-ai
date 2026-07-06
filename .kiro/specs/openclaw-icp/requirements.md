# Requirements Document

## Introduction

This document specifies the requirements for the **OpenClaw Intelligent Capability Platform (ICP)**. The
requirements are **derived from the approved design** (`design.md`) and formalize what that design specifies,
expressed in EARS-compliant, testable acceptance criteria.

OpenClaw ICP transforms OpenClaw from a tool-execution layer into an Intelligent Capability Platform. Once the
user has manually selected OpenClaw Tool Mode, the system reasons in **goals, not skill names**: it understands
the goal, discovers capabilities across installed, marketplace, and generated sources, ranks them, acquires
what is missing, plans multi-capability compositions as a capability graph, generates schema-valid arguments,
applies an intelligent tiered permission model, executes through the frozen runtime, verifies, responds, and
learns.

The design introduces exactly one new conceptual subsystem — the **Capability Intelligence Layer (CIL)** — plus
thin, additive extensions to the frozen A0–A9 components. Every requirement below honors these non-negotiable
constraints from the design and the product brief:

- **Auto Routing Mode is out of scope.** All ICP behavior begins **after** OpenClaw mode is selected and is
  active only inside the `openclaw` tool handler path.
- **A0–A9 is frozen.** Every capability extends a named frozen component (RuntimeManager, ExecutionEngine,
  SemanticSkillRouter, ProductionSkillRegistry, Marketplace/ClawHub, ContainerPool, DockerRuntime, A9
  GenerationPipeline, Planner, Verifier, MCP bridge, BundleInstaller, ApprovalCache). No component is
  redesigned or duplicated.
- **No hardcoding.** No hardcoded prompts, skill names, capability→skill maps, routing tables, or per-category
  branches. All behavior derives from skill metadata, JSON schemas, capability descriptors, embeddings, and
  registry state.
- **Scale target.** Correct and performant at 10,000+ skills, hundreds of categories, multiple marketplaces,
  generated skills, and enterprise/cloud/distributed execution.
- **Reversible rollout.** All new behavior is gated behind the `openclaw_icp_enabled` flag; flag-off yields
  byte-for-byte current behavior.

The acceptance criteria are written to map 1:1 to the design's **Correctness Properties** and **Testing
Strategy**. The requirement numbering is fixed so that each design property resolves to the requirement ID it
forward-references (e.g. Property 8 → 2.1, Property 3 → 6.1).

## Glossary

- **ICP (Intelligent Capability Platform):** The overall feature specified by this document; the goal-centric
  capability system active inside the OpenClaw tool path.
- **CIL (Capability Intelligence Layer):** The single new orchestration/intelligence subsystem
  (`openclaw::cil`) that sits between the frozen handler and the frozen router/engine/registry; it discovers,
  ranks, acquires, plans, recommends, and learns.
- **CapabilityTag:** A namespaced, open-vocabulary string (reverse-DNS style, e.g. `media.image.ocr`)
  describing a semantic capability a skill **provides** or a goal **requires**. Not a closed enum.
- **Capability profile:** The derived view (`CapabilityProfile`) of a skill's `provides`/`consumes`/`inputs`/
  `outputs`/`permissions`, extracted from `SkillMetadata`.
- **Capability (permission):** The frozen `capability::Capability` (kind/mode/scope) that governs runtime
  access; distinct from a semantic `CapabilityTag`.
- **Capability graph:** A DAG of skills/capabilities capturing dependency, alternative, provides-for, and
  supersedes edges; used for planning, dependency resolution, and recommendations.
- **Frozen component:** Any A0–A9 symbol enumerated in the design's frozen-component map; extended, never
  redesigned or duplicated.
- **Derived view:** A rebuildable index or table keyed by `skill_id` that is never an authoritative store; it
  is always reconstructable from `ProductionSkillRegistry` plus marketplace fetch.
- **Degraded mode:** The honest fallback state entered when the embedder or network is unavailable; discovery
  falls back to the frozen BM25 index plus `SemanticSkillRouter`.
- **Goal intent:** The parsed, embedded representation (`GoalIntent`) of a user goal, carrying the required
  `CapabilityTag`s with confidences.
- **Unified installer:** The frozen `BundleInstaller`; the single install path onto which both marketplace and
  generated skills converge.
- **Registry:** `ProductionSkillRegistry` (`skills.db`), the sole source of truth for skills.
- **PermissionEngine / GrantStore:** The tiered permission decision engine and its durable, scoped, revocable
  grant persistence, extending the frozen `ApprovalCache`.
- **Feature flag:** `openclaw_icp_enabled`, the configuration flag gating all ICP behavior.

## Requirements

### Requirement 1: No-Hardcoding and Generic Capability Abstractions

**User Story:** As a platform maintainer, I want all capability behavior driven by metadata and open
vocabularies rather than hardcoded names or categories, so that the platform scales to 10,000+ skills and
unknown future domains without code changes.

#### Acceptance Criteria

1. WHERE a `CapabilityTag` has never been encountered before, THE Capability_Intelligence_Layer SHALL perform
   discovery, ranking, planning, and permission classification for that tag without any code change and without
   any branch that enumerates specific capabilities. *(no-hardcoding / generic abstractions — scale-safe)*
2. THE Capability_Intelligence_Layer SHALL represent every capability domain as an open, namespaced
   `CapabilityTag` string supplied by skill metadata, and SHALL NOT define a closed enumeration of capability
   categories.
3. WHEN the Capability_Intelligence_Layer parses a user goal, THE Capability_Intelligence_Layer SHALL derive
   required capabilities using the configured embedder and one structured LLM call, without keyword tables or
   per-category rules.
4. THE Capability_Intelligence_Layer SHALL derive ranking, compatibility, permission, and recommendation
   decisions from skill metadata, JSON schemas, capability descriptors, embeddings, and registry state, and
   SHALL NOT reference any skill by literal name.
5. WHERE a new capability domain is introduced by a skill publishing a new `CapabilityTag`, THE
   Capability_Intelligence_Layer SHALL index, rank, plan, and permission-classify that skill through the same
   code paths used for existing skills.

### Requirement 2: Intelligent Capability Acquisition

**User Story:** As a user, I want the system to acquire a missing capability automatically by installing from a
marketplace or generating a skill, so that my goal can be fulfilled even when no installed skill matches.

#### Acceptance Criteria

1. WHEN a skill is acquired from a marketplace OR generated by the A9 GenerationPipeline, THE
   Acquisition_Orchestrator SHALL register that skill through the frozen `BundleInstaller` so that the acquired
   skill is structurally identical to an authored skill and its provenance is recorded as metadata only.
   *(unified installer convergence)*
2. WHEN an acquisition targets a skill whose publisher is revoked in the `PublisherRegistry`, THE
   Acquisition_Orchestrator SHALL return `Declined` and SHALL NOT install the skill. *(trust enforced before
   install)*
3. WHEN a required capability is missing or ranks below the configured trust and compatibility thresholds, THE
   Acquisition_Orchestrator SHALL evaluate the best marketplace candidate first and, only if no acceptable
   candidate exists and generation is allowed, fall back to A9 generation.
4. WHEN an acquired skill declares dependencies, THE Acquisition_Orchestrator SHALL resolve them using the
   capability graph and `SkillMetadata.dependencies`, recursively acquiring missing dependencies within a
   bounded depth and rejecting dependency cycles.
5. IF the `BundleInstaller` verification, hash, or signature check fails during an acquisition, THEN THE
   Acquisition_Orchestrator SHALL abort the acquisition, register nothing, and return `Declined` with the
   failure reason.
6. IF no acceptable marketplace candidate exists and generation is disallowed or fails, THEN THE
   Acquisition_Orchestrator SHALL return `Declined` and SHALL NOT report a successful acquisition.

### Requirement 3: Multi-Capability Planning

**User Story:** As a user, I want the system to compose multiple capabilities into a single executable plan, so
that multi-step goals are fulfilled without me manually chaining skills.

#### Acceptance Criteria

1. WHEN the Capability_Planner produces an `ExecutionGraph`, THE Capability_Planner SHALL ensure that graph
   passes the frozen `DependencyResolver::validate` check (acyclic, all executors registered) before execution.
   *(valid capability graph)*
2. WHERE the Capability_Planner adds a composition edge from capability `a` to capability `b`, THE
   Capability_Planner SHALL require that the intersection of `a.outputs` and `b.inputs` is non-empty, composing
   by I/O type matching rather than by skill name. *(type-directed composition)*
3. THE Capability_Planner SHALL express every plan as the frozen `execution::ExecutionGraph` type using
   `NodeKind::Skill` nodes and frozen `Barrier`/`Merge`/`Wait` structural nodes, and SHALL NOT introduce a new
   plan format or modify the ExecutionEngine.
4. WHEN a plan requires fan-in or fan-out, THE Capability_Planner SHALL insert frozen structural nodes to
   coordinate dependencies rather than encoding any specific example workflow.
5. WHERE a plan would exceed the configured breadth or depth caps, THE Capability_Planner SHALL enforce those
   caps and reject or reduce the plan rather than emitting an unbounded graph.

### Requirement 4: Intelligent Capability Discovery (Goal → Execute → Learn)

**User Story:** As a user, I want the system to discover, execute, and learn from capabilities driven by my
goal, so that fulfillment improves over time while resources stay clean.

#### Acceptance Criteria

1. WHEN a run completes, fails, or is cancelled, THE Runtime_Manager SHALL return container and lease counts to
   their pre-run baseline, leaving no leaked containers or leases. *(resource cleanliness via frozen runtime)*
2. WHEN a user goal is received in OpenClaw mode, THE Capability_Intelligence_Layer SHALL discover installed
   candidates via the `CapabilityIndex` and marketplace candidates via the `MarketIndex` in parallel, and rank
   the combined set with the configured multi-signal ranker.
3. WHEN an execution node completes, THE Feedback_Learner SHALL update `SkillStatistics` (success rate, usage
   count, latency) by extending `SemanticSkillRouter::record_feedback`, and THE Capability_Ranker SHALL use
   those updated statistics as the popularity and success ranking signals on subsequent goals.
4. THE Capability_Intelligence_Layer SHALL hand a validated frozen `ExecutionGraph` to the frozen
   `ExecutionEngine` for execution and SHALL NOT interact with containers directly.
5. WHEN execution results are returned, THE SemanticOpenClawHandler SHALL wrap them as verified,
   evidence-wrapped output before responding to the user.

### Requirement 5: Capability Intelligence Layer

**User Story:** As a platform maintainer, I want a single intelligence layer that never becomes a competing
data store, so that the registry remains the sole source of truth and derived views are always rebuildable.

#### Acceptance Criteria

1. FOR ALL skills, THE Capability_Intelligence_Layer SHALL derive every query result purely from
   `ProductionSkillRegistry` plus marketplace fetch, such that rebuilding all derived views from the registry
   yields identical query results (idempotent reindex). *(registry is the sole source of truth)*
2. THE Capability_Intelligence_Layer SHALL persist all derived data (`capability_profiles`, `market_catalog`,
   `capability_edges`, scoped grants) using additive, forward-only migrations keyed by `skill_id`, and SHALL
   NOT drop or rename existing schema.
3. WHEN a derived-view drift or corruption is detected, THE Capability_Intelligence_Layer SHALL recover by
   performing a full rebuild from the registry rather than by manual repair.
4. WHEN the embedding model identifier changes, THE Capability_Intelligence_Layer SHALL invalidate affected
   cached embeddings and trigger a background reindex without downtime.
5. WHEN a new skill is acquired, THE Capability_Intelligence_Layer SHALL perform an incremental index upsert
   rather than a full reindex.

### Requirement 6: Permission System Redesign

**User Story:** As a user, I want intelligent, metadata-driven permission tiers, so that I am prompted only
when genuinely necessary while system-modifying actions always require explicit approval.

#### Acceptance Criteria

1. WHEN a permission decision is evaluated for a grant and its capability set, THE Permission_Engine SHALL
   ensure that narrowing the capability set (new is a subset of old) never converts an existing `Allow` into a
   `Prompt`, and WHEN the capability set is widened such that `requires_reapproval(old, new)` holds, THE
   Permission_Engine SHALL produce a `Prompt` or `Escalated` decision. *(tiered permissions with escalation on
   widening)*
2. IF a node has `classify_risk == Red` or requests a host-scope subprocess, THEN THE Permission_Engine SHALL
   assign the `AlwaysAsk` tier and prompt on every occurrence without remembering the decision, regardless of
   trust tier, unless an explicit `Silent` policy grant exists. *(deny-by-default for system modification)*
3. WHERE a skill has `classify_risk == Green` AND declares no filesystem, network, subprocess, or browser
   permission, THE Permission_Engine SHALL assign the `NeverAsk` tier and SHALL NOT produce any prompt.
   *(GREEN pure skills never ask)*
4. THE Permission_Engine SHALL derive each skill's permission tier from `classify_risk`, its
   `CapabilityProfile.permissions`, and its trust tier, and SHALL NOT assign tiers by matching skill names or
   categories.
5. WHEN a user or policy approves a capability set at a given scope, THE Permission_Engine SHALL persist the
   grant in the `GrantStore` with the correct scope (once, session, workspace, or persistent) and any expiry.
6. WHEN a user revokes a grant, THE Permission_Engine SHALL mark that grant revoked in the `GrantStore` and
   SHALL require fresh approval before the affected capability is next used.
7. WHEN a prior grant covers the requested scope and the capability set is not widened, THE Permission_Engine
   SHALL reuse that grant and allow the action without prompting.

### Requirement 7: Honesty and Backward Compatibility

**User Story:** As a user, I want the system to be honest about what actually happened and to preserve existing
behavior when disabled, so that I can trust its results and roll it back safely.

#### Acceptance Criteria

1. IF an operation did not actually occur (acquisition, planning, or execution), THEN THE
   Capability_Intelligence_Layer SHALL return `Declined`, `degraded`, or an error, SHALL NOT report a fake
   success, and SHALL emit an `AuditLedger` entry for every decision stage. *(no fake success, full telemetry)*
2. WHILE `openclaw_icp_enabled` is `false`, THE SemanticOpenClawHandler SHALL produce output byte-for-byte
   identical to the current direct-router path. *(flag-off parity)*
3. WHEN `openclaw_icp_enabled` is turned off after having been on, THE Capability_Intelligence_Layer SHALL
   restore prior behavior immediately and losslessly, and derived tables SHALL be safely droppable and
   rebuildable.
4. WHILE `openclaw_icp_enabled` is `true`, THE Permission_Engine SHALL behave as a strict superset of the
   frozen `ApprovalCache`, such that GREEN skills still auto-approve and widened capabilities still re-prompt.

### Requirement 8: Intelligent Recommendations (Phase D)

**User Story:** As a user, I want the system to recommend capabilities to install when I lack them, so that I
can decide whether to acquire what my goal needs.

#### Acceptance Criteria

1. WHEN a goal requires a capability the user does not have installed, THE Recommender SHALL return ranked
   candidates ordered by the configured signals (compatibility, popularity, quality, trust, dependencies,
   success).
2. THE Recommender SHALL produce recommendations as pure reads over the `MarketIndex` and capability graph and
   SHALL NOT install any skill without explicit user or policy approval.
3. THE Recommender SHALL assemble each recommendation rationale from real ranking signals and SHALL NOT use
   templated copy keyed to a specific skill name or category.
4. WHERE alternative or superseding skills exist in the capability graph, THE Recommender SHALL include them as
   alternatives in the recommendation.
5. IF no candidate exists above threshold, THEN THE Recommender SHALL return an empty recommendation set or an
   honest decline rather than fabricating a candidate.

### Requirement 9: Marketplace Federation

**User Story:** As an operator, I want the platform to index and search multiple marketplaces through a common
provider abstraction, so that enterprise and private repositories work without a second install path.

#### Acceptance Criteria

1. THE Market_Index SHALL discover marketplace catalogs through the `MarketplaceProvider` trait, wrapping the
   frozen `ClawHubClient` for ClawHub and allowing additional providers without modifying the frozen fetch
   path.
2. WHEN a marketplace catalog is synced, THE Market_Index SHALL embed catalog entries offline into the
   `market_catalog` cache and SHALL NOT perform live per-query marketplace fetches during discovery.
3. WHEN a marketplace provider returns a disallowed host or an oversized manifest, THE Market_Index SHALL
   reject the fetch via the frozen `DomainValidator` and return `Declined` with a reason.
4. THE Market_Index SHALL record version, deprecation, trust hint, quality, and popularity for each catalog
   entry so that discovery and recommendations are version- and deprecation-aware.
5. WHEN catalog synchronization runs, THE Market_Index SHALL perform incremental sync using ETag or
   `fetched_at` markers and process providers concurrently under a bounded work queue.

### Requirement 10: Frontend Evolution

**User Story:** As a user, I want OpenClaw UI surfaces for capabilities, permissions, logs, and the capability
graph, so that I can observe and manage the goal-centric platform.

#### Acceptance Criteria

1. THE Desktop_Surface SHALL add new Tauri commands and events for the capability manager, execution logs,
   developer mode, permission management, and capability-graph view, and SHALL preserve all existing OpenClaw
   Tauri command and event names.
2. WHEN a frozen `openclaw::event` or `RegistryEvent` is emitted, THE Desktop_Surface SHALL push the update to
   the UI, and THE UI SHALL reconcile missed events via polling for eventual consistency.
3. WHERE a feature is not production-ready, THE Desktop_Surface SHALL gate it behind Developer Mode.
4. THE Desktop_Surface SHALL display execution logs sourced from the `AuditLedger` and `openclaw::event`
   streams.
5. THE Desktop_Surface SHALL display capability dependencies, trust levels, generated-skill provenance, and
   available updates or deprecations sourced from their respective derived views.

### Requirement 11: Scale to 10,000+ Capabilities

**User Story:** As a platform maintainer, I want discovery, acquisition, planning, and permissions to remain
performant at 10,000+ skills, so that the platform survives real-world scale without special-case code.

#### Acceptance Criteria

1. WHEN searching installed skills, THE Capability_Index SHALL use approximate-nearest-neighbor dense retrieval
   fused with the frozen BM25 index rather than a linear scan.
2. THE Capability_Index SHALL expose its retrieval behind a trait boundary so an in-process index can be
   replaced by a distributed vector store without changes to callers.
3. WHEN a new skill is acquired at scale, THE Capability_Index SHALL apply an incremental upsert with bounded
   cost rather than a full reindex.
4. THE Grant_Store SHALL index scoped grants by `skill_id` and support partitioning by workspace so grant
   lookups remain performant as grant volume grows.
5. THE Capability_Planner SHALL enforce configurable breadth and depth caps so plan size remains bounded
   regardless of the number of available capabilities.

### Requirement 12: Skill-Category Compatibility via Generic Abstractions

**User Story:** As a skill author, I want cross-domain compatibility expressed through generic abstractions, so
that any capability domain composes without per-category code in OpenClaw.

#### Acceptance Criteria

1. THE Capability_Intelligence_Layer SHALL express all cross-domain compatibility through open `CapabilityTag`
   vocabulary, I/O type tags (`inputs`/`outputs`), and runtime requirements, and SHALL NOT contain per-category
   branches.
2. WHEN evaluating compatibility, THE Capability_Ranker SHALL match a skill's runtime requirements and resource
   class against `RuntimeManager` availability generically for every capability domain.
3. WHERE a skill declares I/O type tags, THE Capability_Planner SHALL determine composability by structural
   type matching of those tags rather than by skill category.
4. WHEN a brand-new capability domain is published, THE Capability_Intelligence_Layer SHALL support its
   embedding, indexing, ranking, planning, and permission classification with zero OpenClaw code changes.

### Requirement 13: Degraded Mode

**User Story:** As a user, I want the platform to keep working honestly when the embedder or network is
unavailable, so that I still get results and clear status instead of silent failure.

#### Acceptance Criteria

1. IF the embedding backend is unavailable or fails to load, THEN THE Capability_Intelligence_Layer SHALL enter
   degraded mode and fall back to the frozen BM25 index plus `SemanticSkillRouter` for discovery.
2. WHILE in degraded mode, THE Capability_Intelligence_Layer SHALL report the degraded state honestly in status
   and SHALL NOT present degraded results as full-fidelity results.
3. IF a marketplace provider is unreachable, THEN THE Market_Index SHALL serve the stale `market_catalog` cache
   and flag affected recommendations as offline.
4. WHEN the embedder or network becomes available again, THE Capability_Intelligence_Layer SHALL exit degraded
   mode and resume full discovery on the next boot or configuration change.
