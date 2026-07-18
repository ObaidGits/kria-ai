# Requirements Document

KRIA Memory Upgrade (Cognitive Memory System)

## Introduction

The KRIA Memory Upgrade replaces KRIA's current fragmented memory (multiple SQLite
files plus a brute-force in-RAM vector index) with a single local-first **cognitive
memory system** — the cognitive backbone every KRIA subsystem reads from and writes
through. It must remember, understand, organize, reason, consolidate, forget
intelligently, and improve over years while remaining transparent, private,
offline-capable, and fully user-controlled.

These requirements are **derived from** the approved technical design
(`design.md`), which is itself the implementation-ready realization of the
authoritative architecture (`MEMORY_ARCHITECTURE_FINAL.md`). The design's twelve
inviolable laws (**L1–L12**) and its canonical requirement set (**R1–R20**, design
§36) are the source for the user stories and EARS acceptance criteria below. Each
requirement cross-references the design sections and correctness properties (CP-n)
that satisfy it, so requirements, design, and tests stay traceable.

### Scope

**In scope (v1, phased P1→P4):** SQLite transactional authority + append-only event
log + transactional outbox; the Memory Write Policy Engine (single write gate); memory
modes; the Cognitive Scheduler; the Memory API Contract; ONNX embeddings with a
degradation floor; vector + full-text + graph retrieval with adaptive fusion; the
Truth Maintenance System; importance + Memory Worth scoring; consolidation/dreaming;
the unified memory lifecycle (merge/split/promote/compress/forget/delete/restore);
Library ingestion with per-item cascade; privacy/erasure via crypto-shredding;
authority-only backup/restore; observability; subsystem + OpenClaw integration; and a
scale benchmark harness.

**Out of scope (v1, reserve schema only):** multi-device sync, cloud services,
multimodal retrieval, local model training, 3D visualization, autonomous multi-agent
orchestration, third-party plugin marketplace.

**Dev-context scoping (steering `dev-context.md`, design §47 — governs the current
build):** KRIA is a single-laptop, single-user, pre-production build where data loss is
acceptable and dead code is deleted. The following requirements are therefore **kept in
design but deferred (future-only)** and are NOT implemented now: **R11 backup/restore**,
the **at-rest encryption** portion of **R18** (rely on OS disk encryption; secret-handling
+ crypto-shred are still built), the cross-process **writer-leader** parts of **R14**
(single-process reality; local-FS guard kept), and portable `.kmem` export/import. **R24**
is delivered as a **hard cutover with legacy deletion** (no compatibility shim). All other
requirements are in the active build (MVP → Phase 3, see `tasks.md`).

## Glossary

- **Authority** — the single SQLite database that is the sole transactional source of
  truth (L2). Every other store is a derived, rebuildable index (L4).
- **Write Policy Engine** — the single mandatory gate through which all durable writes
  flow (L3); a synchronous deterministic fast path plus an asynchronous best-effort
  slow path.
- **Event** — an immutable append-only record used for audit, provenance, and erasure;
  never used to regenerate derived memory content.
- **Memory** — a derived, durable, mutable knowledge unit.
- **Outbox** — index updates enqueued inside the authority transaction and relayed to
  derived indexes idempotently.
- **Crypto-shredding** — erasure achieved by destroying a per-subject encryption key so
  ciphertext becomes permanently unreadable, without mutating the immutable log (L9).
- **EARS** — Easy Approach to Requirements Syntax (WHEN/IF/WHILE … THE SYSTEM SHALL …).

### Requirements Index

| Req | Title | Priority | Phase |
|---|---|---|---|
| 1 | Single transactional authority & immutable event log | Must | P1 |
| 2 | Memory Write Policy Engine (single write gate) | Must | P1 |
| 3 | Memory modes | Must | P1 |
| 4 | Temporary and Incognito non-persistence | Must | P1 |
| 5 | Selective write filtering (quality) | Must | P1 |
| 6 | Truth Maintenance System | Must | P2 |
| 7 | LLM/embedding-independent graceful degradation | Must | P1 |
| 8 | Consent-gated cold start | Must | P1 |
| 9 | Library ingestion & per-item erasure | Must | P2 |
| 10 | Right to be forgotten (crypto-shred + cascade) | Must | P1 |
| 11 | Backup & restore (authority-only) | Must | P2 |
| 12 | Derived-index consistency (outbox + reconciliation) | Must | P1 |
| 13 | Retrieval quality at scale (release gate) | Must | P2 |
| 14 | Crash safety & recovery | Must | P1 |
| 15 | Merge / split atomicity | Must | P2 |
| 16 | Feedback-driven learning | Should | P3 |
| 17 | Explainability | Must | P2 |
| 18 | Encryption at rest | Must | P1 |
| 19 | Namespace & scope isolation | Must | P1 |
| 20 | Resource governance & bounded growth | Must | P1 |
| 21 | Cognitive layer (consolidation, dreaming, reflection) | Should | P3 |
| 22 | Embedding-version migration | Must | P2 |
| 23 | Legacy memory migration | Must | P1 |
| 24 | Backward-compatible subsystem integration | Must | P1 |
| 25 | API evolution & forward-compatible serialization | Must | P1 |
| 26 | Performance budgets & regression gates | Must | P2 |

---

## Requirements

### Requirement 1: Single transactional authority & immutable event log

**User Story:** As a KRIA engineer, I want one SQLite database to be the sole
transactional authority with an immutable append-only event log, so that the system
has one consistent source of truth and every other store can be rebuilt from it.

**Design refs:** L1, L2, L4; design §5, §9, §11, §14, §16. **Correctness:** CP-1, CP-2,
CP-3, CP-5.

#### Acceptance Criteria

1. WHEN any durable state is written THE SYSTEM SHALL mutate the SQLite authority
   within exactly one ACID transaction (`AuthorityTx`) that commits the event, derived
   memory, graph changes, and outbox entries together.
2. THE SYSTEM SHALL expose no code path that writes to the vector index or full-text
   index except the outbox relay.
3. WHEN an event is appended THE SYSTEM SHALL treat its row as immutable, and any
   attempt to UPDATE or DELETE an `events` row SHALL be aborted by a database trigger.
4. WHEN an event is appended THE SYSTEM SHALL assign it a UUID v7 identifier, a hybrid
   logical clock (HLC) value, a UTC timestamp, an originating timezone offset, and a
   BLAKE3 checksum of its payload.
5. IF the same event id is appended more than once THEN THE SYSTEM SHALL treat the
   repeated append as a no-op (idempotent).
6. WHEN a derived index (vector or full-text) is lost or corrupted THE SYSTEM SHALL
   rebuild it from the authority such that retrieval results are equivalent to the
   pre-loss state.
7. THE SYSTEM SHALL NOT regenerate LLM-derived memory *content* from the event log;
   event replay rebuilds indexes only, never memory identity.
8. WHEN events exceed the hot-retention window (default 90 days) THE SYSTEM SHALL roll
   them into immutable, checksummed cold segments that remain queryable.

### Requirement 2: Memory Write Policy Engine (single write gate)

**User Story:** As a KRIA engineer, I want every subsystem to write only through one
mandatory policy engine, so that governance, quality, security, and privacy are
enforced at a single auditable choke point.

**Design refs:** L3; design §5, §8.1, §18. **Correctness:** CP-2, CP-6.

#### Acceptance Criteria

1. THE SYSTEM SHALL require every subsystem to submit a `WriteCandidate` to the Write
   Policy Engine, and SHALL provide no other path to durable state.
2. WHEN a `WriteCandidate` is submitted THE SYSTEM SHALL run the fast path
   synchronously — mode check, ownership/namespace assignment, deterministic security
   scan, and atomic append of the raw event plus outbox seed — without invoking an LLM
   or embedder.
3. THE SYSTEM SHALL complete the fast path within a 2 ms p95 latency target on
   reference hardware.
4. WHEN the fast path commits THE SYSTEM SHALL enqueue the event for the best-effort
   slow path (embed → dedup → contradiction → classify → importance → provenance →
   graph → commit derived memory) and return a decision to the caller.
5. WHEN the slow path detects a duplicate by vector similarity THE SYSTEM SHALL update
   the existing memory (reconsolidate) rather than create a new row.
6. IF a candidate proposes promoting a "rule" from insufficient or correlated evidence
   THEN THE SYSTEM SHALL reject the write via the false-promotion guard and record the
   rejection in the audit log.
7. IF a candidate is marked `sensitivity=secret` or high-impact THEN THE SYSTEM SHALL
   return `NeedsConfirmation` and hold the write until the user approves.
8. WHEN any Write Policy decision is made (stored, rejected, deduped, batched) THE
   SYSTEM SHALL record it with its reason in the memory-audit log.

### Requirement 3: Memory modes

**User Story:** As a user, I want to control how KRIA remembers via always-visible
memory modes, so that I decide when and what is persisted.

**Design refs:** ADR-013; design §6, §23. **Correctness:** CP-7.

#### Acceptance Criteria

1. THE SYSTEM SHALL support the modes Permanent, Temporary, Incognito, Workspace,
   Library-only, Read-only, Guest, Developer, Benchmark, Safe, and Research.
2. THE SYSTEM SHALL enforce the active mode's write decision at the fast-path gate
   using a deterministic decision table.
3. WHILE a session is active THE SYSTEM SHALL allow the user to switch mode
   mid-session and SHALL emit a `mode_switched` boundary event on each switch.
4. THE SYSTEM SHALL always surface the current mode to the UI.
5. WHEN the user switches to Incognito mid-session THE SYSTEM SHALL NOT retroactively
   delete memories already written in that session.
6. WHILE in Workspace mode THE SYSTEM SHALL reject personal-scope writes and allow only
   workspace-scoped writes.
7. WHILE in Safe mode THE SYSTEM SHALL allow only deterministic (no-LLM) writes and
   restrict retrieval to vector plus full-text.

### Requirement 4: Temporary and Incognito non-persistence

**User Story:** As a privacy-conscious user, I want Temporary and Incognito modes to
guarantee nothing durable is written, so that sensitive conversations leave no trace.

**Design refs:** design §18.1, §23. **Correctness:** CP-7.

#### Acceptance Criteria

1. WHILE in Incognito mode THE SYSTEM SHALL persist zero durable rows and hold session
   state in RAM only.
2. WHILE in Temporary mode THE SYSTEM SHALL tag writes as session-scoped and SHALL
   allow retrieval of them only during the current session.
3. WHEN a Temporary session ends THE SYSTEM SHALL hard-delete all session-scoped
   memories and their vectors via the deletion cascade.
4. WHILE in Read-only mode THE SYSTEM SHALL reject all writes and allow full retrieval.

### Requirement 5: Selective write filtering (quality)

**User Story:** As a user, I want KRIA to store what matters and reject noise, so that
memory stays high-signal rather than bloated.

**Design refs:** design §18.2 (quality filter). **Correctness:** —

#### Acceptance Criteria

1. WHEN the slow path evaluates an event THE SYSTEM SHALL reject noise (failed retries,
   cancelled actions, debugging spam, transient errors) and route it to the execution
   log only.
2. WHEN a candidate is rejected by the quality filter THE SYSTEM SHALL record the
   rejection and its reason in the memory-audit log.
3. THE SYSTEM SHALL apply the quality filter deterministically without requiring an
   LLM.

### Requirement 6: Truth Maintenance System

**User Story:** As a user, I want KRIA to never confidently rely on outdated or
contradicted knowledge, so that its answers stay correct over time.

**Design refs:** design §12, §22. **Correctness:** CP-14.

#### Acceptance Criteria

1. THE SYSTEM SHALL assign every memory a staleness class (Immutable, Permanent, Slow,
   Volatile-Verifiable, Volatile-Unverifiable) that governs re-verification rather than
   deletion.
2. WHEN a new fact contradicts an existing memory THE SYSTEM SHALL resolve it using the
   deterministic order: user-stated beats inferred, then more-recently-verified beats
   stale, then higher Memory Worth beats lower, else keep both as competing beliefs and
   surface to the user.
3. WHEN one memory supersedes another THE SYSTEM SHALL move the superseded memory to
   version history (state = Superseded) and SHALL NOT destroy it.
4. WHEN a memory carrying a `verify_against` predicate is retrieved THE SYSTEM SHALL
   re-check it against its source, and IF the source changed THEN THE SYSTEM SHALL
   demote its confidence and flag it stale rather than serve it as current.
5. IF the verification source is unavailable THEN THE SYSTEM SHALL mark the memory
   "unverified" and SHALL NOT assert a stale value as current.
6. WHEN a Volatile-Unverifiable memory (e.g. a mood or transient intent) is retrieved
   THE SYSTEM SHALL surface it with low confidence and a timestamp and SHALL NOT assert
   it as a current fact.
7. WHEN a contradiction is detected THE SYSTEM SHALL reduce the affected memory's
   confidence regardless of its prior value.

### Requirement 7: LLM/embedding-independent graceful degradation

**User Story:** As an offline user, I want core memory to keep working with no LLM, no
GPU, no embedder, and no network, so that KRIA is reliable everywhere.

**Design refs:** L8; design §18.2, §19 (degradation ladder). **Correctness:** CP-16.

#### Acceptance Criteria

1. WHILE the embedder is unavailable THE SYSTEM SHALL store the raw text, queue it for
   later embedding, and keep full-text keyword search functional.
2. WHILE the LLM is unavailable THE SYSTEM SHALL use deterministic heuristic extraction
   and classification, queue consolidation and reflection, and leave storage and
   retrieval functional.
3. WHILE the vector index is unavailable THE SYSTEM SHALL return retrieval results from
   the remaining strategies (full-text and graph).
4. WHEN any optional dependency (LLM, embedder, vector index) is unavailable THE SYSTEM
   SHALL NOT panic and SHALL NOT lose data.

### Requirement 8: Consent-gated cold start

**User Story:** As a new user, I want to consent before KRIA scans my system, so that I
control what is ingested from first run.

**Design refs:** design §36 (R7 note, architecture Issue 8). **Correctness:** —

#### Acceptance Criteria

1. WHEN KRIA is run for the first time THE SYSTEM SHALL display a consent screen before
   performing any filesystem, git, or shell scan.
2. IF the user does not grant scan consent THEN THE SYSTEM SHALL default to onboarding
   questions only and perform no scan.
3. WHEN a cold-start scan is permitted THE SYSTEM SHALL let the user preview and delete
   scan results before they are committed to memory.

### Requirement 9: Library ingestion & per-item erasure

**User Story:** As a user, I want to ingest documents into a personal library and
delete any item completely, so that my reference knowledge is managed and reversible.

**Design refs:** design §8.8, §14, §21.1. **Correctness:** CP-10.

#### Acceptance Criteria

1. WHEN a document is ingested THE SYSTEM SHALL stream it (never fully loading large
   files into RAM), chunk it adaptively, store the original on the filesystem, and
   register item and chunk metadata in the authority.
2. WHEN a duplicate document is ingested THE SYSTEM SHALL detect it by SHA-256 and avoid
   storing a redundant copy.
3. WHEN library ingestion is interrupted THE SYSTEM SHALL resume from a checkpoint on
   restart without corrupting partial state.
4. WHEN a fact is extracted from a document THE SYSTEM SHALL tag it with provenance
   `source: library:{item}:chunk:{idx}`.
5. WHEN a library item is deleted THE SYSTEM SHALL cascade-delete its file, chunks,
   vectors, and derived memories, and SHALL flag any dependent memories as
   `source_deleted` for the user to keep or cascade.
6. WHEN a new version of a library item is ingested THE SYSTEM SHALL append it linked to
   the previous version and SHALL NOT lose the old version.

### Requirement 10: Right to be forgotten (crypto-shred + cascade)

**User Story:** As a user, I want a forget command that makes memories permanently
unrecoverable, so that my right to erasure is genuinely honored.

**Design refs:** L9, ADR-006; design §21.1, §29. **Correctness:** CP-10.

#### Acceptance Criteria

1. WHEN the user issues `forget(scope)` THE SYSTEM SHALL tombstone the targeted
   memories (state = Forgotten) reversibly for 30 days and record a `memory_forgotten`
   event.
2. WHEN the 30-day window elapses or the user requests immediate hard deletion THE
   SYSTEM SHALL cascade-delete the memories across all stores and destroy the
   associated per-subject shred key.
3. WHEN a subject's shred key is destroyed THE SYSTEM SHALL render its encrypted event
   payloads permanently unreadable without mutating the immutable event log.
4. WHEN a hard delete completes THE SYSTEM SHALL ensure no retrieval (vector, full-text,
   or graph) returns content derived from the deleted subject.
5. WHEN a hard delete completes THE SYSTEM SHALL leave no orphan vector, graph edge, or
   library chunk after the next reconciliation sweep.
6. BEFORE a bulk deletion THE SYSTEM SHALL offer export-before-delete, and THE SYSTEM
   SHALL warn that crypto-shredded data is unrecoverable by design.

### Requirement 11: Backup & restore (authority-only)

**User Story:** As a user, I want reliable backups I can restore, so that I never lose
my memory to corruption or hardware failure.

**Design refs:** D-12; design §30. **Correctness:** CP-3.

> **Dev-context note:** **Future-only** — not implemented in the current single-laptop
> build (data loss acceptable; "copy the SQLite file" suffices). Design retained for when
> KRIA leaves dev (§47.6). The authority-only, indexes-rebuild-on-restore design (D-12)
> is validated and ready to build later.

#### Acceptance Criteria

1. WHEN a backup runs THE SYSTEM SHALL back up only the SQLite authority and the outbox
   cursor, and SHALL NOT back up the vector or full-text indexes.
2. THE SYSTEM SHALL write backups atomically (temp file plus rename) and SHALL verify a
   BLAKE3 checksum before marking a backup valid.
3. THE SYSTEM SHALL encrypt every backup.
4. THE SYSTEM SHALL make each backup self-describing by embedding a schema snapshot and
   a format version.
5. WHEN a backup is restored THE SYSTEM SHALL forward-migrate an older format, replay
   the outbox, rebuild the derived indexes, and produce retrieval results identical to
   the backed-up state.
6. THE SYSTEM SHALL support selective restore by namespace and time range.
7. THE SYSTEM SHALL periodically perform an automated test-restore to verify backup
   validity.

### Requirement 12: Derived-index consistency (outbox + reconciliation)

**User Story:** As a KRIA engineer, I want derived indexes to converge with the
authority even after crashes, so that vector and full-text search never silently drift.

**Design refs:** D-5, D-16; design §25, §14 (outbox). **Correctness:** CP-4.

#### Acceptance Criteria

1. WHEN a memory change requires an index update THE SYSTEM SHALL enqueue an outbox
   entry inside the same authority transaction as the memory change.
2. THE SYSTEM SHALL maintain a per-index cursor so each index (vector, full-text)
   replays independently.
3. WHEN the outbox relay applies an entry THE SYSTEM SHALL make the operation
   idempotent, keyed by `(memory_id, index_target, content_hash)`.
4. IF an outbox entry exceeds its retry budget THEN THE SYSTEM SHALL move it to a
   dead-letter state and repair it during reconciliation rather than retry indefinitely.
5. THE SYSTEM SHALL run a reconciliation sweep on a schedule that repairs referential
   integrity against the authority — purging orphan vectors, removing dangling edges,
   deleting parentless chunks, and shredding unused keys.

### Requirement 13: Retrieval quality at scale (release gate)

**User Story:** As a user with years of memories, I want retrieval quality to stay high
as the memory bank grows, so that KRIA stays useful long-term.

**Design refs:** L12; design §19 (candidate gating), §35 (scale benchmark).
**Correctness:** CP-17.

#### Acceptance Criteria

1. WHEN retrieval runs THE SYSTEM SHALL classify the query deterministically, run vector,
   full-text, graph, temporal, and goal-context strategies, and fuse them with adaptive
   Reciprocal Rank Fusion.
2. WHEN fusing candidates THE SYSTEM SHALL exclude superseded and archived memories and
   gate the pool by importance and Memory Worth so signal does not degrade as the bank
   grows.
3. THE SYSTEM SHALL fill a token budget by relevance rather than returning a fixed
   top-K.
4. THE SYSTEM SHALL pass a scale benchmark at 500,000 synthetic memories where Recall
   Precision meets or exceeds the baseline threshold; failing this benchmark SHALL block
   release.
5. THE SYSTEM SHALL report retrieval p95 latency and Recall Precision as metrics over
   time so regressions are detectable.

### Requirement 14: Crash safety & recovery

**User Story:** As a user, I want no data loss on crash or power failure, so that I can
trust KRIA with important information.

**Design refs:** L10; design §18.1, §30. **Correctness:** CP-6.

#### Acceptance Criteria

1. WHEN the fast path acknowledges a write THE SYSTEM SHALL have already committed the
   raw event durably, independent of embedder or LLM availability.
2. WHEN the process is killed or power is lost mid-operation THE SYSTEM SHALL recover on
   restart via SQLite WAL replay and idempotent outbox drain with zero authority data
   loss.
3. WHEN a crash leaves a session open THE SYSTEM SHALL detect open sessions younger than
   24 hours on startup and offer to resume them.
4. WHEN KRIA starts THE SYSTEM SHALL run an integrity check (SQLite quick-check, vector
   index open-verify, cold-segment checksum tail) and offer repair on failure.
5. IF the SQLite authority is corrupted THEN THE SYSTEM SHALL restore from the last good
   daily backup.

### Requirement 15: Merge / split atomicity

**User Story:** As a KRIA engineer, I want merge and split to be atomic and reversible,
so that consolidating memories never leaves the stores inconsistent.

**Design refs:** D-17; design §21.2. **Correctness:** CP-11.

#### Acceptance Criteria

1. WHEN two memories are merged THE SYSTEM SHALL apply all authority changes (memories,
   `derived_from`, graph edges, Memory Worth counters, outbox entries) in one
   transaction that either fully commits or fully aborts.
2. WHEN a merge or split completes THE SYSTEM SHALL preserve `derived_from` provenance
   to the originals and SHALL archive (not delete) the originals.
3. THE SYSTEM SHALL make merge and split reversible for 30 days via a tombstoned
   provenance record.
4. IF a crash occurs after the authority commit but before the index relay THEN THE
   SYSTEM SHALL converge the indexes via idempotent relay and reconciliation.

### Requirement 16: Feedback-driven learning

**User Story:** As a user, I want my feedback and corrections to improve future recall,
so that KRIA adapts to me over time.

**Design refs:** D-19; design §19, §22.3. **Correctness:** CP-13.

#### Acceptance Criteria

1. THE SYSTEM SHALL record feedback as a first-class event type with a signal taxonomy
   (thumbs up/down, correction, undo, cancel, edit, overwrite, ignored suggestion,
   repeated task, automation success/failure).
2. WHEN a task outcome is known THE SYSTEM SHALL update Memory Worth for the retrieved
   set by dividing credit across the set and adjusting for task difficulty.
3. THE SYSTEM SHALL let Memory Worth influence retrieval ranking and archival only after
   at least 20 observations, and SHALL NEVER let Memory Worth trigger a hard delete.
4. THE SYSTEM SHALL cap confidence gains from utility logarithmically so non-user-stated
   facts never reach confidence 1.0.

### Requirement 17: Explainability

**User Story:** As a user, I want to see why KRIA remembered or recalled something, so
that memory stays transparent and trustworthy.

**Design refs:** L6; design §28. **Correctness:** CP-9.

#### Acceptance Criteria

1. WHEN asked to explain a retrieval THE SYSTEM SHALL report the strategies used,
   per-strategy hits, fusion scores, gating decisions, budget allocation, and which
   memories were injected versus filtered and why.
2. WHEN asked to explain a memory THE SYSTEM SHALL report its provenance chain,
   `derived_from`, contradictions, Memory Worth, access history, staleness and
   verification history, and why it was stored.
3. THE SYSTEM SHALL produce a memory health report summarizing totals by type and
   staleness class, average confidence, knowledge gaps, low-worth memories, unresolved
   contradictions, pending LLM tasks, disk usage, and outbox lag per index.
4. THE SYSTEM SHALL provide a monthly "what KRIA believes about you" report with full
   provenance.

### Requirement 18: Encryption at rest

**User Story:** As a user, I want my memory encrypted at rest by default, so that a
stolen disk does not expose my data.

**Design refs:** design §9, §29, §47.3 (PII classifier). **Correctness:** —

> **Dev-context note:** AC 1–2 (app-level at-rest encryption) are **future-only** — the
> build relies on OS-level disk encryption on the single laptop. AC 3–6 (secret handling,
> PII classification, checksum) are **in the active build**.

#### Acceptance Criteria

1. (Future) THE SYSTEM SHALL encrypt the SQLite authority, the vector index directory,
   and all backups at rest by default.
2. (Future) THE SYSTEM SHALL NOT store the vector index at a weaker encryption tier than
   the SQLite authority.
3. THE SYSTEM SHALL classify content sensitivity with deterministic Tier-1 detectors
   (credentials/keys/tokens, financial, health, personal data) into
   `secret`/`private`/`public`, failing safe toward more-private on ambiguity, with a
   sticky user override.
4. WHEN content is marked `sensitivity=secret` THE SYSTEM SHALL never store its value
   (keychain reference + redacted placeholder) and SHALL omit its embedding so only
   keyword retrieval is possible.
5. THE SYSTEM SHALL never store passwords, API keys, or tokens, referencing them only
   via the OS keychain.
6. WHEN loading an embedding model THE SYSTEM SHALL verify its pinned checksum and
   refuse to load on mismatch.

### Requirement 19: Namespace & scope isolation

**User Story:** As a KRIA engineer, I want strict namespace and scope isolation, so that
plugins, workspaces, and agents cannot read or corrupt each other's memory.

**Design refs:** L7; design §13.5, §18.1, §19. **Correctness:** CP-8.

#### Acceptance Criteria

1. THE SYSTEM SHALL assign every memory a namespace, owner, and scope at write time.
2. WHEN retrieval runs THE SYSTEM SHALL filter by namespace, scope, and sensitivity so
   no returned memory has a scope outside the requester's scope or global, unless
   explicitly user-promoted.
3. THE SYSTEM SHALL enforce isolation both at the write gate and as a mandatory
   retrieval filter (defense in depth).
4. THE SYSTEM SHALL restrict an OpenClaw skill to a read-only view of its own namespace
   plus the public core, and SHALL require skills to write only via the orchestrator.
5. THE SYSTEM SHALL require user approval or a high evidence threshold before any
   plugin-originated memory is promoted to the core namespace.

### Requirement 20: Resource governance & bounded growth

**User Story:** As a laptop user, I want background cognition to respect battery, CPU,
memory, and disk limits, so that KRIA never drains or fills my machine.

**Design refs:** ADR-008; design §25, §30. **Correctness:** —

#### Acceptance Criteria

1. THE SYSTEM SHALL route all background work through the Cognitive Scheduler using
   priority classes P0 (foreground) through P4 (maintenance), with P0 always preempting.
2. WHILE on battery or power-saver THE SYSTEM SHALL suspend P3 and P4 background work.
3. WHILE memory pressure is high THE SYSTEM SHALL shed caches and defer P3 and P4 work.
4. THE SYSTEM SHALL chunk background writes (bounded rows and duration per transaction)
   and yield the writer between batches so foreground writes are not starved.
5. THE SYSTEM SHALL bound all queues so growth is never unbounded, applying
   backpressure (degrading to keyword-only) on overflow.
6. WHEN disk usage reaches 80% THE SYSTEM SHALL warn, and WHEN it reaches 95% THE SYSTEM
   SHALL aggressively archive.
7. WHEN a high-frequency source (file watcher, desktop context, GUI-automation loop)
   emits observations THE SYSTEM SHALL apply admission control (per-source debounce +
   coalesce-by-`(source, entity)` + bounded queue), SHALL NOT throttle
   `TriggerProvenance::User` writes, and SHALL always keep failures, contradictions, and
   user-flagged events even under drop-to-sample backpressure.

### Requirement 21: Cognitive layer (consolidation, dreaming, reflection)

**User Story:** As a user, I want KRIA to consolidate and reflect between interactions,
so that raw experience compresses into reusable, higher-quality knowledge.

**Design refs:** L11; design §11, §20. **Correctness:** CP-14 (self-trust).

#### Acceptance Criteria

1. THE SYSTEM SHALL trigger cognitive operations by activity triggers (idle > 30 min,
   session end, idle > 4 h / daily, weekly, backlog threshold, post-failure/success)
   rather than a fixed calendar.
2. WHEN consolidation produces a reflection THE SYSTEM SHALL re-submit it through the
   Write Policy Engine as untrusted `source: self_reflection` with confidence capped at
   0.6.
3. THE SYSTEM SHALL require a minimum number of supporting episodes before promoting a
   reflection to a rule, and SHALL reject a reflection that contradicts a user-stated
   fact.
4. THE SYSTEM SHALL cap reflection-of-reflection depth at 1 and treat compression level
   3 (Rule) as terminal.
5. WHEN a memory is compressed THE SYSTEM SHALL retain its source memories (archived,
   not deleted) so drift is detectable and correctable.
6. THE SYSTEM SHALL make consolidation runs idempotent (content-hash) and resumable from
   a checkpoint.

### Requirement 22: Embedding-version migration

**User Story:** As a KRIA engineer, I want embedding model upgrades handled as a safe
background migration, so that model obsolescence never breaks retrieval.

**Design refs:** C4, D-3; design §31.3. **Correctness:** CP-3.

#### Acceptance Criteria

1. THE SYSTEM SHALL store `model_name`, `model_version`, and `dimension` with every
   embedding and SHALL keep one vector table per model version.
2. THE SYSTEM SHALL never compare or mix vectors from different model versions.
3. WHEN an embedding model is upgraded THE SYSTEM SHALL create a new partition,
   dual-search old and new partitions during migration, and background re-embed
   oldest-first with rate limiting and checkpointing.
4. THE SYSTEM SHALL NOT drop an old vector table until all memories are re-embedded and
   verified.
5. THE SYSTEM SHALL cap concurrent model versions at two (current plus previous).
6. IF a re-embedding batch corrupts data THEN THE SYSTEM SHALL roll back via vector-index
   time travel.

### Requirement 23: Legacy memory migration

**User Story:** As an existing KRIA user, I want my current fragmented memory migrated
into the new authority without loss, so that the upgrade preserves my history.

**Design refs:** design §31.1. **Correctness:** CP-5.

#### Acceptance Criteria

1. WHEN the upgrade runs THE SYSTEM SHALL read each legacy store and emit a synthetic
   event into the new authority log for every legacy fact and document, tagged
   `source: migration:{legacy_db}`.
2. THE SYSTEM SHALL re-embed migrated content with the new model version and discard the
   legacy in-RAM vector index (rebuildable).
3. THE SYSTEM SHALL make the migration resumable so an interrupted migration continues
   on restart.
4. WHEN migration completes THE SYSTEM SHALL verify record counts and a sampled
   retrieval-parity check, and SHALL keep legacy files read-only until verification
   passes, then archive them.

### Requirement 24: Backward-compatible subsystem integration

**User Story:** As a KRIA engineer, I want the memory upgrade to slot into the existing
codebase without breaking current consumers, so that the migration is safe and
incremental rather than a big-bang rewrite.

**Design refs:** §45, §46, §47 (dev-scoped: hard cutover, no shim). **Correctness:**
IA-2, IA-6–IA-9, SI-1, SI-2, CP-8.

> **Dev-context note (steering `dev-context.md`, design §47.1):** single-laptop
> pre-production build → **hard cutover, no compatibility shim**. Consumers are rewritten
> directly onto `memory::api` and legacy modules are **deleted**. The `LegacyMemoryAdapter`
> is not built.

#### Acceptance Criteria

1. THE SYSTEM SHALL rewrite every current `Arc<dyn MemoryRuntime>` call site
   (`tools/registry.rs`, `tools/knowledge.rs`, `platform/telegram.rs`, desktop
   `voice.rs`/`sessions.rs`) directly onto `memory::api`, and SHALL delete the legacy
   memory modules (`store`, `manager`, `facts`, `decay`, `rag`, `vectors`, old
   `embeddings` path) once unreferenced.
2. WHEN a rewritten consumer performs a write THE SYSTEM SHALL route it through the Write
   Policy Engine so no path bypasses the write gate.
3. THE SYSTEM SHALL rewrite the `tools/knowledge.rs` knowledge tools directly onto the
   new Library and retrieval modules (no `RagEngine` facade).
4. THE SYSTEM SHALL wrap the existing `EmbeddingModel` (ONNX MiniLM) as the `Embedder`
   MiniLM tier, and SHALL NOT index hash-fallback vectors into the ANN index — when only
   the hash fallback is available, embeddings are treated as unavailable.
5. THE SYSTEM SHALL give OpenClaw skills only a read-only `SkillMemoryView` scoped to
   their namespace plus public core, and SHALL require skill-derived writes to flow
   through the orchestrator and Write Policy.
6. THE SYSTEM SHALL subscribe the Cognitive Scheduler to the existing automation
   `EventBus` for triggers and SHALL publish memory lifecycle events to that bus,
   without replacing `automation::scheduler`.
7. THE SYSTEM SHALL NOT change any existing Tauri command or event name.
8. WHEN any native tool, MCP tool, or skill completes THE SYSTEM SHALL memorize its
   outcome via the Write Policy Engine with a source-provenance tag
   (`tool:{name}` / `mcp:{server}:{tool}` / `openclaw:{skill}`), mapping
   `TriggerProvenance` (User / ExternalContent / Tool) to source reliability and the
   injection wall.
9. WHEN an MCP server is discovered or becomes unavailable THE SYSTEM SHALL record or
   demote (not delete) capability memories for its tools, so the Planner selects live
   tools while history is preserved.
10. WHEN `forget` targets a tool or MCP-server source THE SYSTEM SHALL cascade-delete
    that source's memories, vectors, and capability rows.

### Requirement 25: API evolution & forward-compatible serialization

**User Story:** As a KRIA engineer, I want a defined API versioning and serialization
policy, so that future changes never break existing callers or restored data.

**Design refs:** design §40. **Correctness:** —

#### Acceptance Criteria

1. THE SYSTEM SHALL expose the Memory API as a versioned module (`memory::api::v1`) with
   an `API_VERSION` constant.
2. WHEN a breaking change is required THE SYSTEM SHALL introduce a new version module
   that coexists with the prior version for at least one minor release or six months.
3. THE SYSTEM SHALL make persisted enums (event type, memory type, mode, staleness
   class, sensitivity, feedback signal) serialize as strings with an `Unknown(String)`
   fallback so an older binary reading newer data never panics and preserves unknown
   values on rewrite.
4. THE SYSTEM SHALL mark deprecated verbs with a deprecation notice, log their use once
   per process, and count them in `metrics()`.
5. THE SYSTEM SHALL tag every event payload with a schema version and dispatch on it so
   old payloads remain readable by new consumers.

### Requirement 26: Performance budgets & regression gates

**User Story:** As a KRIA maintainer, I want objective performance budgets enforced in
CI, so that performance regressions are caught automatically rather than in production.

**Design refs:** design §41. **Correctness:** CP-17.

#### Acceptance Criteria

1. THE SYSTEM SHALL meet the p95 performance budgets defined in design §41 on the
   reference hardware tier (fast-path ≤ 2 ms, retrieval ≤ 120 ms at 100K, cold startup
   ≤ 800 ms, graph 2-hop ≤ 5 ms, among others).
2. THE SYSTEM SHALL emit each budgeted metric from the evaluation harness and SHALL fail
   CI when a metric exceeds its documented failure threshold over a 3-run median.
3. WHILE on battery THE SYSTEM SHALL keep background CPU within budget by suspending P3
   and P4 work, and CI SHALL fail if P3/P4 run on battery.
4. THE SYSTEM SHALL report outbox lag and background CPU in the health report so
   operational breaches are observable.

---

## Traceability Summary

| Req | Design sections | Laws | Correctness properties |
|---|---|---|---|
| 1 | §5, §9, §11, §14, §16 | L1, L2, L4 | CP-1, CP-2, CP-3, CP-5 |
| 2 | §5, §8.1, §18 | L3 | CP-2, CP-6 |
| 3 | §6, §23 | L3 | CP-7 |
| 4 | §18.1, §23 | L3 | CP-7 |
| 5 | §18.2 | L3 | — |
| 6 | §12, §22 | TMS | CP-14 |
| 7 | §18.2, §19 | L8 | CP-16 |
| 8 | §36 (R7) | — | — |
| 9 | §8.8, §14, §21.1 | L5 | CP-10 |
| 10 | §21.1, §29 | L9 | CP-10 |
| 11 | §30 | L2, L4 | CP-3 |
| 12 | §14, §25 | L4 | CP-4 |
| 13 | §19, §35 | L12 | CP-17 |
| 14 | §18.1, §30 | L10 | CP-6 |
| 15 | §21.2 | L2 | CP-11 |
| 16 | §19, §22.3 | L12 | CP-13 |
| 17 | §28 | L6 | CP-9 |
| 18 | §9, §29 | — | — |
| 19 | §13.5, §18.1, §19 | L7 | CP-8 |
| 20 | §25, §30 | — | — |
| 21 | §11, §20 | L11 | CP-14 |
| 22 | §31.3 | — | CP-3 |
| 23 | §31.1 | L4 | CP-5 |
| 24 | §45, §46 (ADR-014) | L2, L3, L7 | IA-1–IA-9, SI-1, SI-2, CP-8 |
| 25 | §40 | — | — |
| 26 | §41 | L12 | CP-17 |
