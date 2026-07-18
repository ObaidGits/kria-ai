# KRIA Memory Architecture — Session Handover

Purpose: hand a new session the exact state of the Memory Architecture
stabilization work. Trust the code + this doc; verify claims by re-running the
commands in the Verification section.

Companion docs (read these): `STABILIZATION_BIBLE.md` (full issue catalogue +
tracker + lock criteria), `requirements.md`, `design.md`, `tasks.md`.

---

## 1. Where we are

- The Memory Architecture (engine layer, 37/37 spec tasks) is complete and was
  previously integrated (P1–P9): retrieval unification, Tauri façade, Memory
  Workspace UI, knowledge graph, event-driven scheduler + live UI, chat
  feedback, cold-start, server MemorySystem.
- We are now executing the **STABILIZATION_BIBLE** hardening plan in verified
  batches. **ALL 11 batches complete — 23/23 tracked items closed. Architecture
  LOCKED (single-laptop production); 14/15 lock criteria ✓, item 3 (API-1 full
  desktop migration) intentionally ◐ / UI-gated.**
- Every batch ends green: `cargo fmt`, `cargo check --workspace`,
  `cargo clippy --workspace` (0 errors), plus targeted tests.

Production readiness (honest): **single-laptop ~8.3/10**, **multi-user/scale ~5/10**.

---

## 2. Batch plan (11 total)

Done: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11. **ALL BATCHES COMPLETE.**

| # | Item(s) | Status |
|---|---------|--------|
| 1 | M6 (no fabricated UUIDs), L1 (poison-safe locks), L2 (honest `/api/chat` status), H4 (blocking I/O → `spawn_blocking`) | ✅ done |
| 2 | H3 — safe backup (VACUUM off write-lock) + restore rebuilds read pool (no stale reads) | ✅ done |
| 3 | H1 — user-turn observation moved into shared `AgentLoop` (server/Telegram/WS now learn) | ✅ done |
| 4 | M1 — real `reason()` (composes retrieval + reasoning + goal + plan) | ✅ done |
| 5 | M2 — `WriteCandidate::global` (fresh session + Global scope); killed sentinel session UUIDs | ✅ done |
| 6 | M3 — route cold-start import through `Library::ingest` (chunk/dedup/version); one ingestion pipeline | ✅ done |
| 7 | H2 — pluggable ANN vector index (`hnsw_rs`) behind `VectorStore` + recall/latency benchmark | ✅ done |
| 8 | R1+R2 — bounded (backpressure) + durable (crash-recoverable) enrichment queue + depth telemetry gauge | ✅ done |
| 9 | M5 + S1/S2 — salience-gate tool-outcome writes; content-level cold-start secret scan; verify injection wall (+ L3 configurable broadcast capacity) | ✅ done |
| 10 | UI-1 + API-1 — server SSE `/memory/events`; one shared memory-API contract (Tauri + Axum adapters) | ✅ done |
| 11 | cleanup — DC1 (delete dead `VectorIndex`), DC2 (fix stub docs — it's a real no-memory fallback), L4 (cold-start cancellation), M4 (live server route test), DOC-1 + final lock pass | ✅ done |

---

## 3. What changed this session (by batch)

### Batch 1 (safety/correctness)
- `crates/kria-core/src/memory/library.rs` — `row_uuid()` helper; corrupt `library_items.id` now errors instead of fabricating a fresh UUID (M6). Test: `corrupt_item_id_surfaces_error_not_fabricated`.
- `crates/kria-core/src/memory/vectors.rs` — all `.lock().unwrap()` → `.lock().unwrap_or_else(|p| p.into_inner())` (poison-safe, L1).
- `crates/kria-desktop/src/commands/memory.rs` — `memory_cold_start_preview/import`, `memory_library_ingest`, `memory_backup`, `memory_restore` wrapped in `tokio::task::spawn_blocking` (H4).
- `crates/kria-server/src/memory_routes.rs` — `/memory/backup` + `/memory/restore` wrapped in `spawn_blocking` (H4).
- `crates/kria-server/src/routes.rs` — `/api/chat` returns honest `status: ok|empty|error` + `error` field; `run_agent_turn` captures `StreamEvent::Error` (L2).

### Batch 2 (H3 — safe backup/restore)
- `crates/kria-core/src/memory/db/mod.rs` — `read_pool` is now `ArcSwap<Vec<Mutex<Connection>>>` + stored `read_pool_size`; `backup_to` runs `VACUUM INTO` on a **read** connection (write lock held only for the brief checkpoint); `restore_from` uses `wal_checkpoint(TRUNCATE)` then **atomically rebuilds the read pool** so no stale pages. Test: `backup_restore_rebuilds_read_pool_no_stale_reads`.

### Batch 3 (H1 — unified observation)
- `crates/kria-core/src/agent/loop_engine/mod.rs` — added `AgentLoop::observe_user_turn`; called at turn start in `run()` (before grounding). Now every host observes identically.
- Removed redundant desktop observes on loop-driven paths: `chat.rs`, `voice.rs`, `image_chat.rs` (+ dropped their now-unused `memory_system` bindings). `gui_cognition.rs` keeps its observe (it does NOT run the loop).
- Verified `stable_session_uuid` (core) ≡ `memory_session_uuid` (desktop) byte-for-byte → privacy mode still gates. Test: `observe_respects_session_privacy_mode`.

### Batch 4 (M1 — real reason)
- `crates/kria-core/src/memory/api.rs` — new `ReasonedContext { retrieval, reasoning, goals, plan }`; `reason()` composes retrieval + `reasoning_context` + `planner_context` + `recommend` (best-effort). `search()` stays pure.
- Callers updated: `memory_reason` (desktop) + `/memory/reason` (server) return `reasoning_context`/`planner_context`/`plan_recommendation`. Test: `reason_composes_reasoning_goal_plan_context`.

### Batch 5 (M2 — global write scope)
- `crates/kria-core/src/memory/types.rs` — `WriteCandidate::global(content)`: fresh per-write session id + `Scope::Global`. Test: `global_writes_are_fresh_and_global_scoped`.
- Replaced all 5 fixed sentinel session UUIDs with `global()`: `tools/knowledge.rs`, `tools/rag.rs`, `commands/memory.rs`, `api.rs::cold_start_import`, `server/memory_routes.rs`.

### Batch 6 (M3 — one ingestion pipeline)
- `crates/kria-core/src/memory/api.rs` — new `MemorySystem::ingest_document(title, author, path, content) -> (item_id, chunk_count, indexed)`: records item + chunks in the authority `Library` (SHA dedup + versioning) then submits each chunk through the Write Policy with `Source::Library { item, chunk }` provenance. On SHA dedup (`created == false`) it skips re-submission → `indexed == 0` (idempotent).
- `crates/kria-core/src/memory/api.rs::cold_start_import` — readable filesystem/workspace candidates now route through `ingest_document` (chunk/dedup/version/provenance) instead of storing ONE 4000-char truncated `Source::Import` memory. Binary/unreadable files + git/shell candidates stay reference-only `Source::Import` (both `Library` and `Import` are `is_untrusted()` → injection wall preserved).
- `crates/kria-core/src/memory/library.rs` — `Library::ingest` now returns `(Uuid, usize, bool)` (adds `newly_created` so callers skip re-submitting chunks on dedup).
- `crates/kria-core/src/tools/rag.rs` — `ingest_document_rag` collapsed onto `ingest_document` (dropped its own chunk loop + now-unused `adaptive_chunk`/`Source`/`WriteCandidate` imports).
- `crates/kria-desktop/src/commands/memory.rs` — `memory_library_ingest` collapsed onto `ingest_document` (dropped its own chunk loop + unused imports). ONE ingestion path across rag tool, desktop command, and cold-start.
- Tests: `cold_start_import_chunks_files_and_dedups_on_reimport` (large file → multiple library chunks + 1 item; re-import dedups to 0 with no duplicate item); updated `cold_start_preview_and_import_are_consent_gated` (files now land as `library:` events).

### Batch 7 (H2 — ANN vector index)
- `Cargo.toml` + `crates/kria-core/Cargo.toml` — added `hnsw_rs = "0.3"` (in-process pure-Rust HNSW, cosine).
- `crates/kria-core/src/memory/stores/ann_vectors.rs` (new) — `AnnVectorStore` behind the existing `VectorStore` trait. SQLite (`mem_vectors`) stays the **durable authority** (`upsert`/`delete`/`all_ids` unchanged → persistence + restart recovery + reconciliation D-16 intact); each `model_version` gets an in-memory HNSW partition + decoded vectors + scope metadata, **lazily rebuilt from SQLite** on first touch. Partitions `<= 256` live vectors use exact in-RAM brute force (no SQLite reload, no ANN error); larger use HNSW with over-fetch + exact re-rank + a recall guard (falls back to brute force if scope filtering starves the candidate set). Overwrite/delete tombstone the old node; compaction rebuilds once tombstones dominate. Killed the per-query full-BLOB reload that ran on every grounding turn.
- `crates/kria-core/src/memory/stores/sqlite_vectors.rs` — `cosine`/`encode_vector`/`decode_vector` promoted to `pub(crate)` (shared, not duplicated). Brute-force store kept for tests.
- `crates/kria-core/src/memory/api.rs` — main `vectors` store swapped to `AnnVectorStore`; added a shared `vectors: Arc<dyn VectorStore>` field reused by write (SlowPath), read (Retriever), **and** delete (`lifecycle()`) so the in-memory index never goes stale vs SQLite.
- Tests (6): rank-by-cosine (small/brute path), secret-filter + delete, overwrite-updates-index, **survives-reload-from-SQLite** (restart), **ANN recall ≥ 0.7 vs exact brute force at 1500 vectors**, top-k-after-deletes (tombstones never returned).

### Batch 8 (R1+R2 — bounded + durable enrichment queue)
- Insight: the durable substrate already existed — durable `events` table + per-consumer `event_consumer_cursor` + idempotent `enrich` + `enrich_pending` catch-up. The gap was an **unbounded** in-memory wake channel with no backpressure, no crash recovery on boot, and no depth metric. Fixed without a new store (SQLite event log IS the durable queue; consistent with the architecture).
- `crates/kria-core/src/memory/api.rs` — `MemoryConfig` gains `enrichment_queue_capacity` (default 1024) + `enrichment_catchup_interval` (default 30 s); wake channel `mpsc::unbounded_channel` → **bounded** `mpsc::channel(capacity)`; new `MemorySystem::pending_enrichment_depth()` gauge (durable backlog = events past the cursor).
- `crates/kria-core/src/memory/write_policy/mod.rs` — `slow_tx` is now a bounded `Sender`; `submit` uses **`try_send`** (backpressure valve — a full channel drops only the *wake*, never the data; the event is already durable, so `submit` never blocks or grows RAM).
- `crates/kria-core/src/memory/write_policy/slow.rs` — `run(rx, catchup_interval)`: **boot crash-recovery sweep** (`enrich_pending`) before serving live wakes + a **periodic catch-up** (`tokio::select!` timer) that recovers wakes dropped under backpressure. `enrich` idempotency (content-hash + cursor) makes replay safe.
- `crates/kria-core/src/memory/stores/{ports,sqlite}.rs` — `EventStore::pending_count(consumer)` (COUNT of events past the cursor) backing the gauge.
- Tests (3): `enrichment_depth_gauge_tracks_backlog` (gauge rises then → 0), `enrichment_backpressure_drops_wake_not_data` (capacity-1, no drainer → all events durable + all enrich on flush; `submit` never blocks/errors), `enrichment_survives_crash_and_is_idempotent` (record → drop instance → reopen file DB → durable backlog recovered, each event enriched exactly once, replay idempotent).

### Batch 9 (M5 salience gate + S1 content secret scan + S2 injection wall + L3)
- **M5** — `memory/integration.rs::outcome_is_salient`: failures/errors/denials always kept; successes kept only if substantive (payload after `"succeeded:"` ≥ 12 chars and not a generic ack). `api.rs::record_tool_outcome` now gates via it: non-salient → `WriteDecision::Batched` (telemetry only, not persisted, does not wake cognition). New `ToolOutcomeStats` (seen/persisted/gated) + `MemorySystem::tool_outcome_stats()` (honest — gating never hides volume).
- **S1** — `memory/cold_start_scan.rs::content_has_secret`: reuses `sensitivity::classify` (labelled keys/tokens/PII) + a Shannon-entropy heuristic (≥24-char token, ≥4.0 bits/char, mixed alpha+digit) for unlabelled credentials. `api.rs::cold_start_import` scans readable file CONTENT before ingest; a secret-bearing file is skipped entirely (not even a path reference).
- **S2** — verified `Source::Import` AND `Source::Library` are `is_untrusted_content()` → the deterministic injection wall (`write_policy/security.rs`) applies on every grounding write. Test proves imported + library-chunk injection content is rejected while benign user content passes.
- **L3** — `MemoryConfig::change_channel_capacity` (default 256, was a magic literal); broadcast channel sized from config.
- Tests (6): `outcome_salience_keeps_failures_and_drops_trivial_successes`, `tool_outcome_salience_gate_drops_trivial_persists_meaningful`, `content_secret_scan_catches_labelled_and_high_entropy`, `cold_start_skips_files_with_in_content_secrets`, `injection_wall_rejects_imported_and_library_content`, `change_channel_capacity_is_configurable`.

### Batch 10 (UI-1 server SSE + API-1 shared contract)
- **UI-1** — `kria-server/src/memory_routes.rs::events_sse` → `GET /memory/events` (Axum SSE) forwards `MemorySystem::subscribe_changes()` to remote clients (`ready`/`memory`/`lagged` events + keep-alive); 503 when memory unavailable. Server integration test asserts it is mounted + gated.
- **API-1** — new `kria-core/src/memory/contract.rs`: the single canonical memory-API surface over `&MemorySystem` producing the authoritative JSON shapes + shared `hit_json`/`parse_scope`. **Server** (`memory_routes.rs`) is now a thin adapter — every handler delegates to `contract::*` (zero server-side shaping). **Desktop** adopts the contract where shape-identical (`memory_health`, `memory_remember`) and shares `parse_scope` (scope-kind drift eliminated). Desktop read commands keep their intentional UI-superset `hit_json` (namespace/decay/access/state/created_at) — the frontend contract (steering: don't break Tauri contracts); the shared CORE fields are guaranteed identical across both hosts by the contract. `contract_shapes_are_stable` test locks the shapes.
- NOTE: a UI client consuming `/memory/events` is optional (no frontend consumer required by UI-1); the desktop already gets live changes via Tauri events. Full desktop migration of the richer read commands onto the contract is gated on coordinated UI changes (tracked, not drift).

### Batch 11 (cleanup + final lock pass) — LAST BATCH
- **DC1** — deleted the dead legacy `memory/vectors.rs::VectorIndex` (carried through desktop `AppState` → telegram as an unused `_vectors`; real vectors go through `AnnVectorStore`). Removed the threading in desktop (`app_state`/`runtime`/`local_api`/`telegram`) + core `platform/telegram.rs` (`spawn`/`poll_loop`/`process_message` signatures) + the dead `vectors` health service. No behavior change.
- **DC2** — CORRECTION: `register_stubs` is NOT test-only — it's the headless production degraded "core registry only" fallback (memory backend unavailable). Instead of mis-gating behind `#[cfg(test)]` (which would break that path), fixed the misleading "for tests" docs on the fn + call site to describe the real no-memory fallback.
- **L4** — `MemorySystem::cold_start_import_cancellable(source, cands, &CancellationToken)` (checks before each candidate, stops early, commits are independent); `cold_start_import` delegates with a fresh token. Test: `cold_start_import_honors_cancellation`.
- **M4** — new server integration test `memory_routes_serve_live_data_with_a_real_memory_system`: builds a **live** `MemorySystem` (headless `OnnxEmbedder`, FTS floor), injects it into `ServerState`, and asserts real 200s + shapes on health/search/remember/metrics/report/library.
- **DOC-1** — prepended a verified IMPLEMENTATION-STATUS banner to `MEMORY_ARCHITECTURE_FINAL.md` (kept the blueprint; corrected stale as-built claims — HNSW not LanceDB, bounded+durable enrichment, one ingestion pipeline, one API contract + SSE, safety as-built) pointing to this handover + the Bible as the live source of truth.
- **FINAL LOCK**: 14/15 criteria ✓ (item 3 API-1 intentionally ◐, UI-gated). Declared **LOCKED (single-laptop production)** in `STABILIZATION_BIBLE.md §24`.

---

## 4. Verification (re-run to confirm green)

```bash
cargo fmt --all
cargo check --workspace                    # green (2 pre-existing dead-code warnings only)
cargo clippy --workspace                   # 0 errors
cargo test -p kria-core --lib memory::     # 208 passed
cargo test -p kria-core --test memory_recovery     # 2 passed
cargo test -p kria-server --test integration_api   # 13 passed (incl. /memory/events + live-memory routes M4)
cd ui && npx tsc --noEmit                  # memory/App files clean
cd ui && npx vitest run                    # memory store tests pass
```

Key new/critical tests:
- `memory::api::injection_wall_rejects_imported_and_library_content`
- `memory::api::cold_start_skips_files_with_in_content_secrets`
- `memory::api::tool_outcome_salience_gate_drops_trivial_persists_meaningful`
- `memory::contract::contract_shapes_are_stable`
- `memory::stores::ann_vectors::ann_recall_matches_brute_force_at_scale`
- `memory::stores::ann_vectors::survives_reload_from_sqlite_authority`
- `memory::api::enrichment_survives_crash_and_is_idempotent`
- `memory::api::enrichment_backpressure_drops_wake_not_data`
- `memory::db::backup_restore_rebuilds_read_pool_no_stale_reads`
- `memory::api::observe_respects_session_privacy_mode`
- `memory::api::reason_composes_reasoning_goal_plan_context`
- `memory::api::backup_and_restore_round_trip`
- `memory::types::global_writes_are_fresh_and_global_scoped`
- `memory::library::corrupt_item_id_surfaces_error_not_fabricated`
- `kria-server integration_api::memory_routes_are_mounted_and_gate_when_unavailable`

---

## 5. Known / open issues (from the Bible, not yet fixed)



- **DC1/DC2/L4/DOC-1** cleanup + final lock pass. Batch 11 (next).

### 3D graph (asked this session)
- The Memory graph is a real interactive **2D SVG force-directed** graph (`ui/src/components/memory/MemoryGraph.tsx`). It is **NOT 3D** — no three.js/WebGL, no `three` dep. A 3D viewer was never built and is not in the current 11-batch plan; it would be a new scoped task (WebGL renderer over the existing `memoryStore` graph APIs; no backend change needed).

---

## 6. Pre-existing failures — NOT caused by memory work (do not attribute)

- `crates/kria-core/tests/config_tests.rs` — several failing: `default_config_has_expected_values` (theme `light` vs test `dark`; model tier `qwen3-vl-4b` vs `qwen2.5-vl-7b`), plus `load_config_from_valid_toml` / `override_file_merges_into_base` where a loaded/overridden TOML value doesn't take effect. **The last two may be a REAL config load/merge regression — investigate before editing expectations (do not rubber-stamp).**
- `routing_mem01` — semantic router routes a Hindi phrase to `gw_calendar_search` (model behavior).
- `agent::continuation_reentry::verifies_and_records_one_action` — passes in isolation; parallel-ordering flake.
- `ui SettingsModal.google.test.tsx` (×2) — `envLockedFields` is not a function (settings-UI test defect).

These are out of memory scope; left untouched intentionally.

---

## 7. Working rules (carry forward)

- Steering: reply starts with `GraphMode: ON` then `Caveman mode: ON` (terse; normal prose for security/destructive warnings). Keep code/paths/identifiers exact.
- Dev-context (`.kiro/steering/dev-context.md`): single-laptop pre-production; data loss acceptable; delete dead code; hard cutovers fine; no compat shims; still write correct/tested code.
- One complete, compilable, tested batch at a time. Never leave half-wired code. Report honestly; never inflate completion %.
- No stubs / placeholders / parallel implementations / bypasses of `MemorySystem` (the single memory authority).

## 8. Exact next action

**All 11 hardening batches complete + the post-lock audit (AUD-01..06) fully
implemented. Memory Architecture is LOCKED (single-laptop production).** No
further batch is queued.

### Post-lock audit stabilization (AUD-01..06) — DONE
- **AUD-01** `pending_enrichment` surfaced end-to-end: `HealthReport.pending_enrichment` → `contract::health` → server `/memory/health` + desktop `memory_health` → TS `HealthReport` → Health tab row. Live-refreshed via the P8 change-event subscription (`subscribeLive` → `refreshHealth`). Tests: `health_surfaces_pending_enrichment_gauge`, contract shape, server HTTP assertion.
- **AUD-02** `tool_outcomes` surfaced end-to-end: `CognitiveReport.tool_outcomes` (+`summary()`) → `contract::metrics` + desktop `memory_metrics` → TS `Metrics.tool_outcomes` → Metrics tab (kept/gated stats). Live-refreshed via change events. Tests: `cognitive_report_aggregates_engines` (extended), contract shape, server HTTP assertion.
- **AUD-03** cold-start cancellation wired full-stack: `AppState.cold_start_cancel` token slot; `memory_cold_start_import` registers a token + runs `cold_start_import_cancellable`; new `memory_cold_start_cancel` Tauri command (registered in `main.rs`); onboarding UI "Cancel import" button + `coldStartCancel` store API. Overwrite-on-start (no race-clear). Core test `cold_start_import_honors_cancellation`.
- **AUD-04** entropy detector reviewed (fail-safe, unchanged); regression tests added — git SHA / UUID / prose NOT flagged; real key-shaped tokens flagged.
- **AUD-05** ANN recall-guard reviewed (correct, in-scope); regression test `ann_scale_query_respects_scope_filter` proves scope filtering never leaks secrets on the ANN path.
- **AUD-06** `tool_outcomes` uses identical field names on server (contract) + desktop (rich) — no duplicate DTO. Remaining desktop read-shape superset is the documented intentional divergence (frontend contract).

### Only remaining (explicitly non-blocking, future work)
- **API-1 full desktop migration** — migrate the richer desktop read commands onto `memory/contract.rs`; UI-gated (would change Workspace-consumed shapes). Single contract exists; core fields already match. Polish, not drift.
- Scale-tier concerns remain deliberately deferred (single-laptop scope per `dev-context`).
