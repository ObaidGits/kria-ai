# Requirements Document

## Introduction

This document specifies the **Capability Provider Platform (CPP)** — KRIA's provider-neutral
capability layer. It is the reference blueprint for how KRIA (the "Brain") discovers, describes,
recommends, acquires, executes, permissions, and learns from capabilities supplied by any number of
**capability providers** (the "Capability Platforms"), of which **OpenClaw is the first**.

The goal is a boundary that never requires architectural redesign as providers, protocols, marketplaces,
runtimes, and capability types evolve over the next 5–10 years. New providers arrive as **data**
(descriptors), **negotiated protocol features**, and **LLM-readable text** — never as KRIA-core code
changes.

This spec **does not redesign** the working, flag-gated Capability Intelligence Layer (CIL) or the frozen
OpenClaw A0–A9 execution/runtime/registry/generation components. It introduces the **anti-corruption
boundary** that turns OpenClaw into one provider among many, **slims the CIL** into a provider-neutral
descriptor reasoner, and **enriches the capability descriptor** so permission, planning, and discovery no
longer reach into any provider's internal types.

### Relationship to existing specs

- `openclaw-icp` (the CIL) — **built and wired**. CPP generalizes it: the CIL keeps its algorithms but is
  refactored to consume `CapabilityDescriptor`s through the `CapabilityProvider` trait instead of OpenClaw
  types. Flag-off parity with today is preserved.
- `openclaw-production-validation` — **validation harness**. CPP reuses `kria-eval::openclaw_eval` and
  extends it to a provider-neutral `capability_eval` suite.

### Non-negotiable constraints (apply to every requirement)

- **Brain/Provider separation.** KRIA-core business logic depends only on the CPP trait + descriptor +
  neutral value types. No provider-specific type (`SkillMetadata`, `LaunchSpec`, MCP framing, `ExecutorKind`
  variants) appears outside a provider's anti-corruption adapter.
- **No hardcoding.** No hardcoded provider names, capability names, category enums, capability→provider maps,
  or per-provider branches in KRIA-core. All behavior derives from descriptors, schemas, open-vocabulary
  tags, embeddings, and negotiated protocol features. Provider identity is an open string, never a closed
  enum.
- **Registry federation, single truth per provider.** Each provider owns its authoritative catalog; KRIA
  holds only a **derived, rebuildable** federated index keyed by `(provider_id, capability_id)`. KRIA never
  becomes a second source of truth.
- **MCP as the contract substrate.** CPP is a descriptor + negotiation profile **layered on MCP**, not a
  forked wire protocol. A plain MCP server is a valid provider with a default descriptor; rich providers add
  metadata and negotiated features.
- **Reversible rollout.** All CPP behavior is gated behind `capability_provider_platform_enabled`; flag-off
  yields byte-for-byte the current (CIL/OpenClaw) behavior.
- **Honesty.** No fake success, no silent provider bypass; every decision stage emits an audit record;
  degraded states (provider offline, embedder down, negotiation failed) are reported truthfully.
- **Scale target.** Correct and performant at 10,000+ capabilities across many providers, categories, and
  marketplaces.

## Glossary

- **Capability Provider:** A source of executable capabilities that implements the `CapabilityProvider`
  trait behind an anti-corruption adapter (OpenClaw, MCP servers, native tools, GUI cognition, browser,
  cloud, future kinds). Identified by an open-vocabulary `provider_id` string.
- **Capability Provider Protocol (CPP):** The versioned, self-describing, negotiated protocol — layered on
  MCP — by which KRIA and a provider agree on protocol version + features and exchange descriptors, discovery
  results, execution requests, effects, and lifecycle operations.
- **Capability Descriptor:** The rich, provider-neutral, LLM-readable, self-describing document for one
  capability (identity, semantics, I/O modality + type tags, triggers/examples, effects, permissions, trust,
  quality, and an open `extensions` map). Base version `v1`, extended additively to `v1.1` (guidance +
  expectations) by this spec; forward-only.
- **Anti-Corruption Layer (ACL):** The per-provider adapter — the ONLY place provider-native types exist —
  that translates between a provider's internal representation and the neutral CPP domain types.
- **Federated Capability Index:** The derived, rebuildable index (dense ANN + lexical BM25 fusion) over all
  providers' descriptors, keyed by `(provider_id, capability_id)`.
- **CIL (Capability Intelligence Layer):** The existing goal→discover→rank→acquire→plan→learn reasoner,
  refactored to operate solely on descriptors and the provider trait.
- **Effects:** The declared side-effect profile of a capability (read/write/network/subprocess/gpu/…,
  reversibility, idempotency, resource class) used for permission and planning without provider-specific
  knowledge.
- **Negotiation:** The handshake in which client (KRIA) and provider agree a protocol version and a feature
  set (e.g. streaming, lifecycle, acquisition, multi-modal I/O), so capabilities a provider lacks are simply
  absent, never errors.
- **Grant:** A durable, scoped, revocable permission decision persisted in the GrantStore.
- **Reference provider:** OpenClaw, which must prove every mandatory and negotiated protocol facet
  end-to-end.

## Requirements

### Requirement 1: Provider-Neutral Capability Boundary (Anti-Corruption Layer)

**User Story:** As a platform maintainer, I want KRIA's Brain to depend only on a provider-neutral boundary,
so that adding, upgrading, or replacing a provider never requires changes to KRIA-core business logic.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL define a single `CapabilityProvider` trait plus provider-neutral
   value types (`CapabilityDescriptor`, execution request/result, effects, error) that contain no
   provider-specific type.
2. WHERE KRIA-core discovers, ranks, plans, permissions, acquires, executes, or learns from a capability, THE
   Capability_Provider_Platform SHALL operate exclusively through the `CapabilityProvider` trait and the
   neutral value types, and SHALL NOT reference any provider-internal type outside that provider's adapter.
3. THE Capability_Provider_Platform SHALL identify each provider by an open-vocabulary `provider_id` string
   and SHALL NOT enumerate providers in any closed enum in KRIA-core (replacing the `ExecutorKind` enum at the
   execution seam with a provider-id).
4. WHEN a new provider adapter is registered, THE Capability_Provider_Platform SHALL make its capabilities
   discoverable, rankable, plannable, permission-classifiable, and executable through the same code paths used
   by existing providers, with no KRIA-core code change.
5. THE OpenClaw integration SHALL be expressed as a `CapabilityProvider` adapter (`OpenClawProvider`) that is
   the sole location where OpenClaw-internal types (`SkillMetadata`, `LaunchSpec`, `ProductionSkillRegistry`,
   MCP framing) are referenced, such that all current pre-existing imports of `kria_core::openclaw::*` from
   non-openclaw KRIA-core modules are removed or routed through the ACL.

### Requirement 2: Capability Provider Protocol (Versioned, Negotiated, MCP-Based)

**User Story:** As a provider author, I want a versioned, self-describing, negotiated protocol layered on MCP,
so that KRIA and my provider agree on exactly which features are supported without breaking when either side
evolves.

#### Acceptance Criteria

1. WHEN a provider is connected, THE Capability_Provider_Platform SHALL perform a negotiation handshake that
   exchanges a protocol version and a declared feature set, and SHALL agree the intersection of
   client-supported and provider-supported features before any capability is used.
2. WHERE a provider does not support an optional protocol facet (streaming, lifecycle/acquisition, multi-modal
   I/O, batch execution), THE Capability_Provider_Platform SHALL treat that facet as absent and SHALL NOT
   surface an error for its absence.
3. THE Capability_Provider_Platform SHALL layer descriptor exchange and negotiation on the existing MCP
   transport, such that a plain MCP server (advertising only `tools/list`/`tools/call`) is a valid provider
   consumed with a default-derived descriptor and the baseline feature set.
4. WHEN the negotiated protocol version differs between KRIA and a provider, THE Capability_Provider_Platform
   SHALL operate at the highest mutually supported version and SHALL record the negotiated version in
   telemetry.
5. IF negotiation fails or times out, THEN THE Capability_Provider_Platform SHALL mark the provider degraded,
   exclude it from discovery, report the reason honestly, and SHALL NOT crash or block other providers.
6. THE Capability_Provider_Platform SHALL carry unknown/forward-compatible negotiated features and descriptor
   fields through an open `extensions` map without rejecting them, so a newer provider can advertise features
   an older KRIA safely ignores.

### Requirement 3: Rich Capability Descriptor (v1.1)

**User Story:** As the Brain, I want a rich, self-describing, LLM-readable descriptor per capability, so that
I can discover, compose, permission, and explain any capability without knowing which provider supplied it.

#### Acceptance Criteria

1. THE Capability_Descriptor SHALL include: identity (`provider_id`, `capability_id`, `version`), semantics
   (name, description, open-vocabulary capability tags), I/O contract (input JSON Schema, output schema, I/O
   modality, and `inputs`/`outputs` type tags for composition), triggers (example prompts/intents for
   retrieval), effects (side-effect classes, reversibility, idempotency, resource class), permissions (neutral
   capability/effect set), trust (publisher, signature state, trust tier), quality/popularity (derived
   stats), and an open `extensions` map.
2. THE Capability_Descriptor SHALL express every capability domain as open, namespaced tag strings supplied by
   the provider, and SHALL NOT define a closed enumeration of capability categories or modalities in a way
   that blocks an unknown future value.
3. WHERE a provider supplies only baseline MCP metadata, THE Capability_Provider_Platform SHALL derive a
   valid `v1` descriptor with conservative defaults (effects = unknown/elevated, modality = text) so a thin
   provider is safely usable.
4. THE Capability_Descriptor SHALL be serializable to an LLM-readable form so recommendations, planning
   rationales, and explanations are assembled from descriptor content rather than provider-specific templates.
5. WHEN the descriptor schema evolves beyond `v1`, THE Capability_Provider_Platform SHALL accept older
   descriptors via forward-only, additive versioning and SHALL NOT require providers to re-emit descriptors to
   remain usable.
6. THE Capability_Descriptor for an OpenClaw skill SHALL be produced by the `OpenClawProvider` adapter from
   `SkillMetadata` + the substrate `tools/list` schema, with no loss of the information the CIL currently
   uses.

### Requirement 4: Federated Discovery and Retrieval at Scale

**User Story:** As a user, I want KRIA to find the right capability across all providers by goal, so that I
never have to name a provider or capability, even with tens of thousands installed.

#### Acceptance Criteria

1. THE Federated_Capability_Index SHALL index descriptors from all registered providers behind a trait
   boundary, keyed by `(provider_id, capability_id)`, fusing dense approximate-nearest-neighbor retrieval with
   lexical BM25 rather than a linear scan.
2. WHEN a user goal is received, THE Capability_Provider_Platform SHALL retrieve top-k candidate descriptors
   across providers and rank them with the configured multi-signal ranker (semantic, lexical, compatibility,
   trust, quality, popularity, success), and SHALL NOT prompt-dump full catalogs to the LLM.
3. THE Federated_Capability_Index SHALL be a derived view rebuildable from providers' catalogs, such that a
   full rebuild yields identical query results (idempotent reindex), and SHALL apply incremental upsert on
   single-capability change rather than a full rebuild.
4. THE Federated_Capability_Index SHALL expose retrieval behind a trait so an in-process index can be replaced
   by a distributed vector store without changes to callers.
5. IF the embedding backend is unavailable, THEN THE Capability_Provider_Platform SHALL enter degraded mode,
   fall back to lexical retrieval, and report the degraded state honestly.
6. WHEN a provider becomes unavailable, THE Capability_Provider_Platform SHALL serve that provider's last
   known descriptors from the derived index flagged as offline, and SHALL exclude its capabilities from
   execution until it recovers.

### Requirement 5: Provider-Neutral Planning and Composition

**User Story:** As a user, I want KRIA to compose capabilities across providers into one executable plan, so
that multi-step goals are fulfilled regardless of which provider each step comes from.

#### Acceptance Criteria

1. THE Capability_Planner SHALL compose capabilities by matching descriptor `inputs`/`outputs` type tags
   (`a.outputs ∩ b.inputs ≠ ∅`) rather than by provider or capability name, and MAY compose steps that span
   different providers.
2. THE Capability_Planner SHALL emit the frozen `execution::ExecutionGraph`, dispatching each node to its
   provider via a `provider_id`-addressed executor, and SHALL NOT introduce a new plan format or modify the
   ExecutionEngine's contract.
3. WHEN a plan is produced, THE Capability_Planner SHALL validate it via the frozen `DependencyResolver`
   (acyclic, all executors resolvable) before execution, and SHALL enforce configurable breadth/depth caps.
4. WHEN a plan node executes, THE Capability_Provider_Platform SHALL route execution through the provider's
   adapter and the frozen `ExecutionEngine`, and SHALL NOT touch any provider's runtime directly from the
   Brain.
5. WHERE a required capability spans a provider that lacks the streaming or batch facet, THE Capability_Planner
   SHALL degrade to the negotiated baseline execution for that node without failing the whole plan.

### Requirement 6: Provider-Neutral Permission and Approval Model

**User Story:** As a user, I want intelligent, descriptor-driven permissions with a real approval flow and
durable grants, so that I am prompted only when genuinely necessary and elevated actions always require
explicit, revocable approval — for any provider.

#### Acceptance Criteria

1. THE Permission_Engine SHALL derive each capability's permission tier from its descriptor `effects`,
   permission set, and trust tier, and SHALL NOT assign tiers by matching provider or capability names.
2. WHERE a descriptor declares no write/network/subprocess/gpu effect and classifies as low risk, THE
   Permission_Engine SHALL assign a never-prompt tier.
3. IF a descriptor declares a system-modifying effect (irreversible write, host-scope subprocess, or
   high/critical risk), THEN THE Permission_Engine SHALL require explicit approval on every use unless an
   explicit standing policy grant exists, regardless of trust tier.
4. WHEN a user or policy approves a capability at a scope (once/session/workspace/persistent), THE
   Permission_Engine SHALL persist the grant durably in the GrantStore with scope and expiry, and SHALL reuse
   it on subsequent matching requests without prompting.
5. WHEN a capability's effect/permission set widens beyond a prior grant, THE Permission_Engine SHALL require
   fresh approval; WHEN it narrows, THE Permission_Engine SHALL NOT convert an existing allow into a prompt.
6. WHEN a user revokes a grant, THE Permission_Engine SHALL mark it revoked and require fresh approval before
   the affected capability is next used.
7. THE Desktop_Surface SHALL present an approval flow that lets a user approve, scope, deny, and revoke grants
   for any provider, and SHALL surface the descriptor `effects` being approved.

### Requirement 7: Capability Acquisition and Lifecycle (Capability-Gated)

**User Story:** As a user, I want KRIA to acquire a missing capability from whichever provider can supply it,
so that my goal can be fulfilled even when nothing is installed — without provider-specific acquisition code
in the Brain.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL treat install/update/remove as **optional negotiated lifecycle
   facets** of the protocol, and SHALL only offer acquisition for a provider that advertises the lifecycle
   feature.
2. WHEN a required capability is missing, THE Acquisition_Orchestrator SHALL request acquisition from the
   best-ranked capable provider through the provider trait, and SHALL NOT contain provider-specific install
   logic in the Brain.
3. WHEN acquisition succeeds, THE Capability_Provider_Platform SHALL incrementally upsert the new descriptor
   into the federated index, and the acquired capability SHALL be structurally indistinguishable from a
   pre-existing one except for provenance metadata.
4. IF acquisition is disallowed by trust/policy/budget, fails verification, or no capable provider exists,
   THEN THE Acquisition_Orchestrator SHALL return an honest decline, acquire nothing, and emit an audit
   record, and SHALL NOT report a fake success.
5. WHERE a provider advertises no lifecycle facet, THE Capability_Provider_Platform SHALL still discover,
   rank, plan, permission, and execute that provider's existing capabilities normally.
6. THE OpenClaw acquisition (marketplace install via the frozen `BundleInstaller` and A9 generation) SHALL be
   exposed through the `OpenClawProvider` lifecycle facet, converging on the existing unified installer with
   no second install path.

### Requirement 8: Recommendations Across Providers

**User Story:** As a user, I want KRIA to recommend capabilities I could acquire, drawn from any provider's
catalog, so that I can decide what to add to fulfill my goal.

#### Acceptance Criteria

1. WHEN a goal needs a capability the user lacks, THE Recommender SHALL return ranked candidate descriptors
   across all providers' catalogs, ordered by the configured signals.
2. THE Recommender SHALL assemble each rationale from real descriptor content and ranking signals, and SHALL
   NOT use templated copy keyed to a provider or capability name.
3. THE Recommender SHALL perform recommendations as pure reads over the federated index and SHALL NOT acquire
   anything without explicit user or policy approval.
4. IF no candidate exists above threshold, THEN THE Recommender SHALL return an empty set or honest decline
   rather than fabricating a candidate.

### Requirement 9: Marketplace Federation

**User Story:** As an operator, I want capability catalogs from multiple marketplaces and providers indexed
through a common abstraction, so that enterprise, community, and private sources work without provider- or
marketplace-specific code.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL discover catalogs through a provider/marketplace abstraction and
   SHALL support additional sources without modifying frozen fetch paths.
2. WHEN a catalog is synced, THE Capability_Provider_Platform SHALL embed entries offline into the derived
   index and SHALL NOT perform live per-query marketplace fetches during discovery.
3. THE Capability_Provider_Platform SHALL perform catalog sync at boot and on demand, incrementally
   (ETag/timestamp), under a bounded concurrent work queue, and SHALL record version, deprecation, trust
   hint, quality, and popularity per entry.
4. WHEN a marketplace or provider is unreachable, THE Capability_Provider_Platform SHALL serve the stale
   cache flagged offline and SHALL reject disallowed hosts/oversized manifests via the frozen validator.

### Requirement 10: Learning Loop

**User Story:** As a user, I want capability selection to improve with use across all providers, so that KRIA
gets better at fulfilling my goals over time.

#### Acceptance Criteria

1. WHEN a capability execution completes, fails, or is cancelled, THE Feedback_Learner SHALL update per-
   capability statistics (success rate, usage count, latency) keyed by `(provider_id, capability_id)`.
2. THE Capability_Ranker SHALL use updated statistics as popularity/success signals on subsequent goals.
3. THE Feedback_Learner SHALL attribute outcomes to the correct provider+capability and SHALL NOT leak
   statistics across providers.

### Requirement 11: Observability, Honesty, and Backward Compatibility

**User Story:** As a user, I want CPP to be observable and honest, and to preserve existing behavior when
disabled, so that I can trust results and roll back safely.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL emit a correlated telemetry/audit record for each stage
   (negotiation, discovery, ranking, permission, acquisition, planning, execution, learning, failure,
   cancellation), each tagged with `provider_id` and a correlation id.
2. IF an operation did not actually occur, THEN THE Capability_Provider_Platform SHALL return decline,
   degraded, or error, and SHALL NOT report a fake success.
3. WHILE `capability_provider_platform_enabled` is false, THE Capability_Provider_Platform SHALL produce
   behavior byte-for-byte identical to the current CIL/OpenClaw path.
4. WHEN the flag is turned off after being on, THE Capability_Provider_Platform SHALL restore prior behavior
   immediately and losslessly, and derived CPP tables SHALL be safely droppable and rebuildable.
5. THE Desktop_Surface SHALL display, per provider, its negotiated protocol version, feature set, health,
   descriptor catalog, execution logs, and grants, and SHALL preserve all existing Tauri command/event names.

### Requirement 12: Reference Provider Conformance (OpenClaw) and Provider SDK

**User Story:** As a provider author, I want a conformance definition and an SDK, so that I can build a new
provider that plugs into KRIA with no KRIA-core change.

#### Acceptance Criteria

1. THE OpenClaw provider SHALL exercise every mandatory protocol facet (describe, negotiate, discover,
   execute, permission via effects, telemetry) and every optional facet it supports (lifecycle/acquisition,
   streaming where available) end-to-end on the real desktop with real Docker.
2. THE Capability_Provider_Platform SHALL provide a provider conformance suite that validates any provider's
   adapter against the protocol (descriptor validity, negotiation, discovery, execution, effects/permission,
   telemetry, degraded behavior) independent of provider internals.
3. THE Capability_Provider_Platform SHALL provide a Provider SDK (traits, descriptor builders, negotiation
   helpers, and a conformance harness) sufficient to implement a new provider without editing KRIA-core.
4. THE Capability_Provider_Platform SHALL include at least one additional minimal reference provider (an MCP
   server consumed with a derived default descriptor) to prove multi-provider federation and prevent the
   protocol from silently overfitting to OpenClaw.

### Requirement 13: Capability and Provider State Machines

**User Story:** As a platform maintainer, I want explicit, observable state machines for capabilities and
providers, so that lifecycle transitions are deterministic, debuggable, and identical across providers.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL model each capability's lifecycle as the existing authoritative
   `SkillState` machine generalized to all providers (Discovered → Available → Installed → Validated → Ready →
   Executing → Failed → Recovering → Ready | Deprecated → Removed), and SHALL NOT introduce a second, parallel
   capability-state representation.
2. THE Capability_Provider_Platform SHALL model each provider as a provider-session state machine (Offline →
   Connecting → Negotiating → Ready → Syncing → Healthy → Busy → Degraded → Updating → Disconnected) derived
   from `ProtocolSession` + provider health, and SHALL persist the current provider state in
   `provider_sessions`.
3. WHEN a capability or provider transition occurs, THE Capability_Provider_Platform SHALL emit a
   `provider_id`-tagged state-transition audit event, and SHALL reject transitions not permitted by the
   machine.
4. THE Desktop_Surface SHALL display the current state of every capability and provider sourced from these
   machines.
5. WHERE a provider or capability enters a terminal-failure state, THE Capability_Provider_Platform SHALL
   route it to the recovery strategy (Requirement 17) rather than leaving it in an inconsistent state.

### Requirement 14: Observability, Diagnostics, and Developer Tools

**User Story:** As an operator and developer, I want unified events, logs, metrics, tracing, a timeline, and
diagnostics across all providers, so that I can understand and debug any execution without provider-specific
tooling.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL emit a single, provider-neutral, correlation-id-linked event stream
   spanning negotiation, discovery, ranking, permission, acquisition, planning, execution, recovery, learning,
   failure, and cancellation, each tagged with `provider_id` and `capability_id`.
2. THE Capability_Provider_Platform SHALL expose metrics (counts, durations, success/failure rates, latencies,
   resource usage) per provider and per capability, extending the existing `ExecutionMetrics`/`PlatformMetrics`
   collectors rather than adding parallel counters.
3. THE Capability_Provider_Platform SHALL produce an ordered execution timeline per goal (plan → per-node
   start/finish/fail → verification → response) reconstructable from the event stream.
4. THE Capability_Provider_Platform SHALL persist an append-only audit record via the existing `AuditLedger`
   for every decision and lifecycle transition.
5. THE Desktop_Surface SHALL provide developer diagnostics (raw descriptor viewer, negotiated protocol/feature
   inspector, event/timeline viewer, provider health, and grant inspector), gated behind Developer Mode where
   not production-ready.
6. WHERE tracing export is enabled, THE Capability_Provider_Platform SHALL emit spans compatible with the
   existing `tracing` subscriber without requiring provider-specific instrumentation.

### Requirement 15: Performance, Caching, and Background Sync

**User Story:** As a user with many capabilities and providers, I want fast discovery and execution through
caching and background work, so that the platform stays responsive at scale.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL maintain caches for descriptors, marketplace catalogs, embeddings,
   provider sessions, and recent execution results, each with an explicit invalidation trigger
   (descriptor/version change, model-id change, provider event, TTL).
2. WHEN a capability's descriptor, version, or provider state changes, THE Capability_Provider_Platform SHALL
   invalidate only the affected cache entries and apply an incremental index upsert, and SHALL NOT perform a
   full rebuild for a single-capability change.
3. THE Capability_Provider_Platform SHALL hydrate full descriptors lazily (retrieval works on lightweight
   indexed fields; full descriptor loaded on selection) so memory stays bounded at 100,000+ capabilities.
4. THE Capability_Provider_Platform SHALL perform catalog sync, embedding, and reindex as background tasks
   under a bounded work queue, and SHALL keep discovery served from cache while background work proceeds.
5. THE Capability_Provider_Platform SHALL meet stated performance budgets (discovery, ranking, permission
   decision, cache hit, incremental upsert) and SHALL record a budget miss as an honest failure, not a
   silent slowdown.

### Requirement 16: Resource Scheduling and Fairness

**User Story:** As a user running many concurrent capabilities across runtimes, I want KRIA to schedule
Docker, GPU, embeddings, LLM, sidecar, and future runtimes fairly, so that no workload starves and priority
work stays responsive.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL schedule capability execution through a provider-neutral resource
   broker that wraps the existing Hardware Resource Authority (`resource/authority/scheduler.rs::admit`) and
   the OpenClaw `RuntimeScheduler`/`admission` path, and SHALL NOT introduce a second scheduler.
2. THE resource broker SHALL admit, queue, prioritize, preempt, and cancel work by resource class and priority
   generically, driven by descriptor `effects.resource_class` and runtime availability, for any provider.
3. WHEN resources are contended, THE resource broker SHALL apply the existing priority ordering (realtime/
   interactive/background) and bounded queues, and SHALL surface queue position/backpressure honestly.
4. WHEN a capability is cancelled, THE resource broker SHALL release its resources (containers, leases, GPU
   reservations) and return counts to baseline.
5. WHERE a runtime is unavailable (no Docker, no GPU), THE Capability_Provider_Platform SHALL degrade honestly,
   excluding capabilities requiring that runtime from execution while keeping others available.

### Requirement 17: Recovery and Fallback

**User Story:** As a user, I want failures to be recovered or clearly reported, so that a provider, skill,
runtime, or marketplace failure never leaves KRIA hung, leaking, or silently wrong.

#### Acceptance Criteria

1. WHEN a provider, capability, runtime, marketplace, or execution fails or times out, THE
   Capability_Provider_Platform SHALL apply a defined recovery policy (retry with backoff, restart, fallback
   to an alternative capability/provider, or honest decline) via the existing `RecoverySystem`/`RecoveryManager`
   rather than ad-hoc handling.
2. WHERE an alternative capability or provider exists for the required capability tag, THE
   Capability_Provider_Platform SHALL offer it as a fallback (subject to permission), and SHALL record the
   fallback in telemetry.
3. WHEN recovery is attempted, THE Capability_Provider_Platform SHALL bound attempts and time, and SHALL notify
   the user with the real reason on exhaustion — never a generic "unknown error".
4. AFTER any failed, timed-out, cancelled, or recovered run, THE Capability_Provider_Platform SHALL restore
   resources to baseline (no leaked containers, leases, processes, threads, or grants).
5. IF a provider repeatedly fails, THEN THE Capability_Provider_Platform SHALL open a circuit breaker for that
   provider (excluding it from discovery/execution) and report it degraded, so one failing provider cannot
   stall the platform.

### Requirement 18: Rich Capability Descriptor v1.1

**User Story:** As the Brain and as a user, I want descriptors rich enough to plan, permission, explain, and
set expectations, so that selection and UX are accurate without executing to find out.

#### Acceptance Criteria

1. THE Capability_Descriptor SHALL support (additively over v1) example prompts, execution examples, output
   examples, failure examples, common mistakes, best-prompt guidance, known limitations, and confidence — used
   for retrieval hints, planning, and user-facing explanation.
2. THE Capability_Descriptor SHALL support expectation metadata: typical latency, cost, GPU requirement, RAM
   requirement, offline support, host/OS requirement, quality signals, compatibility, and version constraints
   — mapping existing `SkillCapabilities`/`ResourceProfile`/effects onto neutral descriptor fields without
   duplicating them.
3. WHERE a provider omits a v1.1 field, THE Capability_Provider_Platform SHALL treat it as unknown with a
   conservative default (e.g. offline=unknown → assume network needed; gpu=unknown → assume none advertised)
   and SHALL NOT fail descriptor validation.
4. THE Permission_Engine and Capability_Planner SHALL use descriptor expectation metadata (effects, resource,
   offline, host) in their decisions generically, with no per-provider branch.
5. THE Desktop_Surface SHALL render descriptor examples, limitations, expectations, and version constraints in
   the capability viewer.

### Requirement 19: Desktop Experience and Navigation

**User Story:** As a user, I want capabilities, providers, marketplace, approvals, activity, logs, and health
as first-class desktop surfaces, so that I can observe and manage the platform as it grows 10x.

#### Acceptance Criteria

1. THE Desktop_Surface SHALL present a first-class Capabilities area (not buried inside Settings) providing:
   Capability Browser, Marketplace, Provider Manager, Approval Center, Agent Timeline / Execution History,
   Capability Health, Descriptor Viewer, Runtime Monitor, Recovery screen, and Developer Mode.
2. THE Desktop_Surface SHALL preserve all existing Tauri command and event names and reuse the existing built
   views (Capability Manager, Capability Graph, Execution Logs, Permission Manager) by elevating them into the
   Capabilities area rather than rebuilding them.
3. THE Desktop_Surface SHALL render large lists (capabilities, catalog, logs, timeline) with virtualization and
   push+poll synchronization so the UI stays responsive at 100,000+ capabilities and many providers.
4. THE Desktop_Surface SHALL reflect backend state changes within a bounded time via push events and reconcile
   missed events via polling, showing honest loading/degraded/offline/empty states.
5. WHERE a surface is not production-ready, THE Desktop_Surface SHALL gate it behind Developer Mode and mark it
   clearly.

### Requirement 20: Production Definition of Done

**User Story:** As a release owner, I want an objective, evidence-based definition of done, so that CPP is
released only when it truly works under real usage.

#### Acceptance Criteria

1. BEFORE release, THE Capability_Provider_Platform SHALL demonstrate, with real (non-fixture) evidence, that
   discovery, recommendation, installation, verification, execution, recovery, permission/approval, restart,
   offline, upgrade, and migration all work end-to-end on the real desktop with real Docker and a real LLM.
2. BEFORE release, THE Capability_Provider_Platform SHALL demonstrate logs, metrics, and tracing are produced
   and correlated for real runs.
3. BEFORE release, THE Capability_Provider_Platform SHALL demonstrate zero leaked Docker containers, processes,
   threads, leases, or permission grants after sustained real usage (100+ diverse prompts and a long-running
   session).
4. BEFORE release, THE Capability_Provider_Platform SHALL demonstrate manual usability of the desktop
   Capabilities area for install, execute, approve, recover, update, and observe flows.
5. BEFORE release, THE Capability_Provider_Platform SHALL confirm no provider-specific knowledge exists inside
   KRIA-core (boundary-integrity check green) and at least two providers are federated.
6. IF any Definition-of-Done item lacks real evidence, THEN THE release gate SHALL emit No-Go listing the
   missing evidence, and SHALL NOT emit a Go on fixture/simulated/skipped evidence.

### Requirement 21: Dead-Code Removal, Deprecation, and Migration

**User Story:** As a maintainer, I want a clear, safe endpoint at which legacy code is intentionally removed,
so that KRIA does not carry duplicate architectures or deprecated modules forever.

#### Acceptance Criteria

1. THE Capability_Provider_Platform SHALL define a migration path with an explicit "debt-removal point": once
   the platform is default-on, soaked, and validated, the superseded legacy paths SHALL be removed.
2. WHEN the debt-removal point is reached, THE Capability_Provider_Platform SHALL remove the deprecated
   `openclaw::handler::register_skill`, retire the closed `execution::ExecutorKind` enum in favor of the
   `provider_id` seam, remove the flag-off direct-router compatibility branch, and remove the reserved
   `#[allow(dead_code)]` runtime fields that are superseded, each behind its own verified step.
3. THE Capability_Provider_Platform SHALL run a dead-code / unused-trait / unused-config detection pass and
   record findings before removal, and SHALL NOT remove code still reachable in a supported configuration.
4. WHEN a legacy path is removed, THE Capability_Provider_Platform SHALL prove via tests that no supported user
   flow regresses, and SHALL preserve one owner each for registry, runtime, execution, marketplace, installer,
   generation, routing, permission, and desktop integration (no duplication surviving).
5. WHILE migration is incomplete, THE Capability_Provider_Platform SHALL keep the legacy path behind the flag
   for safe rollback, and SHALL remove it only after the removal criteria are met.
6. THE Capability_Provider_Platform SHALL consolidate the fragmented `capability`/`capability_registry`
   modules (`mcp/`, `resource/authority/`, `openclaw/`, `platform/intent/`) so exactly one module owns the
   provider-neutral capability boundary.
