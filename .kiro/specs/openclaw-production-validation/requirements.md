# Requirements Document

Feature: OpenClaw Production Validation & Hardening

## Introduction

OpenClaw (the sandboxed Docker skill substrate) has its architecture built and locked across phases
**A0–A9** (runtime, containers, warm pool, registry, semantic router, execution engine, marketplace/
ClawHub, autonomous skill generation). A deep code+runtime audit established the **current state** per
phase. This spec does **not** add new architecture. It **validates that the built system actually works
end-to-end today, and hardens the gaps the audit found**, so OpenClaw can be declared production-ready
against objective, testable criteria.

Each requirement below records: **Purpose** (why), **Current State** (audit evidence — what is verified
to work / partial / broken today), **Expected State** (production-ready target), and **Acceptance
Criteria** (EARS-style, testable). Non-destructive validation is preferred; every container/marketplace
action is exercised against a controlled test rig, never a live shared registry.

### Iteration gate (applies to the whole spec)

Work proceeds one requirement at a time behind a **feature flag where behavior changes** (flag-OFF =
byte-for-byte unchanged). A requirement is **not "done"** until:
- (a) CI-safe unit/integration tests are green,
- (b) a focused **live validation** passes on a real desktop (enable OpenClaw from UI → run the target
  prompt/flow → observe container + skill + response),
- (c) **0 zombie/leaked containers, 0 leaked leases** after the run,
- (d) **no regression** in previously passed requirements.

`Iteration 1 → Fix → Iteration 2 → … → Final Acceptance`. Verification is never weakened to pass; no
fabricated numbers. Each iteration re-runs the full passed-requirement set before advancing.

## Glossary

- **Live gate**: focused re-run of the target prompt/flow through the real UI path (desktop app →
  OpenClaw enabled), observed in logs/telemetry.
- **Test rig**: a controlled, local Docker image + a local/fixture marketplace repo (`index.json` +
  skill bundles) used so validation never depends on the public ObaidGits/kria-skills repo.
- **Leak**: any container, lease, temp mount, or process that survives a completed/failed/cancelled run.
- **Flag-OFF parity**: with the fix's flag disabled, behavior is byte-for-byte the prior behavior
  (asserted by a test).

## Requirements

### Requirement 1: Enable / disable OpenClaw from the UI (lifecycle)

**User Story:** As a user, I want to enable/disable OpenClaw from Settings, so that the runtime, pool,
and registry come up and tear down cleanly on demand.

**Purpose:** A user must be able to turn OpenClaw on and off from Settings and have the runtime, warm
pool, and registry come up / tear down cleanly — the entry point for everything else.

**Current State:** Config-gated (`kria_config.toml` + `openclaw/config.rs`, `init.rs`); desktop boot/
shutdown wiring present. Audit to confirm the Settings toggle actually drives runtime up/down and that
shutdown reaps the pool.

**Expected State:** Toggling OpenClaw in Settings deterministically starts/stops the runtime with no
leaked containers and an honest health/status surface.

#### Acceptance Criteria
1. WHEN OpenClaw is enabled from the Settings UI, the system SHALL start the runtime and report a
   `ready`/`degraded`/`unavailable` status honestly (Docker present vs absent).
2. WHEN OpenClaw is disabled from the Settings UI, the system SHALL stop the runtime and reap the warm
   pool with **0 leaked containers and 0 leaked leases**.
3. IF Docker is unavailable, THEN enabling SHALL report `unavailable` with a clear reason and SHALL NOT
   crash the desktop app or block other tools.
4. WHEN the desktop app shuts down with OpenClaw enabled, the system SHALL tear down all OpenClaw
   containers before exit.
5. IF the `openclaw_enabled` config/flag is OFF, THEN no OpenClaw runtime, pool, or container SHALL be
   created (byte-for-byte prior behavior).

### Requirement 2: Container lifecycle & warm pool integrity

**User Story:** As a user, I want skill containers created, reused, recovered, and cleaned up correctly,
so that runs never leak zombie containers or leases.

**Purpose:** Skills run in containers; the pool must create, reuse, health-check, recover, and clean up
containers without leaks under normal, timeout, cancel, and crash paths.

**Current State:** `pool.rs`, `runtime/docker.rs`, `runtime_manager.rs`, `bridge.rs` (JSON-RPC) present.
Audit to confirm warm-pool reuse, health/recovery, timeout, cancellation, and crash cleanup are wired
and leak-free vs partially wired.

**Expected State:** Every acquire/release cycle is balanced; crashed/timed-out/cancelled containers are
detected and cleaned; no zombies; image version is pinned and verified.

#### Acceptance Criteria
1. WHEN a skill is executed, the runtime SHALL acquire a container from the warm pool (reuse when
   healthy) and release it back or destroy it on completion.
2. WHEN a container becomes unhealthy or exits unexpectedly, the runtime SHALL detect it and remove it
   from the pool (no reuse of a dead container).
3. WHEN a skill run exceeds its timeout OR is cancelled, the runtime SHALL terminate the container and
   release its lease within a bounded time, leaving **0 zombies**.
4. WHEN N sequential skill runs complete, the count of OpenClaw containers/leases SHALL return to the
   pool baseline (leak assertion).
5. WHEN the runtime starts, it SHALL verify the pinned Docker image is present (build/pull if allowed)
   and refuse to run skills against a missing/mismatched image with a clear reason.
6. WHEN a container communicates over the JSON-RPC bridge, malformed/oversized messages SHALL be
   rejected without hanging the runtime.

### Requirement 3: Install a skill from the marketplace (ClawHub)

**User Story:** As a user, I want to discover and install a skill from the marketplace, so that it is
verified and becomes runnable.

**Purpose:** The core user story — discover a skill in the marketplace, download, verify, install, and
have it become executable.

**Current State:** `clawhub.rs`, `registry.rs`, `materialize.rs`, `admission.rs` present. Audit flagged
the marketplace source: the public `https://raw.githubusercontent.com/ObaidGits/kria-skills/.../index.json`
lists **1 skill while the local skills DB has 3** — index/DB drift must be reconciled and validation must
run against the **test rig**, not the drifting public repo.

**Expected State:** Install flow works against a controlled `index.json`: search → download → verify
(hash/signature/manifest schema) → install → register → the skill is discoverable and runnable. Index/DB
drift is detected and surfaced, not silently ignored.

#### Acceptance Criteria
1. WHEN the marketplace is synced against the test-rig `index.json`, the system SHALL list exactly the
   skills declared there and report the source URL used.
2. WHEN a user installs a listed skill, the system SHALL download the bundle, verify its manifest schema
   and integrity (hash/signature), materialize it, and register it as installed.
3. IF verification fails (bad hash, bad signature, invalid manifest), THEN install SHALL abort with a
   clear reason and SHALL NOT register a partial/invalid skill.
4. WHEN the installed-skills view is opened, an installed skill SHALL appear with its version and
   capabilities.
5. IF the marketplace `index.json` and the local skills DB disagree (drift, e.g. 1 vs 3), THEN the
   system SHALL surface the drift (which skills are DB-only vs index-only) rather than silently showing
   one count.
6. WHEN the network/repo is unreachable, sync SHALL fail gracefully (cached/offline state) without
   crashing.

### Requirement 4: Execute an installed skill end-to-end (real pipeline)

**User Story:** As a user, I want a chat prompt to actually run the matching installed skill in a
container and return its real output, so that the pipeline is proven end-to-end.

**Purpose:** Prove the actual runtime path a chat prompt takes to run a skill: prompt → router → tool
selection → OpenClaw → semantic router → registry → execution engine → executor → runtime → container →
skill → response.

**Current State:** `semantic_router.rs`, `resolver.rs`, `handler.rs`, execution-engine wiring present.
Audit to determine whether chat actually routes through A6 semantic router + A7 execution engine, or a
legacy/bypassed path, or mixed.

**Expected State:** A chat prompt that maps to an installed skill executes it in a container and returns
the skill's real output, with the routing path observable in telemetry (which router/engine ran).

#### Acceptance Criteria
1. WHEN a user submits a prompt that matches an installed skill, the system SHALL select OpenClaw, route
   via the semantic router to the correct skill, execute it in a container, and return the skill output.
2. WHEN a skill executes, telemetry/logs SHALL record the actual path taken (router match score,
   registry hit, executor used) so the pipeline is not a black box.
3. IF no installed skill matches the prompt above the router threshold, THEN OpenClaw SHALL decline
   cleanly (fall back to native tools) rather than force a wrong skill.
4. WHEN a skill declares required capabilities, execution SHALL enforce them (deny undeclared
   filesystem/network/etc. access) per the safety policy.
5. WHEN a skill run finishes, its container SHALL be released/destroyed (ties to Requirement 2 leak
   assertion).

### Requirement 5: Autonomous skill generation (A9) end-to-end

**User Story:** As a user, I want KRIA to generate, validate, package, install, and run a new skill, so
that autonomous skill creation is real and not architecture-only.

**Purpose:** Validate whether KRIA can really generate → validate → test → repair → package → sign →
install → register → execute → reuse a **new** skill, or whether A9 is architecture/tests only.

**Current State:** `generation/` present with `designer`, `codegen`, `llm_generator`, `validator`,
`quality`, `sandbox`, `pipeline`, `approval`, `budget`, `decision`, `tests`. Audit to trace the full
flow and mark each stage real vs stub.

**Expected State:** From a capability request, the pipeline produces a working, validated, packaged,
installed skill that then executes via the normal Requirement 4 path — or, where a stage is honestly not
production-ready, it is gated behind Developer Mode and reports that clearly (no fake success).

#### Acceptance Criteria
1. WHEN skill generation is requested for a well-specified capability, the pipeline SHALL run design →
   codegen → validate → test → package and report the outcome of each stage truthfully.
2. WHEN generated code fails validation/tests, the pipeline SHALL attempt bounded repair and, if still
   failing, SHALL abort with the failing stage and reason — it SHALL NOT install a broken skill.
3. WHEN generation succeeds, the packaged skill SHALL install and register via the same path as a
   marketplace skill (Requirement 3) and then execute (Requirement 4).
4. WHERE generation requires human approval or exceeds its budget, the pipeline SHALL pause for approval
   / stop at the budget boundary (no unbounded LLM spend).
5. IF any generation stage is not production-ready, THEN it SHALL be gated behind Developer Mode and
   labeled as such in the UI (no silent fake path).

### Requirement 6: Skill management — update, enable/disable, uninstall, hot reload

**User Story:** As a user, I want to update, enable/disable, and uninstall skills, so that installed
skills stay manageable and consistent.

**Purpose:** Installed skills must be manageable: update to a new version, enable/disable, uninstall
cleanly, and reload without a full restart.

**Current State:** `registry.rs`, `revocation.rs`, `activation.rs`, `materialize.rs` present. Audit to
confirm update/uninstall/enable-disable/hot-reload are wired vs partial.

**Expected State:** Each management action changes runtime behavior correctly and leaves the registry +
filesystem + DB consistent.

#### Acceptance Criteria
1. WHEN a skill is disabled, subsequent prompts SHALL NOT route to it; WHEN re-enabled, routing SHALL
   resume.
2. WHEN a skill is uninstalled, its bundle, registration, and DB row SHALL be removed and it SHALL no
   longer be discoverable or runnable (no orphaned files/leases).
3. WHEN a skill is updated to a new version, the new version SHALL supersede the old and the old
   version's artifacts SHALL be cleaned up.
4. WHEN a skill is installed/updated/removed while OpenClaw is running, the change SHALL take effect
   without requiring a desktop restart (hot reload) OR the system SHALL clearly state a restart is
   required.
5. AFTER any management action, the registry, filesystem, and skills DB SHALL be mutually consistent
   (no drift — ties to Requirement 3.5).

### Requirement 7: Failure injection & recovery

**User Story:** As a user, I want OpenClaw to fail safely under Docker/container/network failures, so
that failures never hang, crash, fake success, or leak resources.

**Purpose:** Production readiness means graceful behavior under failure: Docker down, image missing,
container crash mid-run, bridge hang, network loss, disk pressure.

**Current State:** Recovery/health code exists across `pool.rs`, `runtime_manager.rs`, `handler.rs`.
Audit to confirm each failure mode is handled vs unhandled.

**Expected State:** Every injected failure yields a clear, honest error and full resource cleanup — never
a hang, crash, silent success, or leak.

#### Acceptance Criteria
1. WHEN Docker is stopped mid-session, in-flight and new skill runs SHALL fail with a clear reason and
   SHALL NOT hang; the rest of the app SHALL keep working.
2. WHEN a container crashes mid-run, the runtime SHALL surface a failure, clean up the container/lease,
   and remain able to serve the next run.
3. WHEN the JSON-RPC bridge stalls, the run SHALL time out and clean up (ties to Requirement 2.3/2.6).
4. WHEN the marketplace repo is unreachable or serves a malformed `index.json`, sync/install SHALL fail
   gracefully with a clear reason.
5. AFTER any injected failure, the container/lease count SHALL return to baseline (**0 leaks**).

### Requirement 8: Settings surface completeness

**User Story:** As an operator, I want the Settings panel to expose the controls to observe and control
OpenClaw, so that I can operate it from the UI.

**Purpose:** The audit flagged missing Settings controls. Production readiness requires the operator can
observe and control OpenClaw from the UI.

**Current State:** Partial Settings wiring. Audit to enumerate present vs missing controls.

**Expected State:** Settings exposes the controls needed to operate OpenClaw; anything intentionally
omitted is a conscious decision, not an oversight.

#### Acceptance Criteria
1. WHERE OpenClaw is operable, Settings SHALL expose at minimum: enable/disable, marketplace/repository
   source, installed skills list (with enable/disable/uninstall), generated skills, Developer Mode, and
   an honest health/status + logs surface.
2. WHEN a control acts on the runtime (e.g. refresh repository, sync marketplace, rebuild image), the UI
   SHALL reflect success/failure honestly (no fake "done").
3. IF a capability is not yet production-ready, THEN its control SHALL be hidden or clearly marked
   experimental/Developer-Mode rather than shown as a normal feature.

### Requirement 9: Telemetry, metrics & honest health

**User Story:** As an operator, I want accurate telemetry, metrics, and health, so that OpenClaw's state
can be trusted and verified against reality.

**Purpose:** Nothing can be trusted as production-ready if it cannot be observed. Health/metrics must
reflect reality.

**Current State:** `event.rs`/`events.rs`, audit ledger, health checks present. Audit to confirm metrics
are real (counts, durations, outcomes) vs placeholder.

**Expected State:** Container counts, run outcomes, durations, install/generation results, and health
state are recorded and queryable, and match observed reality.

#### Acceptance Criteria
1. WHEN skills run, the system SHALL record run count, success/failure, and duration accurately.
2. WHEN containers are created/destroyed, live container/lease counts SHALL be observable and match the
   real Docker state (verifiable against `docker ps`).
3. WHEN health is queried, the reported state SHALL reflect actual runtime/Docker/pool state (no
   optimistic constant).
4. WHEN install or generation runs, its outcome SHALL be recorded in the audit ledger.

### Requirement 10: Final acceptance — production-readiness verdict

**User Story:** As the owner, I want an objective, evidence-backed production-readiness verdict, so that
I can decide to freeze OpenClaw or continue development.

**Purpose:** Convert the iteration results into an objective go/no-go per the audit's Section 16
questions.

**Current State:** N/A (aggregation requirement).

**Expected State:** A recorded, evidence-backed verdict: architecture / implementation / runtime /
integration complete? production/developer/user ready? freeze OpenClaw architecture yes/no?

#### Acceptance Criteria
1. WHEN Requirements 1–9 have each passed their live gate with 0 leaks and no regression, the spec SHALL
   record a production-readiness verdict backed by the passing evidence (not opinion).
2. IF any requirement cannot pass, THEN the verdict SHALL list it under remaining work classified
   Critical / Important / Optional / Nice-to-have.
3. WHEN the verdict is "production ready", it SHALL be reproducible by re-running the full validation
   suite from a clean state.

---

## Requirements (Production-Review Additions)

> The following requirements were appended after a senior production-review pass. They do not replace or
> weaken R1–R10; they close gaps that would otherwise force future architectural rework. Where an
> existing requirement already covers a concern, it is referenced rather than duplicated.

### Requirement 11: Root Router path integrity (no bypass)

**User Story:** As the owner, I want every skill execution to flow through the canonical path
Root Router → OpenClaw tool selection → Semantic Router → Execution Engine → Runtime → Container →
Skill → Response, so that no request ever bypasses the Root Router or a pipeline stage.

**Purpose:** R4 proves a skill runs end-to-end, but does not forbid alternate/legacy entry points. A
bypassed Root Router is the single most likely source of future rework and inconsistent safety/telemetry.

**Current State:** `semantic_router.rs`, `resolver.rs`, `handler.rs` and execution-engine wiring exist;
audit flagged uncertainty over whether chat uses A6+A7 or a legacy/mixed path. Must be pinned down.

**Expected State:** The Root Router is the sole entry to OpenClaw; the full stage sequence is traversed
(or explicitly short-circuited with a recorded reason) for 100% of skill executions, verifiable in
telemetry. No code path invokes the runtime/executor directly around the Root Router.

#### Acceptance Criteria
1. WHEN any skill executes (marketplace, GitHub, local bundle, or generated), the execution SHALL enter
   via the Root Router and traverse OpenClaw selection → Semantic Router → Execution Engine → Runtime →
   Container → Skill → Response, with each stage recorded in telemetry (ties to R4.2, R9).
2. THE system SHALL NOT expose or use any alternate entry that reaches the runtime/executor while
   bypassing the Root Router (asserted by test + code review).
3. IF a stage is intentionally short-circuited (e.g. cached decision), THEN the short-circuit and its
   reason SHALL be recorded — it SHALL NOT be a silent bypass.
4. WHEN telemetry for a run is inspected, the recorded path SHALL match the canonical sequence for that
   run type (no missing or reordered stages).

### Requirement 12: Unified installer — all skill sources converge

**User Story:** As a user, I want every way a skill enters the system (marketplace, GitHub repo, local
`.ocskill` bundle, A9-generated, future private/enterprise repos) to use the same installer pipeline,
so that behavior, verification, and registration are identical regardless of source.

**Purpose:** R3 validates the marketplace path only. Multiple divergent installers would guarantee
future rework and inconsistent verification/trust handling.

**Current State:** `clawhub.rs`, `registry.rs`, `materialize.rs`, `admission.rs`, `bundle/` present;
convergence across all sources not yet asserted.

**Expected State:** One installer pipeline (download/acquire → verify manifest+integrity+trust →
materialize → register) is the only way any skill becomes installed, regardless of origin.

#### Acceptance Criteria
1. WHEN a skill is installed from the ClawHub marketplace, a GitHub repository, a local `.ocskill`
   bundle, OR the A9 generator, it SHALL pass through the same installer pipeline (verify → materialize
   → register).
2. THE installer SHALL apply the same manifest-schema, integrity, and trust checks to every source; a
   source SHALL NOT be able to skip verification (ties to R3.2/R3.3).
3. WHERE a future private/enterprise repository is added, it SHALL plug in as a source to the same
   pipeline without a new installer code path (extensible source interface).
4. AFTER installation from any source, the resulting registry entry, filesystem layout, and DB row SHALL
   be structurally identical (source recorded as metadata only, not as a behavioral fork).
5. IF a local `.ocskill` bundle is malformed or fails verification, THEN install SHALL abort with a clear
   reason and register nothing (parity with R3.3).

### Requirement 13: Generated skills are indistinguishable from authored skills

**User Story:** As a user, I want A9-generated skills to be treated exactly like human-authored skills,
so that there is never a separate AI execution path with different behavior, trust, or observability.

**Purpose:** R5 proves generation can produce a runnable skill, but does not forbid a divergent
generated-skill runtime. A separate path would fragment safety, telemetry, and lifecycle forever.

**Current State:** `generation/` produces bundles; must be asserted to converge with R12 installer and
R11 execution path.

**Expected State:** After packaging, a generated skill is byte-compatible with an authored skill and
shares bundle format, installer, registry, marketplace representation, Semantic Router, Execution Engine,
Runtime, telemetry, update, and lifecycle. No `is_generated` branch alters execution.

#### Acceptance Criteria
1. WHEN a generated skill is packaged, its bundle format and manifest SHALL be identical in structure to
   an authored skill's (only provenance metadata differs).
2. WHEN a generated skill is installed and executed, it SHALL use the same installer (R12), Root Router
   path (R11), runtime, and container handling as an authored skill — no separate execution branch.
3. THE system SHALL NOT contain a code path that changes execution, verification, or telemetry behavior
   based solely on a skill being AI-generated (asserted by test + code review).
4. WHEN generated skills are updated, disabled, uninstalled, or hot-reloaded, they SHALL obey R6
   identically to authored skills.
5. WHEN telemetry is recorded, a generated skill SHALL emit the same event set as an authored skill
   (provenance is an attribute, not a separate schema).

### Requirement 14: Settings as single source of truth

**User Story:** As a user, I want every operable OpenClaw feature controllable from the Settings UI, so
that I never need to hand-edit TOML, JSON, or environment variables for normal operation.

**Purpose:** R8 lists which controls must exist; this requirement adds the stronger guarantee that the UI
is authoritative and file-editing is not required for normal operation.

**Current State:** Config lives in `kria_config.toml` + `openclaw/config.rs`; Settings wiring partial.

**Expected State:** All normal-operation settings are read/written through the Settings UI and persisted;
manual file editing is reserved for advanced/recovery scenarios only and is never required for standard
use.

#### Acceptance Criteria
1. WHERE a setting governs normal OpenClaw operation (enable/disable, marketplace source, skill enable/
   disable/uninstall, Developer Mode, pool/timeout/limits exposed to users), it SHALL be settable from
   the Settings UI without editing TOML/JSON/env.
2. WHEN a setting is changed in the UI, it SHALL be persisted and survive a desktop restart.
3. WHEN a setting is changed in the UI, the running runtime SHALL adopt it (immediately or with a clearly
   stated restart requirement — ties to R6.4).
4. IF a setting is advanced/experimental and intentionally file-only, THEN it SHALL be documented as such
   and SHALL NOT be required for normal operation.
5. THE UI SHALL reflect the current persisted value of each setting on load (no stale/default display
   that disagrees with config — ties to R16).

### Requirement 15: No demo / fake / placeholder paths (honesty invariant)

**User Story:** As the owner, I want the shipped system to contain no fake successes, placeholder
implementations, mock UI behavior, or silent bypasses, so that every reported state reflects reality.

**Purpose:** Honesty is asserted piecemeal in R1/R3/R5/R8. This is the global invariant that makes the
production verdict trustworthy.

**Current State:** Audit exists to flag stubs/placeholders/dead paths; this requirement makes their
absence (or explicit gating) a release condition.

**Expected State:** Any feature that is not production-ready either reports its true state (`degraded`/
`unavailable`/`experimental`) or is hidden behind Developer Mode — never presented as working when it is
not.

#### Acceptance Criteria
1. THE production build SHALL NOT return a success result for an operation that did not actually occur
   (no fake success), for install, execute, generate, update, remove, or sync.
2. WHERE a feature is incomplete, it SHALL report `degraded`/`unavailable`/`experimental` honestly OR be
   hidden behind Developer Mode (ties to R8.3, R5.5).
3. THE production UI SHALL NOT display mock/placeholder data as if it were live runtime state.
4. THE runtime SHALL NOT silently bypass a stage or a safety check to make an operation appear to
   succeed (ties to R11.2/R11.3).
5. WHEN a placeholder or TODO remains in a user-reachable path, it SHALL be surfaced honestly or gated,
   and SHALL be enumerated in the final limitations list (ties to R10.2).

### Requirement 16: UI / backend state synchronization (no stale UI)

**User Story:** As a user, I want every backend state change to reflect in the UI promptly, so that what
I see always matches actual runtime state.

**Purpose:** Not covered by R1–R10. Stale UI causes false operator decisions and support burden.

**Current State:** Event system (`event.rs`/`events.rs`) and desktop event emission exist; end-to-end UI
sync not yet asserted.

**Expected State:** Installed/removed/disabled skills, running containers, marketplace sync, health,
logs, generation progress, and container status update in the UI within a bounded time of the backend
change, via events (not manual refresh).

#### Acceptance Criteria
1. WHEN a skill is installed, removed, disabled, enabled, or updated in the backend, the UI SHALL reflect
   the change within a bounded time without a manual refresh.
2. WHEN a container is created or destroyed, the UI's container/status view SHALL update to match the
   real state (ties to R9.2).
3. WHEN a marketplace sync, health change, or generation progresses, the UI SHALL update live.
4. IF an event is missed/dropped, THEN the UI SHALL reconcile to true backend state on next poll/refresh
   (eventual consistency, no permanently stale view).
5. THE UI SHALL NOT show a skill/container/health state that contradicts the backend after
   synchronization completes.

### Requirement 17: Telemetry completeness (every action observable)

**User Story:** As an operator, I want every production action to emit telemetry, so that install,
update, remove, execute, generate, repair, container lifecycle, marketplace sync, router selection,
execution path, failures, and cancellations are all observable.

**Purpose:** R9 covers run/container/install/generation outcomes. This extends coverage to the full
action set and makes "every action observable" the rule.

**Current State:** Audit ledger + events exist; full enumeration not yet guaranteed.

**Expected State:** Each listed action produces a telemetry/audit record with outcome, timing, and
correlation to its run; there is no unobservable production action.

#### Acceptance Criteria
1. THE system SHALL emit telemetry for each of: install, update, remove, execute, generate, repair,
   container creation, container destruction, marketplace sync, router selection, execution-path
   traversal, failure, and cancellation.
2. EACH telemetry record SHALL include outcome (success/failure/cancelled), timing, and a correlation id
   linking it to its run/skill (ties to R11.1, R9).
3. WHEN an action fails or is cancelled, its telemetry SHALL record the reason (parity with R7 honest
   failures).
4. THERE SHALL be no production action in the listed set that completes without a corresponding
   telemetry record (asserted by test).

### Requirement 18: Long-running / soak stability

**User Story:** As the owner, I want OpenClaw to stay stable under continuous long-duration use, so that
memory, containers, database, warm pool, GPU, and the desktop do not degrade over time.

**Purpose:** R2.4 asserts per-cycle leak balance; it does not cover sustained soak behavior where slow
leaks and drift appear.

**Current State:** Not covered by R1–R10.

**Expected State:** Over a sustained soak run, resource usage is bounded, containers/leases return to
baseline, the skills DB stays consistent, the warm pool remains healthy, and the desktop stays
responsive.

#### Acceptance Criteria
1. WHEN OpenClaw runs a sustained soak (many hours / many runs), process memory SHALL remain bounded (no
   monotonic leak) within a stated tolerance.
2. THROUGHOUT the soak, container and lease counts SHALL periodically return to the pool baseline (no
   slow container leak — extends R2.4).
3. AFTER the soak, the skills DB, registry, and filesystem SHALL remain mutually consistent (extends
   R6.5).
4. THROUGHOUT the soak, the warm pool SHALL remain healthy (recovering unhealthy containers per R2.2)
   and the desktop SHALL remain responsive.
5. WHERE GPU is used, GPU memory SHALL return to baseline between runs over the soak (no GPU leak).

### Requirement 19: Upgrade / migration compatibility

**User Story:** As a user, I want OpenClaw upgrades to preserve my installed skills, registry,
marketplace state, generated skills, settings, database, and configuration, so that upgrading never
corrupts or loses my data.

**Purpose:** Not covered by R1–R10. Without a migration contract, future architecture-locked upgrades
risk data loss and rework.

**Current State:** Not covered.

**Expected State:** An upgrade from a prior OpenClaw version preserves all persistent state, migrating
schema/format where needed, with a safe path when migration is not possible.

#### Acceptance Criteria
1. WHEN OpenClaw is upgraded to a new version, installed skills, generated skills, registry entries,
   marketplace state, settings, and the skills DB SHALL be preserved (no data loss).
2. WHERE the on-disk schema or bundle format changes, the upgrade SHALL migrate existing data forward
   without corruption.
3. IF a persistent artifact cannot be migrated, THEN the upgrade SHALL fail safe (preserve the original,
   report clearly) rather than corrupt or silently drop it.
4. AFTER an upgrade, previously installed and generated skills SHALL remain discoverable and executable
   via the normal path (R11) without reinstallation.
5. THE upgrade SHALL be idempotent (re-running it SHALL NOT duplicate or corrupt state).

### Requirement 20: Production acceptance benchmark & freeze gate

**User Story:** As the owner, I want a single, reproducible production benchmark that exercises the whole
system under load and failure, so that OpenClaw is only declared production-ready and architecture-frozen
after it fully passes.

**Purpose:** R10 records a verdict; this requirement defines the concrete, mandatory benchmark and the
freeze gate that the verdict depends on. It is the hardest gate in the spec.

**Current State:** Not covered as a single benchmark; individual behaviors are spread across R1–R19.

**Expected State:** One benchmark suite runs a defined mixed workload plus failure and pressure scenarios
from a clean state and must pass fully; passing it is the precondition for declaring production readiness
(R10) and freezing the architecture.

#### Acceptance Criteria
1. THE benchmark SHALL execute at least: 100 skill-invocation prompts, 50 installs, 20 updates, 20
   removals, and 20 generated skills, from a clean state.
2. THE benchmark SHALL include a desktop restart, a Docker restart, induced container crashes,
   cancellation, and timeouts, and SHALL verify correct recovery and 0 leaks after each (ties to R7).
3. THE benchmark SHALL include parallel execution and memory + GPU pressure scenarios and SHALL verify
   bounded resource use and stability (ties to R18).
4. AFTER the full benchmark, container/lease counts SHALL be at baseline, the DB/registry/filesystem
   SHALL be consistent, and all telemetry (R17) SHALL be present.
5. THE freeze gate SHALL require that ALL of the following are verified before OpenClaw is declared
   production-ready/frozen: architecture, runtime, execution path (R11), unified installer (R12),
   marketplace (R3), skill generation (R5/R13), settings authority (R14), UI sync (R16), telemetry
   (R17), recovery (R7), long-running stability (R18), upgrade compatibility (R19), and this benchmark.
6. IF any element of the freeze gate fails, THEN OpenClaw SHALL NOT be declared frozen, and the failing
   element SHALL be classified Critical / Important / Optional / Nice-to-have (ties to R10.2).
7. THE benchmark SHALL be reproducible from a clean state and its pass/fail SHALL be evidence-backed, not
   asserted by opinion (ties to R10.3).
