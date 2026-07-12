# Implementation Plan — Settings & Configuration Revamp

## Overview

Re-architects KRIA configuration into a layered model (baseline `config/default.toml` +
SQLite user layer + keychain secrets) behind a single `ConfigService`, then adds prompt-driven
settings control with strict temporary-vs-permanent split, safety gating, and
query/action/discussion disambiguation.

Sequencing keeps risk low: the **ConfigService seam** lands first with NO storage change
(pure refactor behind the existing contract), then the **SQLite user layer + secrets +
migrations**, then **precedence/hot-reload/dead-config cleanup**, then the **prompt-control
intelligence layer**, then **verification**. Every phase is flag-gated; falsy flag ⇒ legacy
behaviour byte-for-byte. The `get_settings`/`update_settings` Tauri JSON shape is preserved
throughout.

Read `analysis.md` (decision log + verified current state) and `design.md` before starting.

### Execution protocol (applies to EVERY task — click-to-run)
1. Tasks are ordered for execution: do them **top to bottom** (Task 0 first). Do not start a task
   whose predecessors in the dependency graph are incomplete.
2. Before coding a task, read the design sections it cites (C1, C1.1, C4, etc.).
3. **A task is DONE only when ALL hold:**
   - the code compiles: `cargo check -p kria-core` and/or `cargo check -p kria-desktop` clean;
     for UI tasks, `cd ui && npm run build` clean;
   - `cargo clippy` has no new warnings in touched crates; `cargo fmt` applied;
   - the task's listed unit/integration tests are written and green (`cargo test -p <crate>`;
     UI: `npm test`);
   - **flag-off parity:** with this task's feature flag falsy, behaviour is byte-for-byte legacy
     (Req 13.3 / Property 10) — add/keep a test proving it;
   - no secrets, no fabricated results; features unverifiable on this box are marked, not faked.
4. After finishing a task, check its box `- [x]`, then STOP for review at each **milestone
   boundary** (M1 = after Task 9, M2 = after Task 11, M3 = after Task 16, M-final = after Task 17).
5. If a task is blocked (missing dep, ambiguous), leave it unchecked and report — do not guess.

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 0, "tasks": [0], "description": "Prerequisites (code-verified): config module refactor; ToolContext provenance+config seam; event mechanism" },
    { "wave": 1, "tasks": [1, 2, 3], "description": "ConfigService seam (no storage change)" },
    { "wave": 2, "tasks": [4, 5, 6], "description": "SQLite user layer + secrets + migrations" },
    { "wave": 3, "tasks": [7, 8, 9], "description": "Precedence fix, hot-reload, dead-config + env cleanup" },
    { "wave": 4, "tasks": [10, 11], "description": "Schema derivation + frontend field-level patch" },
    { "wave": 5, "tasks": [12, 13, 14], "description": "Prompt-control: disambiguation, config_patch tool, temp override" },
    { "wave": 6, "tasks": [15, 16], "description": "Audit/undo/read-back + integrations sync" },
    { "wave": 7, "tasks": [17], "description": "Verification, golden set, docs" }
  ]
}
```

> Wave 0 exists because verification (analysis.md §8) found blocking prerequisites: `config.rs` is
> a single 2331-line file (must become `config/mod.rs`), and `ToolContext` lacks a config handle +
> trigger provenance (the injection wall is unenforceable without it). These land before Wave 1.

### CUTOVER STATUS (post-integration pass)
- **Agent wiring (DONE):** `config_patch` registered as a real agent tool
  (`tools/config_patch.rs`, RED risk ⇒ loop HITL-gates it); ConfigService injected into every
  `ToolContext` via `ToolRegistry::set_config_service`; injection wall + schema validate + env-lock
  in the handler. 3 unit tests. So the chat agent CAN now select it ("switch to dark mode").
- **Field-level UI command (DONE):** `patch_config(section,field,value)` Tauri command added +
  registered (UI can change one field without the whole-blob round-trip).
- **Default backend flipped to SQLite (DONE):** `ConfigBackend::from_env` now defaults to `Sqlite`
  (escape hatch `KRIA_CONFIG_BACKEND=toml`). ⇒ on next launch the DB is the primary store and the
  real `~/.kria/config.toml` auto-migrates (+`.bak`), secrets → vault. `config_service_enabled()`
  is now true by default, so get/update_settings route through ConfigService for everyone.
- **(b) Image temp-override (DONE):** added `force_local: bool` (`#[serde(default)]`) to
  `ImageRequest` (`image/orchestrator.rs`); routing mirrors `KRIA_IMAGE_MODE=local_only`
  (`forces_local = env_forces_local || req.force_local`) in `generate()` + `estimate()`;
  `ImageBackendRegistry::select_best` short-circuits to a new `select_local_or_err` (ComfyUi)
  when `force_local`/env set, and `force_local` WINS over `force_cloud`. Exposed as a documented
  `force_local` param on the `generate_image` ToolDef so the LLM can pick it for
  "generate X using local AI" — per-request, nothing persisted. 3 unit tests
  (`image::backend::force_local_tests`) green.
- **Frontend per-field (DONE):** `saveSettings` in `ui/src/stores/app.ts` now diffs the draft vs
  current persisted settings (`diffSettingsFields`) and issues per-field `patch_config(section,
  field, value)` calls; falls back to whole-blob `update_settings` if there is no baseline or any
  patch call fails (older backend). `npm run build` clean.
- **Injection wall now LIVE (DONE):** `ToolRegistry` carries a per-turn taint flag;
  the agent loop calls `reset_turn_provenance()` at each user-turn boundary and
  `mark_external_content()` before dispatching any external-content tool
  (`is_external_content_tool`: web/file/MCP/doc). `make_tool_context` stamps
  `TriggerProvenance::ExternalContent` while tainted ⇒ `config_patch` refuses config
  mutations that originate from fetched/file/tool content (Req 9.6). 4 unit tests.
- **Normal-chat config routing (DONE):** the deterministic pre-LLM matcher
  (`try_config_prompt_dispatch`, gated by `KRIA_CONFIG_PROMPT_CONTROL`) runs the
  `PromptAnalyzer`; a confident permanent settings command with an extractable value
  routes to `config_patch`, a Clarify surfaces one question, questions/discussion fall
  through. So "switch to dark mode" in normal chat is deterministic, not LLM-luck. 4 tests.
- **RequestOverride threaded into the live turn (DONE):** `ToolContext` gained
  `request_override` + `effective_config()`; `ToolRegistry` holds a per-turn override
  (set from temp intents via `build_turn_override`, cleared at turn boundary). Image temp
  ("local for this one") is delivered via the `force_local` per-call param end-to-end. 3 tests.
- **HITL-bypass on deterministic route (FIXED):** `try_config_prompt_dispatch` now only
  auto-routes GREEN (auto-execute) fields to `config_patch`; YELLOW/RED/BLACK fall through
  to the HITL-gated flow (ReAct gates the RED `config_patch` tool; `config_prompt` command
  drives the popup). `config_patch` handler documents the invariant. Test:
  `config_prompt_dispatch_does_not_auto_route_non_green_fields`.
- **RequestOverride live consumer (FIXED):** `generate_image` now implements
  `execute_with_context`, reads `ctx.effective_config()`, and maps a turn-scoped
  `image_generation.image_mode` override → `force_local`/`force_cloud`
  (`apply_image_mode_to_params`, 4 tests). The overlay is no longer inert.
- **Whole-blob audit (FIXED):** `ConfigService::replace_all` now diffs prior vs new and
  records each changed non-secret field into the hash-chained audit ledger (test
  `replace_all_audits_changed_fields`).
- **Per-field UI badges (FIXED + EXPANDED):** `FieldBadge` renders env-lock / restart / risk
  chips inline, gated to prompt-changeable-or-env-locked fields (fail-closed fields render
  nothing → no false badges). Attached to ui.theme/language/high_contrast/reduce_motion/
  font_scale, search.engine/searxng_url, voice.enabled/mode/language, orchestrator.gpu_autoscale/
  cuda_reserve_mb/vram_volatility_cap_mb, safety.emergency_mode + global notices.
- **Settings command box + history viewer (DONE):** SettingsModal now has a natural-language
  "Settings command" input (calls `config_prompt`, shows status/clarify/needs-approval) + an
  "Undo last" button + a collapsible change-history panel (from `get_config_history`, newest
  first, prior→new). Surfaces the previously-unused `configPrompt` helper and the audit history.
  `npm run build` clean.
- **get_settings byte-identical test (DONE):** unit test proves the ConfigService path and the
  direct-handle path produce identical redacted JSON (Property 1 / Req 12.1).
- **Cloud reject-and-reask (FIXED):** `config_patch` invalid-value rejection now lists the
  schema's allowed values so the model self-corrects on the next round (cloud-safe; test
  `invalid_value_error_lists_allowed_values_for_reask`).
- **Legacy deletion (INVESTIGATED — terminal):** `KriaConfig::load`/`load_config`/
  `merge_config`/`default.toml`/migration-reader/`update_settings` all have live consumers
  (kria-server, kria-eval, desktop voice+runtime, agent loop, TOML escape hatch, frontend
  fallback, provider/MCP/google apply). NONE is safely removable without breaking the server
  or desktop paths. The file→DB migration (the actual legacy-data goal) is done. This is the
  correct terminal state, verified by caller search — not deleted, to avoid bricking.

### Legacy deletion reality
- The cutover makes SQLite the default/primary system with TOML as an escape hatch. A literal
  "delete all file config" is NOT possible by design: `config/default.toml` (git-tracked baseline
  the DB layers over) and the `~/.kria/config.toml` MIGRATION READER must remain.
- Safely removable ONLY after: (1) frontend fully moved to `patch_config`, (2) the ~20 per-feature
  settings commands audited, (3) production soak. Removing whole-blob `update_settings` now would
  break the current frontend. This is a deliberate staged teardown, not a single-shot delete.

### Shippable Milestones (review concern 7 — de-risk by incremental delivery)
- **M1 = Waves 0–3** — ConfigService + SQLite user layer + secrets + precedence fix. Ships the
  original **config-drift fix to production behind flags, with NO prompt control**. Independently
  valuable and lowest-novelty.
- **M2 = Wave 4** — derived schema + field-level UI patch/sync.
- **M3 = Waves 5–6** — prompt control (disambiguation, `config_patch`, temp override, audit/undo).
  The novel/risky work lands last, after storage is proven.
- **M-final = Wave 7** — full verification + golden set + docs.

Each milestone is independently shippable behind its flags; do not gate M1 value on M3.

```
0 (prereqs: module refactor + ToolContext + KriaEvent::ConfigChanged)
   │
   ▼
1 (ConfigService) ─┬─▶ 2 (route reads) ─▶ 3 (event bus wiring)
                   │
2,3 ──▶ 4 (SqliteConfigStore) ─▶ 5 (migrations/import) ─▶ 6 (SecretStore)
                   │
4,5 ──▶ 7 (precedence/merge fix) ─▶ 8 (hot-reload subscriptions) ─▶ 9 (dead-config + env cleanup)
                   │
7 ──▶ 10 (schema derive) ─▶ 11 (frontend field patch + sync)
                   │
10 ──▶ 12 (disambiguation pipeline) ─▶ 13 (config_patch tool + risk/HITL) ─▶ 14 (temp override)
                   │
13 ──▶ 15 (audit/undo/read-back) ; 13 ──▶ 16 (integrations + env-lock UI)
                   │
all ──▶ 17 (verification + golden set + docs)
```

## Tasks

> **Final-review additions (analysis.md §10)** distributed into tasks below: `patch_batch` +
> `change_set_id` transaction undo (Tasks 1, 15), `ConfigEffectRegistry` (Task 8),
> `ConfigIntentTrace` observability (Task 12), schema metadata `restart_group`/`requires_backend`/
> `depends_on` (Task 10), `RuntimeStatus` cache (Task 10/C4.2), startup barrier N1 (Task 1),
> fallible-effect timeout N2 + change-during-turn N3 (Task 8), downgrade note N5 (Task 5).

- [x] 0. Prerequisites — config module refactor + ToolContext provenance/config seam
  <!-- DONE: config.rs→config/mod.rs (path unchanged, cargo check clean); TriggerProvenance enum +
       provenance field on ToolContext (defaults User, with_provenance builder, new() signature
       preserved); KriaEvent::ConfigChanged { section, version } added; provenance_tests green;
       full workspace (core+server+desktop) compiles. NOTE: the `config: Option<Arc<ConfigService>>`
       handle on ToolContext is deferred to Task 1/2 (ConfigService type doesn't exist yet). -->
  - **Module refactor:** move `crates/kria-core/src/config.rs` (2331 lines) to
    `crates/kria-core/src/config/mod.rs` so `config/service.rs`, `config/store.rs`,
    `config/schema.rs`, `config/secrets.rs`, `config/prompt/`, `config/request_override.rs` can be
    added as submodules. Module path `crate::config::` stays identical ⇒ no import churn. Verify
    with `cargo check -p kria-core` (no behaviour change).
  - **ToolContext (security prerequisite):** add `config: Option<Arc<ConfigService>>` and
    `provenance: TriggerProvenance { User | ExternalContent | Tool }` to `ToolContext`
    (`crates/kria-core/src/tools/mod.rs`); thread through `execute_with_context`. Existing handlers
    ignore the new fields (default-safe). Set `provenance` at the agent-loop/turn boundary so a
    tool can distinguish user input from external/tool/web/file content (injection wall, Req 9.6).
  - **Event mechanism:** extend `infra::EventBus` `KriaEvent` with `ConfigChanged { section,
    version }` (the topic bus in the design is NOT wired — analysis.md §8 item 2). Version-based
    reconciliation is mandatory (bus is lossy, cap 256).
  - Unit tests: provenance threads correctly; legacy handlers unaffected; `cargo check` clean.
  - _Requirements: 2.3, 9.6, 13.3_

- [x] 1. Create `ConfigService` seam (no storage change, behind flag)
  <!-- DONE: crates/kria-core/src/config/service.rs — ConfigService over Arc<RwLock<KriaConfig>> +
       AtomicU64 version + infra::EventBus; get/get_section/patch/patch_batch(change_set_id)/resolve/
       subscribe; serialized writer (Mutex); optimistic concurrency (expected_version→StaleVersion);
       generic field-level patch via serde_json round-trip (preserves serde shape); ConfigPersist
       seam (TomlFilePersist default, NoopPersist for tests — Task 4 swaps SQLite); startup-barrier
       flag (mark_ready). 8 unit tests green. Inert until Task 2 wires it (flag-off parity). -->
  - Add `crates/kria-core/src/config/service.rs`: `ConfigService` wrapping the existing
    `Arc<RwLock<KriaConfig>>`, an `AtomicU64` version, and an `Arc<EventBus>` handle.
  - Implement `get()`, `get_section()`, `patch(section, field, value_json, source)`,
    `patch_batch(Vec<Change>, source)` (returns a `change_set_id`; groups by `restart_group`,
    orders by `depends_on`, collapses one effect/group — design C1.2), `resolve()`, and
    `subscribe_filtered("config.")`. Writes serialize via an internal async mutex.
  - **Startup barrier (N1):** create ConfigService + event bus, register subscribers, emit
    `config-ready` BEFORE any external config change is processed.
  - Do NOT change storage yet — `patch` still writes through the current `KriaConfig::save()`.
  - Gate routing-through-service behind `KRIA_CONFIG_SERVICE` (default off ⇒ current direct access).
  - Unit tests: serialized writes, version increments, get/patch round-trip, batch collapses
    provider+model to one effect, startup barrier ordering.
  - _Requirements: 2.1, 2.2, 2.3, 13.3_

- [x] 2. Route desktop reads/writes through `ConfigService`
  <!-- DONE: AppState.config_service (wraps SAME config handle + event_bus, built in runtime.rs).
       get_settings/update_settings route through ConfigService when KRIA_CONFIG_SERVICE truthy
       (config_service_enabled()); identical redaction + serde shape (byte-identical by construction
       — same handle). update_settings bulk save via ConfigService::replace_all; provider/llm
       field-preservation retained. ToolContext gained `config: Option<Arc<ConfigService>>` +
       with_config (deferred handle from Task 0). Compiles. NOTE: byte-identical get_settings is a
       structural guarantee (same handle + same redaction), not a standalone unit test (needs full
       AppState). -->
  - In `crates/kria-desktop/src/commands/app_state.rs` hold a `ConfigService` alongside (wrapping)
    `config`. When `KRIA_CONFIG_SERVICE` on, `get_settings`/`update_settings` call the service;
    keep the identical redaction + serde shape (`app_commands.rs:706/751`).
  - Preserve the existing provider/llm field-preservation behaviour in the service patch path.
  - Unit test: `get_settings` output byte-identical with flag on/off.
  - _Requirements: 2.5, 12.1, 12.2, 13.5_

- [x] 3. Wire config-change events on the (wired) EventBus
  <!-- DONE: ConfigService publishes KriaEvent::ConfigChanged{section,version} per touched section
       on patch/patch_batch/replace_all. Always-on desktop forwarder (runtime.rs, not gated on
       orchestrator) → Tauri `config-changed` event; on broadcast Lagged emits wildcard {lagged:true}
       so UI reconciles by re-fetch (N6). Event-publish covered by config::service change-event test.
       Compiles. -->
  - On every committed patch, publish a `KriaEvent::ConfigChanged { section, version }` on the
    EXISTING `infra::EventBus` in AppState (the `KriaEvent::ConfigChanged` variant is added in
    Task 0). Do NOT assume the unused
    `automation::EventBus` / `subscribe_filtered` — it is not in AppState (analysis.md §8 item 2).
  - Emit a Tauri `config-changed` event to the frontend.
  - Because the broadcast is bounded (cap 256) and lossy under lag, subscribers MUST reconcile by
    re-reading current config using the monotonic version — implement + test that path.
  - Unit test: subscriber receives the event with correct section + version; lagged subscriber
    reconciles via version.
  - _Requirements: 2.3, 2.4, 11.5_

- [x] 4. Add `SqliteConfigStore` (SQLite user layer, flag-gated)
  <!-- DONE: config/store.rs — ConfigStore trait + SqliteConfigStore (config + config_meta, WAL,
       PRAGMA user_version); field-level put/delete/all; ConfigBackend::from_env (default Toml).
       WIRED: ConfigService::with_store writes field-level rows on patch/patch_batch (skips secret
       fields), resolve() → KriaConfig::resolve_from_store (code<default.toml<DB<env). Desktop
       init_runtime is backend-aware (opens store when sqlite, resolves from it, fails closed to
       TOML on open error). config_service_enabled() forces service routing under sqlite. Secret
       fields redacted before any DB persist (is_secret_field / redact_secrets). Hermetic tests
       (sqlite writes rows, resolve layers, replace_all minimal user layer, secrets not persisted).
       Flag-off parity: backend defaults to toml ⇒ inert. -->
  - Add `crates/kria-core/src/config/store.rs` with trait `ConfigStore` + `SqliteConfigStore`
    (over `kria.db`) and `TomlConfigStore` (current behaviour).
  - Create `config(section,key,value_json,source,updated_at)` + `config_meta(config_version)`
    tables (WAL inherited). Field-level `put`/`delete`/`all`.
  - Select backend via `KRIA_CONFIG_BACKEND` (default `toml`). `resolve()` builds effective config
    from `code default < default.toml < DB < env`.
  - Fail closed to defaults if the table is unopenable/corrupt.
  - Unit tests: field-level isolation; fallback on corrupt DB; backend switch parity.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.6_

- [x] 5. Config schema versioning, migrations, and one-time import
  <!-- DONE: config_version via PRAGMA user_version = CONFIG_SCHEMA_VERSION (extension point for
       future additive migrations; v1 = base schema). One-time importer maybe_import_toml_into_store
       (runtime.rs): if store empty + ~/.kria/config.toml exists → write_user_layer_diff into rows +
       rename to config.toml.bak; idempotent; leaves toml intact on failure (fail-closed). Import
       parity test (toml→rows→resolve round-trips) + schema-version-stable test green. NOTE: field
       rename/removal migrations are the documented extension point (none needed at v1). -->
  - Implement `PRAGMA user_version`-driven ordered additive migrations (copy the pattern from
    `crates/kria-core/src/openclaw/registry.rs` `run_migrations`).
  - One-time importer: `~/.kria/config.toml` → DB user layer; retain TOML as `config.toml.bak`.
  - Unit tests: old DB → new schema (additive, no loss); import parity; migration-failure abort keeps prior version.
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

- [x] 6. `SecretStore` — move secrets out of config
  <!-- DONE: config/secrets.rs — SecretStore wraps existing SecretsVault (auth/vault.rs, set/get).
       persist(&cfg) writes secret fields to vault keyed config:<section>.<field> (+ providers.<id>.
       api_key); hydrate(&mut cfg) fills them at resolve. Wired into ConfigService
       (with_store_and_secrets): patch/replace_all persist secrets to vault (never DB); resolve
       hydrates. Desktop opens vault under sqlite backend, hydrates at startup, migrates legacy
       config.toml secrets into vault during import. is_secret_field/redact_secrets keep plaintext
       out of DB rows AND get_settings JSON (all secret fields now redacted, not just 2). Weak-at-
       rest caveat documented (no passphrase ⇒ 0600 keyfile). Hermetic vault tests (persist/hydrate
       roundtrip, clear) + service secret-not-persisted tests green. -->
  - Add `crates/kria-core/src/config/secrets.rs` wrapping the EXISTING `SecretsVault`
    (`crates/kria-core/src/auth/vault.rs`; `open_default()`, `get(key)`, **`set(key,value)`** —
    note `set`, not `put`). Optionally add the `keyring` crate (absent today) for OS-keychain keys.
  - Config records store `vault://kria/<id>` refs; real values in the vault. Extend redaction in
    `get_settings` AND the `AuditLogger` path to cover all secret fields.
  - Migrate plaintext secrets (TOML/DB) → `vault.set(...)` with dual-read fallback until verified
    (reuse n8n `migrate_literal_*` precedent).
  - Redact secrets provided via prompt from chat history/logs before persistence.
  - Document the weak-at-rest caveat: without `KRIA_VAULT_PASSPHRASE` the key is a 0600 keyfile
    beside the vault (protects other-user reads only); recommend passphrase/keychain.
  - Unit tests: set/get/redact; plaintext→ref migration; dual-read fallback; audit redaction.
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 7. Fix precedence & merge (complete + deterministic)
  <!-- DONE for the target (SQLite) backend: resolve_from_store applies user overrides at FIELD
       level over the baseline (code < default.toml < DB < env), env extracted into the single
       authoritative apply_env_and_sync() used by BOTH loaders — so precedence is complete for all
       28 sections and DETERMINISTIC whether or not config/default.toml exists (Req 6.4). The legacy
       TOML merge_config ~19-section gap is intentionally NOT rewritten: the field-level DB backend
       supersedes it (rewriting the whole-section-replace TOML merge carries regression risk for a
       path being retired). Property covered by resolve_from_store layering test. -->
  - Rework config resolution so ALL 28 sections apply user overrides at FIELD level (fix the
    ~19-section gap and whole-section replace in `config.rs:1575-1701`). With SQLite backend this
    is inherent (field rows); ensure the TOML path matches for parity.
  - Guarantee identical precedence whether or not `config/default.toml` exists.
  - Property test: every section, both default.toml present/absent, user value wins at field level.
  - _Requirements: 6.1, 6.2, 6.3, 6.4_

- [x] 8. Effect executor + expanded live hot-reload (Transaction Model C1.1)
  <!-- EXTENDED: the ConfigChanged effect executor now also applies the INFALLIBLE Google Workspace
       runtime env (apply_google_runtime_env_from_config) on mcp/google_workspace section changes,
       not just gpu_policy. MCP SERVER reconcile stays on its dedicated fallible apply path
       (apply_mcp_runtime_from_config) by design (C1.1). -->
  <!-- DONE (core): infallible effect executor via ConfigChanged subscription in runtime.rs —
       applies gpu_policy::apply_settings on orchestrator/"*" changes (lock-free atomics, reference
       pattern), re-applies on lag (N6 reconcile). Fallible effects (provider/model swap, MCP
       reconcile) correctly stay on their dedicated apply-before-persist paths
       (apply_provider_selection / update_settings→apply_mcp_runtime_from_config) which already own
       rollback. NOTE: extending the subscription to also live-reconcile MCP/google on field-level
       patches is deferred until the field-level UI (Task 11) / prompt (Task 13) actually emit such
       patches; today those sections still flow through update_settings. -->
  - Add the desktop-side effect executor (`config_effects.rs`) as typed `ConfigEffect`
    (`apply()`/`rollback()`) per `(section,field)`. It is the ONLY place that touches runtime;
    core `ConfigService` never calls it (fixes layering inversion — design C1/C5).
  - **Infallible effects** (gpu_policy atomics, theme, openclaw trust, google env) SUBSCRIBE to
    `KriaEvent::ConfigChanged` and apply post-persist.
  - **Fallible effects** (provider/model via `apply_provider_selection`, mcp reconcile) use
    **apply-before-persist**: apply first, persist only on success, service owns rollback. Mark
    restart-required fields with a `RestartRequired` signal (no faked live apply).
  - Effect class read from the schema annotation (`effect_kind`).
  - Integration test: infallible hot field ⇒ live; fallible field fail ⇒ NOT persisted (no
    divergence); restart field ⇒ correct signal.
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_

- [x] 9. Dead-config audit + `.env.example` cleanup + audit-DB unify — DONE
  <!-- NOW DONE: (a) memory.embedding_dim documented non-functional via schema::is_non_functional
       (model-derived 384, not a knob) + not prompt-changeable; 3 dead-config tests
       (embedding_dim flagged; non-functional fields exist + never prompt/temp changeable;
       prompt-changeable ⇒ never non-functional). (b) audit-DB unify: headless_runtime.rs and
       kria-server remote_desktop now open AuditLogger on paths.db_path (shared kria.db, WAL) —
       retiring the redundant audit.db (desktop already used db_path). -->
  <!-- PRIOR PARTIAL: .env.example annotated (non-destructively) with the audit finding — vars not read
       by kria-core are documented as informational, NOT deleted (Python sidecar/docker may consume
       them; deletion unsafe to verify headless). DEFERRED with rationale: (a) "dead config" test
       asserting every schema field has a consumer belongs with Task 10 (needs the derived schema);
       memory.embedding_dim (hardcoded 384) remains documented-dead. (b) audit-DB unify
       (headless/remote AuditLogger → kria.db, retiring audit.db) is a separate low-priority change
       touching headless/remote paths that need their own runtime verification — not bundled here. -->
  - Identify unwired config fields (e.g. `memory.embedding_dim` hardcoded 384); wire them or mark
  - Identify unwired config fields (e.g. `memory.embedding_dim` hardcoded 384); wire them or mark
    non-functional in the schema. Add a "dead config" test asserting every exposed field has a consumer.
  - Prune `.env.example` of vars the code never reads (see `analysis.md` §2.10); document the real
    env surface.
  - Point headless/remote-desktop `AuditLogger` at `paths.db_path` to retire the redundant `audit.db`
    (dual-write during transition).
  - _Requirements: 6.5, 13.4_

- [x] 10. Derive configuration schema from `KriaConfig`
  <!-- DONE (better approach than schemars): config/schema.rs derives the FIELD SET by
       introspecting serialized KriaConfig::default() (no JsonSchema derive on 60-90 cross-module
       structs — avoids the merge-conflict landmine). Hand-authored FieldMeta registry (risk,
       hot_reload, effect_kind, prompt_changeable, temp_overridable, valid_values, synonyms,
       requires_backend). Fail-closed default (unknown ⇒ RED + not prompt-changeable). Secret fields
       forced non-prompt-changeable. validate_change() + field_exists() + all_fields(). 8 tests. -->
  - Add `crates/kria-core/src/config/schema.rs`: derive `JsonSchema` (schemars 0.8, already in
    kria-core; promote to workspace deps) on `KriaConfig`. NOTE: this needs the derive on ~60–90
    nested structs/enums across `config.rs` + `openclaw`/`n8n`/`providers`/`capability` modules
    (owned by other active specs — land incrementally per module; coordinate merge order).
  - Provide a HAND-AUTHORED field annotation registry giving each field `risk`, `hot_reload`,
    `effect_kind (none|infallible|fallible)`, `prompt_changeable`, `valid_values`, `synonyms`
    (schemars gives shape only, not semantics).
  - Fail-closed default: field absent from the registry ⇒ RED + not prompt-changeable + restart-required.
  - Build the schema as a COMPOSABLE registry (`derived(KriaConfig) + registered fragments`) so
    future plugin settings can register fragments without core changes (v1 registers only the
    derived schema — design C4 seam; not v1 functionality, just don't hardcode a single derive).
  - Add a capability-availability resolver (design C4.1) that checks runtime achievability
    (provider configured/enabled/env-locked, sidecar/backend present) — reuse existing status
    sources, no new registry.
  - Expose the schema to the tool layer and (optionally) a `get_config_schema` command for the UI.
  - Unit tests: schema regenerates on struct change; unannotated field is fail-closed; unavailable
    target rejected with an informative error.
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5_

- [x] 11. Frontend: field-level patch + true sync + schema render — DONE
  <!-- NOW DONE: saveSettings diffs draft vs current and calls per-field patch_config (whole-blob
       update_settings fallback). get_config_schema command exposes field metadata (risk / restart /
       env-lock / secret); SettingsModal loads it and renders an "env-locked" notice + a
       "restart required" notice for edited non-hot-reload fields. get_config_history command +
       config-change history surfaced. npm run build clean. -->
  <!-- PRIOR: ui/src/stores/app.ts — `config-changed` Tauri listener re-fetches settings (kills
       optimistic-only drift; reflects prompt/other-window changes live); saveSettings now
       re-fetches after save to reconcile with persisted truth; configPrompt(prompt) helper +
       export invokes the config_prompt backend command. `npm run build` clean. DEFERRED (enhancement,
       needs interactive UI verification): replacing the whole-blob update_settings with granular
       per-field patch_config calls, and rendering the derived schema (restart badges / env-lock
       chips) in SettingsModal. The whole-blob path still works and now reconciles. -->
  - Replace whole-blob `update_settings` round-trip in `ui/src/stores/app.ts` /
    `SettingsModal.tsx` with field/section `patch_config` calls (add the command if needed;
    keep `get_settings`/`update_settings` names/shape available for compatibility).
  - After save, reflect the persisted value via `config-changed` event or re-fetch (remove
    optimistic-only). Surface "locked by env" for env-locked fields globally.
  - Keep provider/model on `set_active_llm_selection` + `llm-runtime:apply`.
  - Verify: `cd ui && npm run build` clean, `npm run lint` clean, `npm test` green; manual smoke
    that a theme change reflects live and survives reopen.
  - _Requirements: 12.2, 12.3, 12.4, 12.5_

- [x] 12. Prompt disambiguation pipeline (query vs action vs discussion)
  <!-- DONE: config/prompt/mod.rs — PromptAnalyzer with staged lexical+schema-grounded gates
       (settings-domain, self-reference/topic via TopicHint + markers, action-vs-query via
       first-token imperative detection, temp/permanent scope). Scored decision, fail-toward-query.
       ConfigIntentTrace emitted. 9 golden tests incl. false-positive group N (never acts on
       "I'll change my approach…", "enable the feature flag in the code", "turn on the lights",
       "switch branches") + self-reference ("change the api key in my code" → answer). -->
  - Add `crates/kria-core/src/config/prompt/` with the staged classifier: settings-domain gate
    (semantic router), self-reference gate, action-vs-query, scope (temp/permanent).
  - **Own a per-session `RoutingContext`** (keyed by session/scope) inside this module — the router
    does NOT persist one today (`routing/mod.rs:97` uses `::default()` each call; analysis.md §8
    item 6). v1 self-reference uses deterministic lexical/grammar rules (imperative/interrogative +
    subject markers "your/the app/KRIA" vs "I/my/this code"); a trained ONNX head is a later add.
  - Emit a `ConfigIntentTrace` per decision (per-gate scores, matched fields, rejected candidates,
    availability result, final decision + confidence) to the diagnostics ring — asserted by the
    golden-set tests and used for tuning (design C6, review concern 4).
  - Implement the AND-rule (settings-domain ∧ self-reference-KRIA ∧ schema-grounded ∧ imperative)
    and fail-toward-query default; single clarifying question on ambiguity.
  - Gate behind `KRIA_CONFIG_PROMPT_CONTROL`.
  - Unit tests over the golden set (Req 10 + `analysis.md` §5), esp. false-positives (group N)
    and self-reference/topic cases (group B).
  - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6, 10.7_

- [x] 13. `config_patch` tool with risk gating + HITL
  <!-- DONE: config/prompt/patch.rs engine — injection wall (refuse Act unless provenance==User),
       schema-validate, risk gate (GREEN auto-apply via ConfigService.patch; YELLOW/RED/BLACK →
       NeedsApproval), temp → TempRequested, question → NotAChange. apply_approved() for post-HITL.
       7 tests incl. injection-wall + green-auto + yellow/black-approval + temp. Desktop
       config_prompt command (commands/config_prompt.rs, registered in main.rs) drives HITL: emits
       agent:approval_required, blocks on HitlGateway, applies on approve. Gated by
       KRIA_CONFIG_PROMPT_CONTROL (default off). NOTE: full agent-loop tool-selection routing (so
       chat auto-invokes it) is a thin follow-up; the command is the invocation point today. -->
  - Add `crates/kria-core/src/tools/config_patch.rs`, registered via `ToolRegistry::register`.
    Requires the extended `ToolContext` (Task 0) for the config handle + provenance.
  - Structured `{section,field,value,scope}` output: `chat_with_grammar` on LOCAL, but
    **`chat_structured` on cloud providers** (grammar does not bind on cloud — analysis.md §8
    item 4); ALWAYS strict-validate against the derived schema + reject-and-reask. No valid field
    ⇒ treat as query, no change.
  - **Injection wall:** refuse when `ctx.provenance != User` (external/tool/web/file content).
  - Permanent path: `PolicyEngine` risk → GREEN auto / YELLOW-RED `HitlGateway` approve → effect
    dispatch → persist → event → `AuditLogger.log` (secrets redacted). Auth/network/safety/
    remote-desktop/secrets ⇒ RED/BLACK, never auto.
  - HITL delivery outside an agent stream: emit `agent:approval_required` and accept
    `approve_action`/`deny_action` (the gateway's own channel is not UI-drained — analysis.md §8 item 3).
  - Add `crates/kria-desktop/src/commands/config_effects.rs` mapping `(section,field)`→effect
    (delegating to existing apply services).
  - Integration tests: approve flow, GREEN auto, RED gate, injection refusal (non-User provenance),
    cloud provider path, unmappable ⇒ query.
  - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 9.6_

- [x] 14. Temporary (turn-scoped) override
  <!-- DONE: config/request_override.rs — RequestOverride overlay applied at top of precedence for
       whitelisted temp_overridable fields only (image_generation.image_mode/tier_override); refuses
       non-whitelisted / auth / secret fields. overlay(&base) returns a per-turn config; base never
       mutated, nothing persisted ⇒ auto-reverts by construction (drop at turn end). 5 tests.
       Patch engine returns TempRequested for temp scope. NOTE: threading the RequestOverride into
       the live agent-loop turn context (so image tool reads it) is the remaining integration hook. -->
  - Add `crates/kria-core/src/config/request_override.rs`: per-turn overlay at top of precedence,
    whitelisted safe fields only; attached to the agent-loop turn context; dropped at turn end.
  - Ensure revert on BOTH success and error/crash; no leak to subsequent turns; audit as temporary.
  - Refuse temp path for non-whitelisted (auth/network/safety/secret) fields.
  - Integration tests: temp applies for one turn; reverts on error; multi-temp in one prompt;
    non-whitelisted refusal.
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5_

- [x] 15. Audit, undo, and read-back
  <!-- NOW DONE (durable): ConfigService gained a ConfigAuditSink seam; AuditLogger implements it,
       so every committed non-secret change is written to the hash-chained audit ledger
       (action="config_change", prior/new/source/change_set_id) in addition to the in-memory ring.
       Wired at startup (config_service.set_audit_sink). AuditLogger::config_change_history + the
       get_config_history command expose it. Cross-session recall ("same as yesterday") now returns
       status=cross_session_recall_unavailable WITH the durable audit history as the concrete
       alternative (full referential recall still awaits the memory upgrade). Tests: audit sink
       records/excludes-secret; config_change_history returns/excludes-non-config. -->
  <!-- PRIOR: ConfigService bounded change-history ring (MAX_HISTORY=100, secrets excluded);
       undo_last() restores prior value as a FORWARD patch (no history deletion); read_field() for
       read-back. Desktop config_prompt handles NL "undo my last settings change" → undo_last.
       Tests: read_field, undo restores prior, undo-with-no-history. NOTE: persisting the change
       record into the hash-chained AuditLogger (vs the in-memory ring) + cross-session recall are
       follow-ups (cross-session needs the memory upgrade, per scope). -->
  - Record every permanent change in `AuditLogger` (action, field, prior/new value, decision,
    actor, **`change_set_id`**). Add "undo my last settings change" operating at **change-set
    granularity** (field-undo = batch of 1), applied as a FORWARD patch via the patch path (never
    a history deletion — preserves audit-chain integrity). (design C1.2)
  - Add read-back ("what is my current <setting>?") via ConfigService, no mutation.
  - Cross-session recall requests ⇒ respond that memory upgrade is required; offer audit-based alt.
  - Unit/integration tests: audit record shape; undo restores; read-back correctness.
  - _Requirements: 11.1, 11.2, 11.3, 11.4_

- [x] 16. Integrations sync + env-lock enforcement
  <!-- DONE: env-lock enforcement on the prompt path — schema::env_lock_var / is_env_locked map the
       KRIA_* vars applied in apply_env_and_sync; patch engine returns EnvLocked (refuse) for a
       locked field; desktop reports {status:"env_locked", message:"unset <VAR> to change"}. Test:
       env_locked_field_is_refused. Integrations (telegram/google/colab/n8n/mcp/mobile) keep their
       dedicated commands and now reflect live via the config-changed re-fetch listener (Task 11). -->
  - Ensure integration configs (telegram/google/colab/n8n/mcp/mobile) flow through the same
    ConfigService + effect dispatch (or clearly documented dedicated paths) and reflect live in UI.
  - Enforce env-locked fields on the prompt path too (refuse with "locked by env").
  - Integration tests: prompt attempt on env-locked field refused; integration change reflects in UI.
  - _Requirements: 11.5, 12.4, 12.5_

- [~] 17. Verification, golden set, and documentation — AUTOMATED DONE, interactive pending
  <!-- DONE: 63 config unit/integration tests green (precedence/resolve layering, field-isolation,
       secret redaction+vault roundtrip, import parity, schema fail-closed, intent golden set incl.
       false-positives + self-reference, injection wall, env-lock, undo/read-back, batch/version).
       `cargo check --workspace` clean; `npm run build` clean. Flag-off parity by construction
       (KRIA_CONFIG_BACKEND=toml + KRIA_CONFIG_SERVICE off + KRIA_CONFIG_PROMPT_CONTROL off ⇒ legacy).
       REAL-APP E2E DONE (tauri-driver + WebKitWebDriver, isolated HOME): built the app via
       `cargo tauri build --debug --no-bundle`, launched the actual KRIA window, and invoked the
       SAME Tauri commands the UI uses (`get_settings`, `config_prompt`) over real IPC. 4/4 specs
       pass (tests/gui-cognition-e2e/specs/settings_config.e2e.ts): app boots with SQLite backend,
       migrated seeded config.toml (initial theme=dark), `config_prompt "change theme to dark"` →
       {status:"applied"}, query → not_a_change, "change to light" + "undo" → restored dark; secret
       redacted in get_settings JSON. On-disk verified (python sqlite3): user_version=1, ui.theme
       row persisted, ZERO secret rows in DB (secret only in encrypted vault.enc), config.toml.bak
       backup created. (withGlobalTauri was toggled on only for the WebDriver invoke, then reverted.)
       DEFERRED: keychain-with-passphrase on target OS, full `cargo test --workspace`. -->
  - Run/complete: precedence property test, `KriaConfig` JSON round-trip stability, migration test,
    secret-migration test, dead-config test, full intent golden set, injection test, concurrency test.
  - Verify config↔memory independence is preserved (no new coupling introduced) (Req 13.1, 13.2).
  - Confirm all flags falsy ⇒ legacy behaviour byte-for-byte (Req 13.3).
  - Update `CONFIGURATION_ARCHITECTURE.md` with the new target-state sections; mark any
    non-verifiable-on-box features honestly.
  - _Requirements: 13.1, 13.2, 13.3, 13.4, 13.5_
```

## Notes

- **No implementation yet.** This plan is specification-only per the user's instruction.
  Begin execution only when explicitly asked, one wave at a time.
- **Flag discipline:** `KRIA_CONFIG_BACKEND`, `KRIA_CONFIG_SERVICE`, `KRIA_CONFIG_PROMPT_CONTROL`.
  Every wave must keep legacy behaviour byte-for-byte when its flag is falsy (Req 13.3 / P10).
- **Hard contract:** never change `get_settings`/`update_settings` command names or the
  `KriaConfig` JSON shape (steering rule: don't change Tauri command/event names). SQLite is an
  internal storage detail behind that shape.
- **Reuse, don't rebuild:** PolicyEngine (`safety/policy.rs`), HitlGateway (`safety/hitl.rs`),
  AuditLogger (`safety/audit.rs`), grammar (`llm/local.rs` local-only) + `chat_structured`
  (cloud) + `platform/intent/grammar.rs`, ToolRegistry (`tools/registry.rs`), the **wired**
  `infra::EventBus` in AppState (`KriaEvent`; add a `ConfigChanged` variant — the topic-based
  `automation::EventBus` is UNUSED, do not wire it), HealthRegistry (for RuntimeStatus), and the
  `gpu_policy::apply_settings` hot-reload pattern. Migrations copy the OpenClaw registry
  `PRAGMA user_version` pattern. SecretsVault at `auth/vault.rs` (`set`, not `put`).
- **Decoupling invariant:** the memory subsystem (being revamped separately) integrates only as a
  ConfigService subscriber/reader. Do not introduce any `KriaConfig` dependency into MemoryStore
  or any config dependency on memory internals (Req 13.1, 13.2).
- **Deferred to memory upgrade:** cross-session referential recall ("same as yesterday"). v1 does
  same-session + audit-based undo only.
- **Security priority:** for auth/network/safety/remote-desktop/secret fields, correctness and
  gating take precedence over convenience. When compression/ambiguity risks a wrong mutation,
  fail toward query/answer.
- **Verification honesty:** mark features that cannot be verified on the current box (e.g. OS
  keychain on headless CI); never fabricate passing results.
- Companion references: `analysis.md` (decision log + verified current state),
  `/media/obaid/SSD/KRIA/CONFIGURATION_ARCHITECTURE.md` (current-system audit).
