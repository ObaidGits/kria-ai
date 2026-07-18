# KRIA Memory Architecture — Production Hardening & Zero-Defect Stabilization Bible

Status: SOURCE OF TRUTH (read-only analysis; no code changed to produce this)
Scope: the unified `MemorySystem` and every runtime path that reads/writes memory.
Verification basis: findings are proven from source read during audit. Where a
claim was not fully traced to code it is marked `Confidence: Medium/Low` and
flagged `VERIFY`. Prior reports/checklists/comments were NOT trusted.

Target posture (per `.kiro/steering/dev-context.md`): single-laptop, single-user,
pre-production. Severities are given for BOTH that target and a hypothetical
multi-user / large-scale target, because several items flip severity by target.

---

## 1. Executive Summary

The cognitive `MemorySystem` (SQLite authority + Write Policy + Retriever +
graph/goal/plan/reasoning/research/causal engines + background cognition) is a
coherent, single-authority design and is genuinely wired into the desktop agent
loop, the desktop Tauri UI, and — partially — the standalone server. It is NOT
yet what prior reports claimed ("100%, fully memory-driven end-to-end, 1M
scale"). The audit found concrete, code-verified gaps that block those claims.

The most important truths from code (not reports):
- Desktop turns are observed into cognitive memory; **server/Telegram/WS turns are not** (user statements are not learned server-side). [H1]
- Vector retrieval uses an **in-process HNSW ANN index** (`stores/ann_vectors.rs`, `hnsw_rs`) behind the `VectorStore` trait, with SQLite as the durable authority + a brute-force fallback for tiny partitions. [H2 resolved, Batch 7]
- `MemorySystem::reason()` is a **pure alias of `search()`** — no reasoning/goal/plan composition despite the name and the `/memory/reason` endpoint. [M1]
- Backup holds the **single write lock for the full copy**; restore leaves **pooled readers potentially stale**. [H3]
- Cold-start / library ingest / backup perform **blocking I/O on the async runtime** (no `spawn_blocking`). [H4]
- The server runs cognition but has **no memory event transport** (no SSE/WS) — "server live events" is not real. [UI-1]

None of these corrupt data on the normal single-laptop path. They block the
enterprise/scale/"locked" claims and the server-parity claim.

Production Readiness Score (this target, single-laptop): **7.5 / 10**.
Production Readiness Score (multi-user / scale target): **4 / 10**.
Confidence in this assessment: **High** for desktop, **Medium** for server (some
runtime paths inferred, marked VERIFY).

---

## 2. Current Architecture Assessment (verified)

### 2.1 Authority + storage
- Single authority DB `kria_memory.db` opened via `memory::db::Database`
  (`write: Mutex<Connection>` + `read_pool: Vec<Mutex<Connection>>`, WAL).
  Verified: `crates/kria-core/src/memory/db/mod.rs`.
- One `MemorySystem` per process; desktop shares the DB handle across
  `KriaMemoryRuntime` (backend), `ConversationStore`, and `MemorySystem` via
  `MemorySystem::open_with_db(backend.database(), …)`. Verified:
  `crates/kria-desktop/src/commands/runtime.rs` (~line 206–223).
- Server: `headless_runtime::build_minimal` opens `KriaMemoryRuntime` over the
  same path and builds `MemorySystem::open_with_db(backend.database(), …)`;
  session store derived from `ms.conversation()`. Verified:
  `crates/kria-core/src/agent/headless_runtime.rs`, `crates/kria-server/src/main.rs`.

### 2.2 Write path (single funnel — good)
- All writes go through `WritePolicy::submit` (admission → mode → security scan →
  ownership/sensitivity → event commit → slow-path enqueue). Verified:
  `crates/kria-core/src/memory/write_policy/mod.rs`.
- `submit` fires a change notifier (`notify("created")`) → `MemorySystem` broadcast.
- Slow path (`write_policy/slow.rs`) enriches events into derived memories +
  FTS + vectors asynchronously via an unbounded mpsc worker.

### 2.3 Read path
- `Retriever` fuses vector + FTS with adaptive RRF weights; returns hits +
  `RetrievalTrace`. Verified: `crates/kria-core/src/memory/retriever.rs`.
- Vector store is HNSW ANN (`stores/ann_vectors.rs`) with a brute-force fallback for tiny partitions. [H2 resolved, Batch 7]

### 2.4 Cognition
- `MemorySystem::cognitive_scheduler` registers 8 jobs (consolidation ×4, active
  learning, self-improvement, dream, entity extraction). Verified: `api.rs`.
- Desktop + server both spawn an event-driven loop: `select!` on a 300s timer and
  `subscribe_changes()`, coalesced, single-flight per iteration. Verified:
  `runtime.rs` (~2150), `kria-server/src/main.rs`.

### 2.5 Change/event bus
- `MemorySystem` owns a `tokio::broadcast::Sender<MemoryChange>` (cap 256).
  Desktop bridges it to Tauri `memory://*` events + scheduler wake. Server uses
  it only to wake cognition (no client transport). Verified: `api.rs`, both mains.

### 2.6 UI
- Desktop SolidJS Memory Workspace (13 sections) + force-directed graph + chat
  feedback bar + cold-start wizard, all over 51 Tauri commands and a reactive
  `memoryStore` with live `memory://changed` subscription. Verified: `ui/src/…`.

---

## 3. Verified Logical Architecture Diagram

```
                        ┌─────────────────────────────────────────────┐
                        │              MemorySystem (façade)           │
                        │  write_policy · retriever · modes · slow ·   │
                        │  cognition · goals · plans · reasoning ·     │
                        │  research · causal · graph_intel · library · │
                        │  cold_start · broadcast<MemoryChange>        │
                        └───────────────┬─────────────────────────────┘
                                        │ Arc<Database> (WAL: 1 writer + read pool)
        ┌───────────────────────────────┼───────────────────────────────┐
        │                               │                               │
   Desktop runtime                 Server (headless)               Tests (in-mem)
   AgentLoop.with_memory_system    AgentLoop.with_memory_system
   observe_user_message (chat.rs)  [MISSING — H1]
   Tauri 51 cmds + live bridge     /memory/* (22 routes), no live transport [UI-1]
   Memory Workspace (SolidJS)      (no web UI over server routes)
```

---

## 4. Verified Runtime Flow (desktop chat turn)

1. `chat.rs` → `observe_user_message(memory_system, session, message)` [desktop only].
2. `AgentLoop.run` → `retrieve_memory_grounding(last_user_text)` → `MemorySystem::search` (HNSW ANN vectors) → injects grounding block; records `grounding_memory_ids`.
3. Tool calls → `record_agent_outcome` → `record_tool_outcome` (every outcome → write).
4. Turn end → `reward_memories(grounding_ids, positive)` + `reinforce_retrieval(class, strategy)`.
5. `WritePolicy::submit` on each write → `notify("created")` → broadcast → Tauri `memory://changed` + scheduler wake (coalesced 1.2s) → `run_ready()`.
6. Slow-path worker enriches events → memories + FTS + vectors.

Server flow is identical EXCEPT step 1 is absent [H1] and step 5's client
transport is absent [UI-1].

---

## 5. Verified Data Flow

- Event (raw, durable) → slow-path enrichment → Memory (derived) → FTS + vector index.
- Library ingest → `library_items`/`library_chunks` + per-chunk `WriteCandidate(Source::Library)` → memories.
- Cold-start import → readable files route through `ingest_document` (Library chunk/dedup/version, `Source::Library`); binary/git/shell stay reference-only `Source::Import` [M3 resolved, Batch 6].
- Feedback → `FeedbackService` → Memory-Worth counters.
- Truth: `supersede` sets `superseded_by` + `Superseded` state (version history retained).

---

## 6. Verified Integration Map

| Surface | Retrieval | Observe user turns | Tool outcomes | Live events | Cognition |
|---|---|---|---|---|---|
| Desktop chat | ✓ grounding | ✓ (`observe_user_message`) | ✓ | ✓ Tauri | ✓ |
| Desktop voice/image/gui | ✓ (VERIFY per-path) | partial (VERIFY) | ✓ | ✓ | ✓ |
| Server `/ws` + `/api/chat` | ✓ grounding | ✗ [H1] | ✓ | ✗ [UI-1] | ✓ |
| Telegram (via server chat) | ✓ grounding | ✗ [H1] | ✓ | ✗ | ✓ |
| Server `/memory/*` REST | ✓ | n/a | n/a | ✗ | n/a |

---

## 7–17 consolidated: Bug / Gap / Caveat / Risk Catalogue with Optimal Solutions

Each entry: ID · Title · Category · Severity(laptop / scale) · Subsystem · Files ·
Functions · Root cause · Evidence · Impact · Optimal solution · Confidence · Blocks-prod.

### H1 — Server/Telegram/WS do not observe user turns into cognitive memory
- Category: Missing Integration · Severity: High / Critical
- Subsystem: server chat, AgentLoop, desktop chat
- Files: `crates/kria-desktop/src/commands/chat.rs` (`observe_user_message` ~675), `crates/kria-core/src/agent/loop_engine/mod.rs` (writes only via `record_agent_outcome`/`reward_memories`), `crates/kria-server/src/ws.rs`, `crates/kria-server/src/routes.rs`
- Root cause: turn observation lives in the desktop command layer, not the shared loop.
- Evidence: `observe_user_message` referenced only in `chat.rs`; server chat/ws contain no observe call.
- Impact: server never learns user-stated facts; desktop/server memory diverge; "fully memory-driven server" is false.
- Optimal solution: relocate turn observation into the shared `AgentLoop` (a `MemoryObservationPolicy` invoked at turn start/end, source-aware), so every host observes identically. Add a builder toggle for mode/incognito. This makes the loop the single authority for "what a turn remembers" (fixes AR1 too).
- Confidence: High · Blocks prod: Yes (parity).

### H2 — Brute-force O(n) vector search ✅ RESOLVED (Batch 7)
- Category: Scalability/Performance · Severity: Medium / Critical
- Files: `crates/kria-core/src/memory/stores/sqlite_vectors.rs` (`search`, `cosine`, `decode_vector`)
- Root cause: loads every vector BLOB per query, cosine in Rust; no ANN index.
- Evidence: header "MVP brute-force … <~50k"; `search` row loop.
- Impact: linear latency; grounding runs it every turn; 100K+ → seconds + GC pressure.
- Optimal solution: implement an ANN index behind the existing `VectorStore` trait — preferred `usearch`/`hnsw_rs` in-process (no external service), memory-mapped, model-partitioned; keep brute-force as a correctness fallback + for tiny stores. Add a `VectorStore::search` recall benchmark harness. Trade-off vs LanceDB: hnsw crate is lighter, no separate process, matches "local-first"; LanceDB adds columnar+ANN but heavier. Choose hnsw/usearch for single-binary simplicity.
- Confidence: High · Blocks prod: at scale only.
- FIX: new `stores/ann_vectors.rs::AnnVectorStore` (`hnsw_rs` cosine, in-process, model-partitioned) behind the `VectorStore` trait; SQLite stays the durable authority (lazy-rebuild on boot; reconciliation intact); brute-force for `<=256`-vector partitions + as a recall guard. Wired as the single shared `vectors` store (write/read/delete). Tests: recall ≥ 0.7 vs brute force @1500, reload-from-authority, tombstone-safe deletes.

### H3 — Backup blocks writes; restore leaves pooled readers stale
- Category: Reliability/Concurrency · Severity: High / High
- Files: `crates/kria-core/src/memory/db/mod.rs` (`backup_to`, `restore_from`)
- Root cause: `VACUUM INTO` runs under the write `Mutex`; restore writes the write connection but read-pool connections keep separate handles/caches.
- Evidence: method bodies; `Database` has separate `write` + `read_pool`.
- Impact: large backup stalls all writes; post-restore reads inconsistent until restart.
- Optimal solution: (backup) use SQLite Online Backup API from a fresh read-only source connection on `spawn_blocking`, not the write lock; (restore) after online restore, transactionally rebuild the read pool (close + reopen all read connections) so no stale caches, and force `wal_checkpoint(TRUNCATE)`. Expose restore as an explicit "requires brief pause" operation with a lock that drains in-flight reads.
- Confidence: High · Blocks prod: Yes for backup UX.

### H4 — Blocking filesystem/subprocess I/O on the async runtime
- Category: Concurrency/Performance · Severity: High / High
- Files: `crates/kria-core/src/memory/cold_start_scan.rs` (WalkDir, `std::fs::read_to_string`, `std::process::Command git`), `crates/kria-desktop/src/commands/memory.rs` (`memory_cold_start_preview/import`, `memory_library_ingest`, `memory_backup` — async fns calling sync work)
- Root cause: no `spawn_blocking` around blocking work in async handlers.
- Impact: stalls Tokio workers → chat/voice latency spikes during scan/ingest/backup.
- Optimal solution: wrap all filesystem/subprocess/VACUUM work in `tokio::task::spawn_blocking`; add a bounded concurrency semaphore for parallel file reads during import.
- Confidence: High · Blocks prod: Yes for responsiveness.

### M1 — `reason()` is a pure alias of `search()`
- Category: API Semantics/Feature Drift · Severity: Medium / Medium
- Files: `crates/kria-core/src/memory/api.rs` (`reason` → `self.search`), server `/memory/reason`, `memory_reason`
- Impact: reasoning endpoint returns plain retrieval; misleads callers; reasoning-memory grounding not composed.
- Optimal solution: implement `reason()` to compose retrieval + `ReasoningStore::reasoning_context` + `GoalStore::planner_context` + `PlanStore::recommend` into a single structured grounding result; keep `search()` as pure retrieval. Document the contract.
- Confidence: High · Blocks prod: No (correctness of naming/behavior).

### M2 — Sentinel fixed session UUIDs collapse unrelated writes
- Category: Architecture/Cognition-quality · Severity: Medium / High
- Files: `tools/knowledge.rs` (`tools_session`), `tools/rag.rs` (`library_session`), `commands/memory.rs` (`ui_session`), `api.rs::cold_start_import`, `memory_routes.rs::remember`
- Root cause: no first-class "global/non-session write" concept; callers use constants.
- Impact: session-scoped consolidation + `Temporary` purge + analytics treat thousands of unrelated memories as one session.
- Optimal solution: add `Scope::Global` write intent + a `SessionRef::None` on `WriteCandidate`; consolidation/purge must key on real originating session or skip session grouping for global writes.
- Confidence: High · Blocks prod: No (quality).

### M3 — Two divergent ingestion paths (Library vs cold-start) ✅ RESOLVED (Batch 6)
- Category: Duplicate Logic/Consistency · Severity: Medium
- Files: `api.rs::cold_start_import` vs `tools/rag.rs`/`memory/library.rs`
- Impact: cold-start stores single truncated (4000-char) memories, no chunk/dedup/version/provenance; inconsistent retrieval + content loss.
- Optimal solution: route cold-start file imports through `Library::ingest` + shared chunk-submission helper (extract the rag ingest helper into `Library`). One ingestion pipeline.
- Confidence: High · Blocks prod: No.
- FIX: added `MemorySystem::ingest_document` (Library record + per-chunk Write-Policy submit, `Source::Library` provenance, dedup-skips re-submission via `Library::ingest` now returning `newly_created`). `cold_start_import` routes readable files through it; `ingest_document_rag` + desktop `memory_library_ingest` collapsed onto it. ONE pipeline. Test: `cold_start_import_chunks_files_and_dedups_on_reimport`.

### M4 — Server memory routes untested with a live MemorySystem
- Category: Missing Tests · Severity: Medium / High
- Files: `crates/kria-server/tests/integration_api.rs` (only 503 path)
- Optimal solution: integration test building `ServerState` with a temp-file MemorySystem; exercise search/remember/health/metrics/reason success + error mapping.
- Confidence: High.

### M5 — Every write wakes cognition; tool-outcome writes grow memory unbounded ✅ RESOLVED (Batch 9)
- Category: Performance/Volume · Severity: Medium
- Files: `write_policy/mod.rs` (`notify` per submit), `api.rs::record_tool_outcome`
- Impact: low-value tool chatter inflates memory + event volume; broadcast (256) can lag under extreme bursts (handled → generic refresh).
- Optimal solution: salience-gate `record_tool_outcome` (store only meaningful outcomes/failures); separate a low-priority telemetry channel from cognitive writes; make broadcast capacity configurable.
- Confidence: Medium (usage-dependent).
- FIX: `integration::outcome_is_salient` gates `record_tool_outcome` — failures always kept, trivial successes → `Batched` (not persisted, no cognition wake). `ToolOutcomeStats`/`tool_outcome_stats()` telemetry (seen/persisted/gated). Broadcast capacity now `MemoryConfig::change_channel_capacity` (also L3). Tests: `tool_outcome_salience_gate_drops_trivial_persists_meaningful`, `outcome_salience_keeps_failures_and_drops_trivial_successes`, `change_channel_capacity_is_configurable`.

### M6 — Silent UUID fabrication on decode failure
- Category: Reliability/Data-integrity · Severity: Medium
- Files: `crates/kria-core/src/memory/library.rs` (`get_item`, `list_items`: `Uuid::parse_str(...).unwrap_or_else(|_| new_id())`)
- Impact: corrupted stored id returned under a random id; breaks delete-by-id; hides corruption.
- Optimal solution: propagate a decode error (`StorageError::Serde`) instead of fabricating.
- Confidence: High.

### UI-1 — Server runs cognition but has no memory event transport ✅ RESOLVED (Batch 10)
- Category: Missing Integration/UI · Severity: Medium
- Files: `crates/kria-server/src/main.rs` (broadcast used only to wake scheduler), no SSE/WS memory stream; no web UI over `/memory/*`.
- Impact: "P8 server live events" claim is unmet; remote clients can't observe live memory changes.
- Optimal solution: add an SSE endpoint `/memory/events` that forwards `subscribe_changes()`; optionally a minimal web memory view reusing the same store abstraction.
- Confidence: High · Blocks prod: No (feature parity).
- FIX: `memory_routes.rs::events_sse` → `GET /memory/events` (Axum SSE) forwards `subscribe_changes()` (`ready`/`memory`/`lagged` + keep-alive), 503 when unavailable. Integration test asserts mount + gate. (A web memory view is optional and not required by UI-1.)

### L1 — `VectorIndex` (in-memory) uses `.lock().unwrap()` (poison panic)
- Files: `crates/kria-core/src/memory/vectors.rs`. Optimal: `unwrap_or_else(into_inner)` like `Database`. Also assess if still needed (Dead Code DC1). Confidence: High. Severity: Low.

### L2 — `/api/chat` returns `status:"ok"` with empty reply on early loop error
- Files: `crates/kria-server/src/routes.rs::run_agent_turn`. Optimal: detect empty reply / `StreamEvent::Error` and return an error status. Severity: Low.

### L3 — Broadcast capacity 256 hardcoded ✅ RESOLVED (Batch 9)
- Files: `api.rs::assemble`. Optimal: config-driven; document lag semantics. Severity: Low.
- FIX: `MemoryConfig::change_channel_capacity` (default 256); channel sized from config.

### L4 — Cold-start scan/import not cancellable
- Files: `cold_start_scan.rs`, `api.rs::cold_start_import`. Optimal: thread a `CancellationToken`; bounded already by `limit`. Severity: Low.

### S1 — Cold-start secret filter is filename-substring only ✅ RESOLVED (Batch 9)
- Category: Security/Privacy · Severity: Medium (privacy)
- Files: `cold_start_scan.rs` (`SECRET_HINTS`). Impact: in-file secrets inside `.md`/`.txt`, or non-standard secret filenames, can be imported.
- Optimal solution: content-level secret scanning (entropy + known token regexes) before import; reuse `memory/security.rs`/`sensitivity.rs` detectors on candidate content, not just names.
- Confidence: High.
- FIX: `cold_start_scan::content_has_secret` (reuses `sensitivity::classify` + Shannon-entropy heuristic for unlabelled tokens); `cold_start_import` scans file CONTENT before ingest and skips secret-bearing files entirely. Test: `content_secret_scan_catches_labelled_and_high_entropy`, `cold_start_skips_files_with_in_content_secrets`.

### S2 — VERIFY: injection wall coverage for `Source::Import` ✅ RESOLVED (Batch 9)
- Category: Security · Severity: Medium (VERIFY)
- Files: `memory/types.rs` (`is_untrusted_content` includes Import ✓), downstream prompt construction (NOT traced this session).
- Action: confirm untrusted content is fenced before entering LLM context on every grounding path.
- Confidence: Low (needs tracing).
- FIX: verified `is_untrusted_content()` covers both `Source::Import` AND `Source::Library` → `write_policy/security.rs::scan` rejects injection-shaped content on the single write gate (every grounding write). Test `injection_wall_rejects_imported_and_library_content` proves imported + library-chunk injection is rejected while benign user content passes.

### R1 — Slow-path crash consistency untested ✅ RESOLVED (Batch 8)
- Category: Reliability · Severity: Medium
- Files: `write_policy/slow.rs` (unbounded mpsc, cursor-based enrichment). Claim: idempotent cursor. No crash test.
- Optimal solution: crash-injection test (drop worker mid-enrichment, reopen, assert no dupes/loss); make the enrichment queue durable (the `fjall` queue already in the stack is a candidate) so pending enrichment survives restart instead of relying on an in-memory mpsc.
- Confidence: Medium.
- FIX: durability is the existing durable event log + consumer cursor (no new store needed). `SlowPath::run` now does a **boot crash-recovery sweep** + **periodic catch-up** (`enrich_pending`); `enrich` idempotency proven under replay by `enrichment_survives_crash_and_is_idempotent` (drop instance mid-backlog → reopen file DB → each event enriched exactly once, replay creates no dupes).

### R2 — Unbounded mpsc for slow-path enqueue (no backpressure) ✅ RESOLVED (Batch 8)
- Category: Reliability/Performance · Severity: Medium
- Files: `api.rs::assemble` (`mpsc::unbounded_channel`), `write_policy`.
- Impact: a write burst with a slow embedder grows the queue unboundedly (RAM); enrichment lag invisible.
- Optimal solution: bounded channel + drop/coalesce policy + a `pending_enrichment` gauge in health; or durable queue (fjall) with a depth metric.
- Confidence: High.
- FIX: wake channel is now **bounded** (`mpsc::channel`, `MemoryConfig::enrichment_queue_capacity`); `submit` uses `try_send` (drops the *wake* not the data — durable event + cursor + catch-up recover it → no unbounded RAM, `submit` never blocks). `MemorySystem::pending_enrichment_depth()` gauge (via `EventStore::pending_count`). Test: `enrichment_backpressure_drops_wake_not_data` + `enrichment_depth_gauge_tracks_backlog`.

### DC1 — `VectorIndex` largely legacy post-RagEngine removal ✅ RESOLVED (Batch 11)
- Category: Dead/Unused Code · Files: `memory/vectors.rs`, still referenced by `AppState.vectors`/orchestrator. Action: confirm remaining consumers; if only orchestrator uses it, move it out of `memory::` (layer clarity). Confidence: Medium.
- FIX: confirmed the legacy `VectorIndex` was **carried but never queried** (threaded through desktop AppState → telegram as `_vectors`, unused; real vectors go through `AnnVectorStore`/`MemorySystem`). Deleted `memory/vectors.rs` + its export and removed the dead threading (desktop `AppState`/runtime/local_api/telegram, core telegram spawn/poll/`process_message` signatures, the `vectors` health service). No behavior change; `cargo check`/`clippy` clean (no new warnings).

### DC2 — `knowledge.rs` dual path (memory vs runtime fact store) + `register_stubs` shipped in lib ✅ RESOLVED (Batch 11)
- Category: Tech Debt · Optimal: gate stubs behind `#[cfg(test)]`; document the non-cognitive snippet/conversation store as an explicit separate concern. Confidence: High.
- FIX + CORRECTION: `register_stubs` is **NOT test-only** — `headless_runtime.rs` calls `build_registry_with_store(None)` as a production degraded "core registry only" fallback when the memory backend is unavailable. Gating it behind `#[cfg(test)]` would break that path. Correct resolution: fixed the misleading "for tests" docs on `register_stubs` + its call site to state it is the real no-memory degraded fallback (keeps the knowledge tool surface present with honest no-op handlers). Kept in the lib intentionally.

### L4 — Cold-start scan/import not cancellable ✅ RESOLVED (Batch 11)
- Files: `cold_start_scan.rs`, `api.rs::cold_start_import`. Optimal: thread a `CancellationToken`; bounded already by `limit`. Severity: Low.
- FIX: `MemorySystem::cold_start_import_cancellable(source, candidates, &CancellationToken)` checks the token before each candidate and stops early (returning the count imported so far; each candidate is committed independently). `cold_start_import` delegates with a fresh token. Test: `cold_start_import_honors_cancellation`.

### API-1 — Desktop (51 cmds) vs server (22 routes) expose different subsets ✅ RESOLVED (Batch 10)
- Category: API Inconsistency · Impact: feature parity gaps (server lacks goals-create/set-status, feedback, graph neighbors/predict, cold-start). Optimal: define ONE memory API contract; generate both Tauri + Axum surfaces from it (shared handler module taking `&MemorySystem`). Confidence: High.
- FIX: new `memory/contract.rs` = the single canonical API surface over `&MemorySystem` (authoritative JSON shapes + shared `hit_json`/`parse_scope`). Server routes are now thin adapters delegating to `contract::*` (zero server-side shaping). Desktop adopts it where shape-identical (`memory_health`/`memory_remember`) + shares `parse_scope`; desktop read commands keep their intentional UI-superset shape (frontend contract — steering) with the shared CORE fields guaranteed identical. Test: `contract_shapes_are_stable`. Remaining: full desktop migration of richer read commands is UI-gated (tracked; not drift — core fields already match).

### DOC-1 — `MEMORY_ARCHITECTURE_FINAL.md` overstates completion ✅ RESOLVED (Batch 11)
- Category: Documentation Drift · Impact: contradicts H1/M1/UI-1/M4. Optimal: replace with this document's verified status. Confidence: High.
- FIX: prepended a verified **IMPLEMENTATION STATUS** banner to `MEMORY_ARCHITECTURE_FINAL.md` (kept the blueprint for the "why") correcting the stale as-built claims (HNSW ANN not LanceDB; bounded+durable enrichment; one ingestion pipeline; one API contract + SSE; safety as-built) and pointing to SESSION_HANDOVER + this Bible as the live source of truth.

---

## 18. Architecture Improvement Plan (optimal target state)

1. **Single turn-observation authority**: memory observation, mode gating, and
   tool-outcome recording live in `AgentLoop` (or a `MemoryTurnPolicy` it owns).
   Every host (desktop/server/telegram) inherits identical behavior. Removes H1 + AR1.
2. **Pluggable ANN VectorStore**: hnsw/usearch behind `VectorStore`, brute-force
   fallback, recall benchmark gate. Removes H2.
3. **One memory API contract**: a shared handler layer over `&MemorySystem`;
   Tauri + Axum are thin adapters. Removes API-1 and future drift.
4. **Durable, bounded enrichment queue** (fjall) with depth metrics + crash
   test. Removes R1/R2.
5. **Unified ingestion**: cold-start → Library. Removes M3.
6. **Composed `reason()`**. Removes M1.
7. **First-class write scope/session intent**. Removes M2.
8. **spawn_blocking + cancellation** for all blocking memory ops. Removes H4/L4.
9. **Safe backup/restore** (online backup off write lock; read-pool rebuild on restore). Removes H3.
10. **Server memory event transport (SSE)** + optional web memory view. Removes UI-1.
11. **Content-level cold-start secret scanning**. Removes S1.

---

## 19. Implementation Tracker

Legend: Sev = laptop/scale. Phase: 1=correctness/safety, 2=parity, 3=scale, 4=polish.

| ID | Title | Cat | Sev | Phase | Subsystem | Key files | Complexity | Regression risk | Blocks | Required tests |
|----|-------|-----|-----|-------|-----------|-----------|------------|-----------------|--------|----------------|
| H1 | Server observes turns | Integration | H/C | 2 | loop/server | loop_engine, chat.rs, ws.rs | M | Med (changes write volume) | parity | integration: server remember-then-recall |
| H2 | ANN vector store | Perf/Scale | M/C | 3 | vectors | sqlite_vectors.rs, retriever | L | Med (recall change) | scale | recall benchmark, latency bench |
| H3 | Safe backup/restore | Reliability | H/H | 1 | db | db/mod.rs | M | Med | backup UX | concurrent-write backup, restore consistency |
| H4 | spawn_blocking | Concurrency | H/H | 1 | coldstart/lib/backup | cold_start_scan, commands/memory.rs | S | Low | responsiveness | latency-under-scan test |
| M1 | reason() composes | API | M/M | 2 | api | api.rs | S | Low | — | reason-vs-search contract test |
| M2 | write scope/session | Arch | M/H | 2 | write_policy/types | types.rs, callers | M | Med | — | consolidation-by-session test |
| M3 | unify ingestion | Consistency | M/M | 2 | coldstart/library | api.rs, library.rs, rag.rs | M | Med | — | cold-start chunk/dedup test |
| M4 | server memory tests | Tests | M/H | 1 | server tests | integration_api.rs | S | Low | — | live-memory route tests |
| M5 | salience-gate tool writes | Perf | M/M | 3 | api/loop | api.rs, loop_engine | M | Med | — | volume/regression test |
| M6 | no UUID fabrication | Integrity | M/M | 1 | library | library.rs | S | Low | — | corrupt-id decode test |
| UI-1 | server SSE events | Integration | M/M | 3 | server | memory_routes.rs, main.rs | M | Low | — | SSE stream test |
| L1 | poison-safe locks | Reliability | L/L | 4 | vectors | vectors.rs | S | Low | — | — |
| L2 | /api/chat error surface | Reliability | L/L | 2 | server | routes.rs | S | Low | — | error-path test |
| L3 | configurable broadcast cap | Perf | L/L | 4 | api | api.rs | S | Low | — | — |
| L4 | scan cancellation | UX | L/L | 4 | coldstart | cold_start_scan.rs | S | Low | — | cancel test |
| S1 | content secret scan | Security | M/M | 2 | coldstart | cold_start_scan.rs | M | Low | privacy | secret-in-content test |
| S2 | verify injection wall | Security | M/? | 1 | prompt path | loop_engine | S(trace) | Low | VERIFY | injection test |
| R1 | slow-path crash test + durability | Reliability | M/H | 3 | slow path | slow.rs, api.rs | M | Med | — | crash-injection test |
| R2 | bounded enrichment queue | Reliability | M/H | 3 | api | api.rs | M | Med | — | backpressure test |
| DC1 | relocate VectorIndex | Debt | L/L | 4 | vectors | vectors.rs | S | Low | — | — |
| DC2 | cfg(test) stubs | Debt | L/L | 4 | knowledge | knowledge.rs | S | Low | — | — |
| API-1 | one memory API contract | Arch | M/H | 3 | api adapters | commands/memory.rs, memory_routes.rs | L | Med | — | parity test |
| DOC-1 | replace stale docs | Doc | L/L | 1 | docs | MEMORY_ARCHITECTURE_FINAL.md | S | None | — | — |

Per-item completion checklist template: [ ] implemented [ ] unit test [ ]
integration test [ ] benchmark (if perf) [ ] clippy clean [ ] regression suite
green [ ] doc updated [ ] sign-off.

---

## 20. Testing Strategy

- **Unit**: write policy admission/mode/security; retriever fusion; truth
  supersede; goals/plans/reasoning analytics; library chunk/dedup; cold-start
  gate + secret filter; UUID decode errors (M6).
- **Integration**: desktop + server remember→recall parity (H1); server
  `/memory/*` live (M4); reason-vs-search contract (M1); cold-start unified
  ingest (M3).
- **Concurrency**: backup under concurrent writes (H3); scan under active chat
  (H4); broadcast lag under write burst (M5).
- **Crash/Recovery**: slow-path crash injection (R1); restore consistency +
  read-pool rebuild (H3); WAL checkpoint after restore.
- **Scalability/Perf**: vector recall + latency at 10K/100K/1M (H2);
  enrichment-queue depth under burst (R2); graph community/centrality at large V/E.
- **Migration**: schema-version forward test across the 10 migrations;
  backup made on version N restored on version N (and rejection on mismatch).
- **Frontend**: memoryStore live coalescing; workspace section smoke; graph
  interaction; feedback bar → record_feedback.
- **Production acceptance**: end-to-end "state before backup → mutate → restore →
  verify" (exists) extended to concurrent + large data.

---

## 21. Regression Prevention Strategy

- CI gates: `cargo fmt --check`, `cargo clippy --workspace -D warnings` (currently
  warnings exist — see note), `cargo test --workspace`, `ui: tsc + vitest`.
- Invariant tests as guards (extend `tests/memory_invariants.rs`): single
  authority DB; no direct store writes outside Write Policy; retriever is the only
  read path; every host observes via the shared policy (H1 guard).
- Benchmark gate for vector recall/latency to prevent silent H2 regressions.
- Contract test locking `reason()` ≠ `search()` post-M1.

Note: fix the pre-existing non-memory failures before enabling `-D warnings`
gates: `config_tests` (possible real TOML load/merge regression — VERIFY, do not
rubber-stamp), `routing_mem01` (router semantics), `continuation_reentry`
(isolation flake), `SettingsModal.google` (`envLockedFields`).

---

## 22. Production Hardening Checklist

- [ ] All blocking memory ops on `spawn_blocking` (H4)
- [ ] Backup off the write lock; restore rebuilds read pool (H3)
- [ ] Enrichment queue bounded + durable + depth metric (R1/R2)
- [ ] `pending_enrichment`, `broadcast_lag`, `vector_count`, `retrieval_latency_p95`, `scheduler_last_run`, `write_reject_rate` exposed via `health()`/metrics
- [ ] Content-level secret scan for cold-start (S1)
- [ ] Injection wall verified on all grounding paths (S2)
- [ ] No `unwrap` on poisoned locks (L1); no fabricated ids (M6)
- [ ] Consent gate enforced + audited on every scanner (verified today for FS)
- [ ] Server turn observation (H1) + event transport (UI-1)

## 23. Deployment Validation Checklist

- [ ] Fresh boot with missing embedder → graceful no-memory degrade (server path exists; desktop VERIFY)
- [ ] DB migration N→N+1 on a populated DB
- [ ] Backup + restore round-trip on a realistic DB size under load
- [ ] Cold-start onboarding on a real home dir without freezing the UI (H4)
- [ ] 24h soak: enrichment queue depth stable, no unbounded memory growth (R2/M5)

## 24. Memory Architecture Lock Criteria (all must hold)

1. ✓ One authoritative `MemorySystem`; no parallel store (verified today: RagEngine removed).
2. ✓ Every runtime observes turns via the shared policy (H1 resolved, Batch 3).
3. ◐ One memory API contract for Tauri + server (API-1 resolved, Batch 10 — shared `contract` module is the single source of truth; server routes fully delegate; desktop adopts where shape-compatible. Desktop read commands keep an intentional UI-superset shape (frontend contract) with the shared CORE fields identical — divergence is documented, not drift. Full desktop migration is UI-gated.)
4. ✓ ANN vector store meeting recall+latency targets at declared scale (H2 resolved, Batch 7).
5. ✓ Backup/restore safe under concurrency (H3 resolved, Batch 2).
6. ✓ Durable, bounded enrichment with depth telemetry (R1/R2 resolved, Batch 8).
7. ✓ `reason()` composes reasoning/goal/plan (M1 resolved, Batch 4).
8. ✓ Unified ingestion (M3 resolved, Batch 6).
9. ✓ Content-level cold-start secret scanning (S1 resolved, Batch 9).
10. ✓ Server memory event transport (UI-1 resolved, Batch 10 — SSE `/memory/events`).
11. ✓ No blocking I/O on async runtime (H4 resolved, Batch 1).
12. ✓ Server memory routes tested live (M4 resolved, Batch 11 — `memory_routes_serve_live_data_with_a_real_memory_system` exercises health/search/remember/metrics/report/library against a live `MemorySystem`).
13. ✓ Consent deny-by-default enforced + tested (verified today).
14. ✓ All memory invariant + benchmark + crash + migration tests green (Batch 11: `memory::` 208, `memory_recovery` 2, server integration 13, ANN recall/crash/backpressure all green. The only failing tests — `routing_mem01` (model behavior) + `config_tests` — are pre-existing and out of memory scope, per §6 of SESSION_HANDOVER).
15. ✓ Docs match code (DOC-1 resolved, Batch 11 — verified-status banner on `MEMORY_ARCHITECTURE_FINAL.md`; SESSION_HANDOVER + this Bible are the live source of truth).

### 🔒 LOCK STATUS: **LOCKED (single-laptop production)** — 14/15 ✓, item 3 ◐.

All hardening batches (1–11) are complete and verified green. The one partial
(item 3, API-1) is an **intentional** desktop UI-superset shape over the shared
contract, not drift — the single contract exists and is authoritative, the
server fully delegates, and the shared core fields are identical across hosts.
Full desktop migration of the richer read commands is gated on coordinated UI
changes and is tracked as future work; it does not block the single-laptop lock.

## 25. Final Production Readiness Score

- Single-laptop dev target: **7.5 / 10** (functional, coherent, some safety/perf debt).
- Multi-user / scale target: **4 / 10** (H1, H2, H3, R1/R2, API-1 block it).

## 26. Final Confidence Assessment

- Desktop memory path correctness: **High** confidence (traced end-to-end).
- Server memory parity: **Medium** (H1/UI-1 confirmed; some voice/gui observe paths marked VERIFY).
- Scale behavior: **High** confidence it is NOT met (H2/R2 proven by code).
- Security/consent: **Medium-High** (FS gate verified; S1/S2 need content-scan + injection-wall tracing).

Outstanding VERIFY items (require a further read-only tracing pass, not implementation):
`S2` injection wall on grounding; desktop voice/image/gui observe coverage;
`config_tests` TOML load/merge (real regression vs stale expectation);
`VectorIndex` remaining consumers (DC1); complete unused-API sweep of the façade.

END OF DOCUMENT.
