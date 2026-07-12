# KRIA Configuration Architecture Audit

> **Status:** Investigation only. Describes the **real current state** of the codebase.
> No redesign implemented. All file paths and line numbers reflect the source as audited.
>
> **Scope:** How configuration is defined, loaded, merged, propagated, owned, read,
> mutated, saved, and hot-reloaded across `kria-core`, `kria-desktop`, `kria-server`,
> and the SolidJS UI.

---

## Table of Contents
1. High-Level Overview
2. Startup Flow
3. Configuration Sources
4. Configuration Precedence
5. Configuration Ownership
6. Configuration Consumers
7. Saving Flow
8. Hot Reload Matrix
9. Current SQLite Usage
10. Environment Variables
11. Current Problems
12. Pain-Point Analysis
13. Current Strengths
14. Future Recommendation
15. Migration Plan

---

## SECTION 1 — High-Level Overview

### 1.1 What configuration *is* in KRIA

There is **one canonical config struct**: `KriaConfig`
(`crates/kria-core/src/config.rs:8-49`). It is a 28-section tree
(`llm`, `voice`, `classifier`, `memory`, `safety`, `agent`, `server`, `ui`,
`search`, `mcp`, `telegram`, `hardware`, `orchestrator`, `colab`, `routing`,
`image_generation`, `executive`, `planner`, `uncertainty`, `skill_compiler`,
`curiosity`, `browser_agent`, `openclaw`, `capability`, `n8n`, `providers`,
`mobile`, `ntfy`, `remote_desktop`).

A **separate, unrelated** config struct also exists: `KriaSystemConfig`
(`config.rs:806-882`) — infra/SRE knobs (QoS, target-pool, snapshot tolerance).
It is loaded from a **different file** (`kria_config.toml`) via the `config` crate
and is not merged into `KriaConfig`.

### 1.2 Where configuration starts

Three physical inputs feed `KriaConfig`:

| Layer | File | Role |
|-------|------|------|
| Project default | `config/default.toml` (discovered by walking up from exe + CWD) | Base / dev baseline |
| User override | `~/.kria/config.toml` | What the user actually edits (Settings UI writes here) |
| Environment | process env (`KRIA_*`) + `.env` | Final override for a **subset** of fields |

Plus **out-of-band** sources not part of `KriaConfig`:
- `config/mcp_servers.json` and `~/.kria/mcp_servers.json` (merged into `config.mcp.servers` by `load_mcp_servers`, `config.rs`).
- `kria_config.toml` → `KriaSystemConfig` (infra).
- localStorage-only UI prefs (frontend, never persisted to backend).

### 1.3 How it loads (one sentence each)

- `KriaConfig::load(None)` finds `config/default.toml`, parses it, merges
  `~/.kria/config.toml` on top via `merge_config`, then applies `KRIA_*` env
  overrides, then re-syncs legacy `llm.*` fields from the active provider.
- Result is an **owned `KriaConfig`** value that the runtime mutates in place
  (tier clamping, context-window clamping) during startup, then finally wraps
  in `Arc<RwLock<KriaConfig>>` and hands to Tauri state.

### 1.4 Who owns / reads / modifies / saves

```
             ┌─────────────────────────────────────────────────────────┐
             │  ~/.kria/config.toml   (single writable source of truth) │
             └───────────────▲──────────────────────────┬──────────────┘
                             save()                      load()
                              │                            │
           ┌──────────────────┴───────┐        ┌───────────▼───────────────┐
  WRITE →  │ KriaConfig::save()        │        │ KriaConfig::load()          │ ← READ
           │ (whole struct, pretty     │        │ default.toml + user +       │
           │  TOML, atomic rename)     │        │ env → owned KriaConfig       │
           └──────────────────▲───────┘        └───────────┬───────────────┘
                              │                             │ moved/cloned
        update_settings /     │                    ┌────────▼─────────────────┐
        apply_provider_...    │                    │ AppState.config           │
                              │                    │  Arc<RwLock<KriaConfig>>  │  (desktop only)
           ┌──────────────────┴───────┐            └────────┬─────────────────┘
           │  Tauri command handlers   │                     │ .read()/.write()
           │  (kria-desktop/commands)  │◄────────────────────┘
           └──────────────────▲───────┘
                              │ invoke()
                     ┌─────────┴──────────┐
                     │  SolidJS UI stores  │  (settings signal cache, optimistic)
                     └────────────────────┘
```

- **Owns the live handle:** `AppState.config: Arc<RwLock<KriaConfig>>`
  (`crates/kria-desktop/src/commands/app_state.rs:60`). Single instance.
- **Reads:** every subsystem — but most read a **clone/snapshot** taken at
  construction, not the live handle (see Section 5).
- **Modifies at runtime:** `update_settings` and the provider-apply path
  (`providers.rs`) mutate `*state.config.write()`.
- **Saves:** only `KriaConfig::save()` — always the **whole struct** to
  `~/.kria/config.toml`.

### 1.5 Server vs Desktop

`kria-server` loads the same `KriaConfig::load(None)` but stores it as a **plain,
immutable owned value** in `ServerState.config` (`kria-server/src/lib.rs:21`) —
no `RwLock`, no runtime mutation, no reload. There is **no `config.rs` in
kria-server**; it reuses `kria_core::config`.

---

## SECTION 2 — Startup Flow (REAL)

### 2.1 Desktop (`kria-desktop`) — the primary product

There is **no `lib.rs`** in kria-desktop. `main.rs:16` builds the Tauri app; in
`.setup` it calls `tauri::async_runtime::spawn(init_runtime(handle))`
(`main.rs:82`). **All real startup is in
`crates/kria-desktop/src/commands/runtime.rs::init_runtime()` (line 5).**

Real ordered flow:

```
main() [main.rs:16]
  └─ tauri::Builder … .setup(|app| { spawn(init_runtime(handle)) })   [main.rs:82]
       │
       ▼
init_runtime(handle)  [runtime.rs:5]
 1. KriaPaths::resolve()                         → ~/.kria/* dirs created
 2. setup_logging(logs_dir)
 3. config = KriaConfig::load(None)              → default.toml + user + env  [runtime.rs:11]
 4. paths = config.resolve_paths()
 5. gpu_policy::apply_settings(gpu_autoscale, cuda_reserve_mb, vram_volatility_cap_mb)  [runtime.rs:14]
 6. n8n.migrate_literal_api_key_to_file() → if migrated, config.save()        [runtime.rs:21]
 7. resolve_hardware_info(config, hw_cache)      precedence env>config>cache>detect
 8. write hardware_tier.json cache
 9. MUTATE config in place:                                                   [runtime.rs:48-77]
       - clamp llm.context_window to tier ceiling
       - fill hardware.threads / gpu_layers from tier if unset
       - resolve voice.stt_model "auto" → tier model
10. MemoryStore::open(db_path)                   → kria.db (SQLite)           [runtime.rs:88]
11. TaskStore::open(db_path) + spawn_reminder_scheduler (30s poll)
12. OpenClawSubsystem::boot(data_dir)            → skills.db (+ retry pool if enabled)
13. set_live_trust_config(openclaw.trust)        process-wide trust snapshot
14. ContainerPool::new(openclaw_config)          only if openclaw.enabled
15. … construct ModelRouter, Orchestrator, ToolRegistry, ImageOrchestrator,
      McpServerManager, Router (semantic), AgentLoop, voice pipeline …
      ALL built from `config` value or CLONES/snapshots of its sub-sections
16. audit_db = Connection::open(db_path); AuditLogger::new(...)               [runtime.rs:1318]
17. config wrapped LATE into Arc<RwLock<KriaConfig>> and stored in AppState    [runtime.rs ~1613]
18. app.manage(AppState { config, model_router, agent_loop, … })
19. agent ready
```

**Critical ordering fact:** `config` is an **owned `mut KriaConfig`** for nearly
the whole of `init_runtime`. It is wrapped in `Arc<RwLock<>>` **after** every
subsystem has already been built from clones. Therefore subsystems do **not**
observe later `write()`s unless an explicit "apply" path re-pushes to them.

### 2.2 Server (`kria-server`)

```
main() [kria-server/src/main.rs:37]
 1. KriaPaths::resolve(); setup_logging
 2. config = KriaConfig::load(None)               [main.rs:40]
 3. initialize_fleet_schema()
 4. FleetRuntime::initialize(&config)
 5. bind_host = mobile.bind_interface || server.host  (warns on 0.0.0.0)
 6. headless agent via kria_core::agent::headless_runtime::build_minimal(&config)
 7. ServerState { config: KriaConfig (owned, immutable) }  [lib.rs:21]
 8. Axum serve
```

No `Arc<RwLock>`, no live reload, no file watcher.

---

## SECTION 3 — Configuration Sources

| # | Source | Purpose | Priority | Read by | Written by | Mutable at runtime | Persistent |
|---|--------|---------|----------|---------|------------|--------------------|------------|
| 1 | Hardcoded `Default` impls (`config.rs`) | Code-level fallback for every field | Lowest | `load_config` when file absent | — (compiled) | No | Yes (in binary) |
| 2 | `config/default.toml` | Project/dev baseline | Low | `KriaConfig::load` (base) | Devs (git) | No | Yes |
| 3 | `~/.kria/config.toml` | User override — **the** editable source | Medium | `load_config` (merged) | `KriaConfig::save()` | Yes (via Settings) | Yes |
| 4 | `KRIA_*` env vars + `.env` | Ops/secret/CI overrides (subset only) | High | `load_config`, `apply_*_env_overrides`, `KriaPaths` | user/shell | No (read at load) | Session only |
| 5 | `config/mcp_servers.json` / `~/.kria/mcp_servers.json` | MCP server definitions | additive | `load_mcp_servers` | manual / MCP commands | via commands | Yes |
| 6 | `kria_config.toml` → `KriaSystemConfig` | Infra QoS / pool / snapshot | separate tree | `KriaSystemConfig::load` | manual | No | Yes |
| 7 | `~/.kria/hardware_tier.json` | Cached hardware detection | cache | `resolve_hardware_info` | `init_runtime` | rewritten each boot | Yes |
| 8 | SQLite `kria.db` `preferences` table | Session/UI prefs (theme, per-session flags) | runtime | MemoryStore | MemoryStore | Yes | Yes |
| 9 | `AppState.config` `Arc<RwLock<KriaConfig>>` | Live in-memory copy (desktop) | runtime authority | all commands | `update_settings`, provider-apply | Yes | No (mirror of #3) |
| 10 | Subsystem-local clones/snapshots | What most components actually use | runtime | each subsystem | construction-time | Mostly no | No |
| 11 | localStorage (`kria_assistant_frontend_prefs`, `kria_labs_frontend_prefs`, `kria_mcp_catalog`) | Pure-frontend prefs / hardcoded mock | UI only | UI | UI | Yes | Yes (browser) |
| 12 | Process-wide static snapshots (e.g. `gpu_policy` atomics, `trust_runtime`) | Hot knobs read without holding config | runtime | specific modules | `apply_settings`/`set_live_trust_config` | Yes | No |

---

## SECTION 4 — Configuration Precedence

### 4.1 Documented / intended precedence

```
code Default  <  config/default.toml  <  ~/.kria/config.toml  <  KRIA_* env
```

Implemented in `load_config` (`config.rs:1370-1452`):
1. Parse `default.toml` (or `KriaConfig::default()` if absent).
2. `merge_config(&mut base, &user)` — field-level merge of user override.
3. Apply `KRIA_*` env overrides (12 direct vars + provider + voice).
4. `sync_legacy_llm_from_active_provider` — reconcile `llm.*` from `providers.active()`.

### 4.2 Where precedence actually breaks

**A. `merge_config` is incomplete (`config.rs:1575-1701`).**
Only ~9 of 28 sections have merge paths: `llm` (selected fields), `providers`,
`voice`, `classifier`, `safety.emergency_mode`, `agent`, `hardware`, `colab`,
`n8n`, `openclaw`. **Sections with NO merge path** (e.g. `memory`, `search`,
`server`, `ui`, `routing`, `image_generation`, `mcp`, `telegram`, `orchestrator`,
`executive`, `planner`, `mobile`, `remote_desktop`, `ntfy`, `capability`) are
taken **entirely from `default.toml`** — a user override for those is silently
ignored **unless** `default.toml` is absent (then user file is the sole base).

> Consequence: whether a user's `~/.kria/config.toml` value wins **depends on
> whether `config/default.toml` was found on disk**. In a dev checkout it is
> found (so many user overrides are dropped); in a bare install it is not (so the
> user file becomes the base and all fields apply). This is a hidden,
> environment-dependent precedence inversion.

**B. Whole-section replace vs field merge.** `voice`, `agent`, `colab`,
`classifier` use `if user.section != Section::default() { base = user.clone() }`.
A single non-default field pulls the **entire** section from the user file,
masking newer `default.toml` fields.

**C. Env can silently override the UI.** `KRIA_ACTIVE_PROVIDER`,
`KRIA_ACTIVE_MODEL`, `KRIA_LLM_MODE`, `KRIA_CLOUD_API_KEY`, voice vars, tier, etc.
are applied **after** the merged file. The UI is aware of only the LLM ones
(ProviderSettings shows a "locked by environment variables" message); all other
env overrides are invisible to the UI, so the UI shows the saved file value while
the runtime uses the env value.

**D. `sync_legacy_llm_from_active_provider` rewrites `llm.*`.** After merge+env,
the legacy `llm.routing_mode/active_model/cloud_*` fields are **overwritten** from
the active provider entry — so editing `llm.*` directly in the file has no effect
when a provider is active.

### 4.3 Effective runtime precedence (real)

```
KRIA_* env (subset)
   ↓ overrides
provider-derived llm.* (sync_legacy_llm_from_active_provider)
   ↓
~/.kria/config.toml   (ONLY for sections merge_config handles, AND only if default.toml present)
   ↓
config/default.toml   (base for un-merged sections)
   ↓
code Default
```

Plus per-request/session overrides read straight from env (bypass config):
`KRIA_IMAGE_MODE`, `KRIA_IMG_TIER`, GUI-cognition vars, STT/TTS sidecar vars, etc.

---

## SECTION 5 — Configuration Ownership

Single live handle: `AppState.config: Arc<RwLock<KriaConfig>>`
(`app_state.rs:60`). Everything else is a snapshot.

| Component | Source file | borrow / clone / cache | Reloads on change? |
|-----------|-------------|------------------------|--------------------|
| `AppState.config` | `app_state.rs:60` | **owns** `Arc<RwLock<KriaConfig>>` | authoritative; mutated by apply paths |
| `ModelRouter` | `providers.rs`, `runtime.rs` | snapshot via `from_config(&config)` | No — re-applied imperatively via `sync_active_provider` / `attach_server_manager` |
| Orchestrator (llama-server) | `runtime.rs`, `providers.rs` | snapshot of `orchestrator` + `llm` + provider | No — stopped/started on provider apply only |
| `AgentLoop` | `runtime.rs` | scalar snapshots (`agent.max_tool_rounds`, `min_confidence_to_act`, `clarify_threshold`) | No — needs restart |
| `ToolRegistry` | `runtime.rs` | snapshot at build; tools read env directly for some knobs | No |
| `ImageOrchestrator` | `runtime.rs` | snapshot of `image_generation` (+ `KRIA_IMAGE_MODE` env at read) | No — restart |
| `McpServerManager` | `runtime.rs`, `mcp.rs` | reads `config.read().mcp.servers` on demand | **Yes** — `apply_mcp_runtime_from_config` reconciles live |
| Semantic `Router` | `runtime.rs` | snapshot of `routing` | No |
| `MemoryStore` | `runtime.rs:88` | opens `kria.db`; `memory` limits snapshot | No |
| Voice pipeline | `voice.rs` | **re-reads config from disk each `start_voice`** | Partial hot-reload (per session) |
| OpenClaw `ContainerPool` | `runtime.rs` | snapshot `openclaw` at boot | No — restart |
| OpenClaw trust | `trust_runtime.rs` | process-wide static via `set_live_trust_config` | **Yes** — `openclaw_update_settings` re-pushes |
| GPU policy | `gpu_policy.rs` | process-wide atomics via `apply_settings` | **Yes** — `update_settings` re-pushes |
| Google runtime env | `app_commands.rs` | applied from config to env | **Yes** — on `update_settings` |
| `kria-server` `ServerState.config` | `lib.rs:21` | **owned immutable** `KriaConfig` | No |

**Summary:** 4 live-reloadable knobs (MCP, OpenClaw trust, GPU policy, Google env,
voice-per-session). Everything else is a boot-time snapshot requiring restart.

---

## SECTION 6 — Configuration Consumers (dependency graph)

```
config/default.toml ─┐
~/.kria/config.toml ─┼─► load_config() ─► KriaConfig (owned) ─► Arc<RwLock<KriaConfig>>
KRIA_* env ──────────┘        (config.rs)                          (AppState.config)
                                                                        │
        ┌───────────────┬──────────────┬─────────────┬────────────────┼──────────────┬───────────────┐
        ▼               ▼              ▼             ▼                 ▼              ▼               ▼
   ModelRouter     Orchestrator   AgentLoop    ToolRegistry     McpServerMgr   ImageOrch.    Voice pipeline
   (llm/providers) (orchestrator) (agent.*)   (safety/tools)   (mcp.servers)  (image_gen)   (voice.* + disk)
        │               │                                          │
        │               │                                          └─► reconcile() live
        └── sync_active_provider / attach_server_manager (imperative re-push)

Out-of-band consumers (read env / files directly, bypass KriaConfig):
   gpu_policy atomics · trust_runtime static · google env · GUI-cognition vars
   image_mode/img_tier env · STT/TTS sidecar vars · KriaSystemConfig (kria_config.toml)

Frontend:
   UI stores ──invoke("get_settings")──► get_settings  ──► config.read() (6 sections only)
   UI stores ──invoke("update_settings")─► update_settings ─► save() + partial live-apply
   ProviderSettings ──invoke("list_providers"/"get_active_llm_runtime"/"set_active_llm_selection")
   (many more per-feature commands — see Section 7.3)
```

### 6.1 Key consumer call sites
- `get_settings` (`app_commands.rs:706`): clones `config.read()`, **redacts**
  `llm.cloud_api_key` and every `providers[].endpoint.api_key`, returns JSON.
- `update_settings` (`app_commands.rs:751`): deserialize full config, **preserve**
  provider/model/llm fields from current, `save()`, re-apply GPU/MCP/Google.
- `apply_provider_selection` (`providers.rs`): validated stop/start runtime,
  `save()`, `*config.write() = desired`.
- `apply_mcp_runtime_from_config` (`mcp.rs:306`): `config.read().mcp.servers` →
  `manager.reconcile(...)`.

---

## SECTION 7 — Saving Flow

### 7.1 Generic settings save (Settings modal)

```
User edits field in SettingsModal          [ui/src/components/SettingsModal.tsx]
  └─ updateField(section, field, value) → draft signal
  └─ Save → appStore.saveSettings(draft())            [ui/src/stores/app.ts:3183]
        └─ invoke("update_settings", { settings })
              ▼ (Rust)
        update_settings(settings, state)               [app_commands.rs:751]
          1. from_value::<KriaConfig>(settings)
          2. PRESERVE from current live config:
                providers, llm.active_model, local_api_url, cloud_provider,
                cloud_api_key, cloud_model_id, cloud_endpoint, routing_mode, models
          3. sync_telegram_mcp_server_config / sync_google_workspace_server_config
          4. apply_google_runtime_env_from_config
          5. new_config.save()  → ~/.kria/config.toml (whole file, atomic rename)
          6. gpu_policy::apply_settings(...)            (live)
          7. *state.config.write() = new_config
          8. apply_mcp_runtime_from_config(state)       (live MCP reconcile)
        ▲ back in UI
  └─ OPTIMISTIC: setSettings(newSettings)  (NO re-fetch), applyTheme, applyUiRuntimePreferences
```

**Note:** the UI does **not** re-fetch after `update_settings`; it trusts the draft.
If the backend rejects/normalizes any field (or preserves provider fields the UI
tried to change), the UI shows the wrong value until the next `get_settings`.

### 7.2 Provider/model save (separate path)

```
ProviderSettings "Use this provider"        [ProviderSettings.tsx]
  └─ invoke("set_active_llm_selection", {providerId, modelId})
        ▼
  apply_provider_selection(...)              [providers.rs]
     - lock llm_runtime_apply_lock; reject if a local turn is running
     - local  → apply_local_runtime_selection (stop+start orchestrator, rebind router, rollback on fail)
     - remote → apply_external_provider_selection (test connection, rebind, stop local)
     - desired_config.save(); *config.write() = desired
     - publish "llm-runtime:apply" event
  └─ UI awaits refreshAll()  → RE-FETCH (correct sync)
```

`save()` (`config.rs:1354`): serialize **whole** `KriaConfig` to pretty TOML, write
`config.toml.tmp.<pid>`, atomic `rename` → `~/.kria/config.toml`.
**Secrets are written in plaintext** (no redaction on the save path; redaction is
only in `get_settings`).

### 7.3 Per-feature save commands (bypass update_settings)

`set_active_llm_selection`, `upsert_provider`, `remove_provider`,
`update_telegram_config`, `set_google_workspace_account`, `set_memory_enabled`,
`set_briefing_config`, `update_ironclad_config`, `openclaw_update_settings`,
`save_n8n_settings` + `save_n8n_api_key_secret`, `set_mobile_config`,
`set_gui_automation_enabled`, `set_gui_cognition_readiness_bypass`,
`add/remove/toggle_mcp_server`. Each has its own read/save/apply logic.

---

## SECTION 8 — Hot Reload Matrix

| Config area | Effect of change | Why |
|-------------|------------------|-----|
| `orchestrator.gpu_autoscale / cuda_reserve_mb / vram_volatility_cap_mb` | **Hot** | `gpu_policy::apply_settings` re-pushes atomics on save & boot |
| `mcp.servers` | **Hot** | `apply_mcp_runtime_from_config` reconciles manager live |
| `openclaw.trust.*` | **Hot** | `set_live_trust_config` process-wide snapshot |
| Google account/env | **Hot** | `apply_google_runtime_env_from_config` on save |
| Active provider / model (`set_active_llm_selection`) | **LLM restart** (managed) | stops/starts orchestrator, rebinds router with rollback |
| `voice.*` | **Voice restart (per session)** | `start_voice` re-reads config from disk |
| `llm.context_window`, `hardware.*`, `orchestrator.*` (other) | **App restart** | consumed once during `init_runtime` clamping/build |
| `agent.*` (rounds, confidence, thresholds) | **App restart** | AgentLoop snapshots scalars at build |
| `routing.*`, `image_generation.*` (non-env), `memory.*`, `classifier.*`, `executive/planner/uncertainty/...` | **App restart** | snapshot at construction |
| `server.*`, `mobile.*`, `remote_desktop.*` | **App/server restart** | bind + build once |
| `ui.theme / font_scale / high_contrast / reduce_motion` | **Hot (frontend)** | `applyTheme` / `applyUiRuntimePreferences` in UI |
| `KriaSystemConfig` (`kria_config.toml`) | **Restart** | loaded once by infra subsystems |
| Any `KRIA_*` env var | **Process restart** | env read only during `load_config` / at module read |

---

## SECTION 9 — Current SQLite Usage

### 9.1 Answer to the core question
**No configuration/settings is stored in SQLite today.** All user config is the
TOML file `~/.kria/config.toml`. A repo-wide search for a `settings`/`config`/
`app_config`/`kv_store` table returned zero matches. The only config-adjacent
tables are `preferences(key,value)` (session/UI prefs) and `briefing_config(id,json)`.

### 9.2 Databases

| DB file (under `~/.kria/`) | Opener | Source file | Tables | WAL | Migrations |
|----------------------------|--------|-------------|--------|-----|-----------|
| `kria.db` | `MemoryStore` | `memory/store.rs:106` | conversations, memory_facts, memory_links, **preferences**, audit_log, snippets, document_chunks, chat_media, + FTS5 virtuals | Yes | `CREATE TABLE IF NOT EXISTS` |
| `kria.db` (shared) | `TaskStore` | `tasks/store.rs:100` | tasks, reminders | Yes | idem |
| `kria.db` (shared) | `WorldModelStore` (PSDG) | `agent/world_model/store.rs:49` | world_facts, world_facts_archive | Yes | idem |
| `kria.db` (shared) | `AuditLogger` (desktop) | `safety/audit.rs:77`, wired `runtime.rs:1318` | audit_log | inherits | `ALTER TABLE` via `pragma_table_info` probes |
| `skills.db` | `ProductionSkillRegistry` | `openclaw/registry.rs:453` | skills, skill_health, skill_statistics, skill_dependencies, registry_events, capability_profiles, market_catalog, capability_grants_scoped, capability_edges | Yes | **Versioned** (`PRAGMA user_version` + migrations) |
| `skills.db` (companion) | `AuditLedger` | `openclaw/audit.rs:46` | HMAC-signed ledger | Yes | `CREATE TABLE IF NOT EXISTS` |
| `cpp_grants.db` | Capability `GrantStore` | `capability.rs:171`, `runtime.rs:1137` | capability grant tables | — | idem |
| `devices.db` | `DeviceRegistry` (+ `SecretsVault`) | `mobile/pairing.rs:92` | devices | — | idem |
| `audit.db` | `AuditLogger` (headless + remote-desktop) | `headless_runtime.rs:55`, `remote_desktop/mod.rs:33` | audit_log | inherits | idem |

Schema-defined but **not wired to a production file** (in-memory or test-only):
QuarantineRegistry, FailureAnalyzerStore, SkillCompiler, PromptOptimizer,
MlOrchestrator ledger, WorkflowTelemetryStore, ContactsStore, BriefingStore.

Not SQLite: `DecisionStore` (JSONL), vector index (`vectors.usearch`/`vectors.bin`),
n8n registries/backups (JSON under `~/.kria/n8n/`).

### 9.3 Unification assessment
- `kria.db` is already the de-facto shared DB (MemoryStore + TaskStore +
  WorldModelStore + desktop AuditLogger, all WAL, one physical file).
- Satellite files: `skills.db`, `cpp_grants.db`, `devices.db`, `audit.db`.
- **Redundancy:** `audit_log` is written to both `kria.db` (desktop) and `audit.db`
  (headless/remote-desktop) with the same schema — clearest consolidation target.
- All are SQLite + WAL with no cross-file FKs, so a settings/config table could be
  added to `kria.db` with negligible infra cost.

---

## SECTION 10 — Environment Variables

### 10.1 Vars actually read by `KriaConfig` load (override config fields)

| Env var | Overrides | Notes | Keep as | 
|---------|-----------|-------|---------|
| `KRIA_LLM_MODE` | `llm.routing_mode` | also gates legacy-llm sync | runtime override (advanced) |
| `KRIA_CLOUD_API_KEY` | `llm.cloud_api_key` | **secret** | secret-only (keychain) |
| `KRIA_ACTIVE_PROVIDER` | `providers.active_provider` | UI shows "locked" | runtime override |
| `KRIA_ACTIVE_MODEL` | active provider `active_model` | | runtime override |
| `KRIA_PROVIDER_API_KEY` / `KRIA_PROVIDER_<ID>_API_KEY` / `KRIA_OPENAI_API_KEY` / `OPENAI_API_KEY` / `KRIA_GEMINI_API_KEY` / `GEMINI_API_KEY` / `GOOGLE_API_KEY` / `KRIA_ANTHROPIC_API_KEY` / `ANTHROPIC_API_KEY` / `KRIA_OPENROUTER_API_KEY` / `OPENROUTER_API_KEY` / `KRIA_OPENCODE_API_KEY` | provider `endpoint.api_key` | **secrets** | secret-only (keychain) |
| `KRIA_TIER` | `hardware.tier` | | DB/runtime config |
| `KRIA_AGENT_AUTONOMY_PROFILE` / `KRIA_AGENT_MAX_TOOL_ROUNDS` / `KRIA_AGENT_MIN_CONFIDENCE` | `agent.*` | | DB/runtime config |
| `KRIA_COLAB_ENABLED` / `KRIA_COLAB_MCP_SERVER` | `colab.*` | | DB config |
| `KRIA_ENABLE_ONNX_L0` / `KRIA_ONNX_L0_MODEL_PATH` | `classifier.*` | | DB config |
| `KRIA_VOICE_MODE / _STT_ENGINE / _TTS_ENGINE / _LANGUAGE / _ENABLE_PARTIALS / _BARGE_IN / _ENABLED` | `voice.*` | applied in `apply_env_overrides` | DB config |
| `KRIA_MODELS_DIR` | `KriaPaths.models_dir` | only path env honored | keep (deploy) |
| `KRIA_SYSTEM_CONFIG_PATH` | `KriaSystemConfig` file path | | keep (ops) |
| `KRIA_GPU_AUTOSCALE / KRIA_CUDA_RESERVE_MB / KRIA_VRAM_VOLATILITY_CAP_MB` | GPU policy atomics | read at `apply_settings` time | runtime override |
| `KRIA_IMAGE_MODE` / `KRIA_IMG_TIER` / `KRIA_IMAGE_CLOUD_FALLBACK` | image gen at request time | env wins over config | runtime override |

### 10.2 Vars read directly by subsystems (bypass config entirely)
GUI cognition (`KRIA_GUI_COG_*`), STT/TTS sidecar (`KRIA_STT_*`, `KRIA_TTS_*`,
`KRIA_WHISPER_PARTIAL`), wake word (`KRIA_WAKE_*`), input backend
(`KRIA_INPUT_BACKEND`, `KRIA_UINPUT_*`, `KRIA_ENABLE_YDOTOOL_GUI_BACKEND`),
docker exec (`KRIA_EXEC_DOCKER_*`), fleet (`KRIA_FLEET_*` — HMAC keys = secrets),
OAuth client IDs/secrets (`KRIA_GOOGLE_/GITHUB_/MICROSOFT_CLIENT_*` = secrets),
`KRIA_VAULT_PASSPHRASE` (secret), diagnostics, `KRIA_USER_TZ`, `KRIA_UI_DIST`,
`KRIA_WORKSPACE_ROOT`, `KRIA_LLAMA_API_URL`, `KRIA_LLAMA_SERVER_PORT`,
`KRIA_SEARXNG_URL`, plus a large `KRIA_EVAL_*` and test-only family.

### 10.3 `.env.example` drift (finding)
`.env.example` documents many vars that the **Rust code never reads**:
`KRIA_SQLITE_PATH`, `KRIA_AUDIT_LOG_PATH`, `KRIA_ROLLBACK_DIR`, `KRIA_LOG_DIR`,
`KRIA_PLUGINS_DIR`, `KRIA_WORKFLOWS_DIR`, `KRIA_KNOWLEDGE_DIR`,
`KRIA_MAX_CONTEXT_TURNS`, `KRIA_TOOL_TIMEOUT_SECONDS`, `KRIA_HITL_TIMEOUT_SECONDS`,
`KRIA_EMERGENCY_MODE`, `KRIA_WAKE_WORD`, `KRIA_LANGUAGE`, `KRIA_TELEGRAM_*`, etc.
Paths are actually hardcoded in `KriaPaths` (only `KRIA_MODELS_DIR` is honored).
This `.env.example` is aspirational/stale and misleads users into setting vars
that do nothing.

### 10.4 Recommendation classes
- **Secret-only (move to OS keychain / secret file):** all `*_API_KEY`,
  `*_CLIENT_SECRET`, `KRIA_FLEET_HMAC_*`, `KRIA_VAULT_PASSPHRASE`, `KRIA_CLOUD_API_KEY`.
- **Keep as env (deploy/ops):** `KRIA_MODELS_DIR`, `KRIA_SYSTEM_CONFIG_PATH`,
  `KRIA_UI_DIST`, `KRIA_WORKSPACE_ROOT`, `KRIA_LOG_FILTER`, eval/test family.
- **Should become DB/runtime config (stop being env):** voice, agent, colab,
  classifier, tier, GPU policy, image mode — these duplicate UI-editable fields
  and cause the "UI vs env" mismatch.
- **Delete from `.env.example`:** everything in 10.3 that code ignores.

---

## SECTION 11 — Current Problems

1. **Multiple sources of truth for one screen.** `get_settings` covers only 6
   sections; providers, hardware, memory-toggle, telegram, google, colab, n8n,
   mobile, ironclad, openclaw, GUI-automation each have their own load/save
   command. The Settings UI stitches ~20 independent sources.
2. **Environment-dependent precedence inversion.** `merge_config` drops user
   overrides for most sections **when `config/default.toml` exists**, but honors
   them when it doesn't. Behavior differs between dev checkout and installed app.
3. **Incomplete `merge_config`.** ~19 of 28 sections have no user-merge path.
4. **Whole-section replace** (`voice`, `agent`, `colab`, `classifier`) masks new
   default fields when the user changed one field.
5. **Snapshot staleness.** Subsystems built from clones before the `Arc<RwLock>`
   exists; runtime edits do not reach them without an explicit apply path (only 4
   knobs have one).
6. **Optimistic UI save with no re-fetch.** `saveSettings` trusts the draft; the
   backend deliberately preserves provider/llm fields, so the UI can display
   values the backend didn't accept.
7. **Legacy `llm.*` vs `providers` duality.** `sync_legacy_llm_from_active_provider`
   overwrites `llm.*` from the active provider, and `update_settings` force-preserves
   provider/llm fields — two competing representations of "which model".
8. **Env silently overrides UI** for non-LLM fields (voice, tier, agent, colab)
   with no UI indication.
9. **Plaintext secrets on disk.** `save()` writes API keys unredacted to
   `~/.kria/config.toml`; redaction exists only on the read path.
10. **`.env.example` drift** — documents ~20 vars the code ignores (Section 10.3).
11. **Duplicate audit DBs** — `audit_log` in both `kria.db` and `audit.db`.
12. **No config file watcher.** External edits to `~/.kria/config.toml` are not
    detected; require restart.
13. **Two unrelated config files** (`config.toml` vs `kria_config.toml`) with
    similar names — easy to confuse.
14. **Repeated parsing / repeated `KriaPaths::resolve()`** on many command calls.
15. **Hardcoded frontend mock** (`mcpCatalog`) renders as if configurable but never
    persists.
16. **Server has no live config** — `ServerState.config` is immutable; UI-style
    reconfiguration is desktop-only.

---

## SECTION 12 — Pain-Point Analysis

| Symptom | Root cause | Affected files | Components | Severity |
|---------|-----------|----------------|------------|----------|
| **LLM server not reachable** | Provider/model apply is a stop-start orchestrator dance with rollback; if a local turn is running it's rejected; env `KRIA_ACTIVE_PROVIDER`/`KRIA_LLM_MODE` can pin a runtime the UI can't change; `llm.local_api_url` snapshot vs live orchestrator port | `providers.rs` (apply_*), `llm/orchestrator/*`, `config.rs` (sync_legacy) | ModelRouter, Orchestrator | High |
| **Settings change unexpectedly** | `sync_legacy_llm_from_active_provider` rewrites `llm.*`; `update_settings` force-preserves provider/llm; whole-section replace in merge | `config.rs:1370-1701`, `app_commands.rs:751` | config load, update_settings | High |
| **Frontend shows one value, backend uses another** | 20 independent load sources + optimistic save w/o re-fetch + env overrides invisible to UI | `ui/src/stores/app.ts`, `SettingsModal.tsx`, `ProviderSettings.tsx`, `config.rs` env block | UI stores, get/update_settings | High |
| **Configuration resets** | Environment-dependent `merge_config` drop (default.toml present ⇒ user override for unmerged sections ignored) | `config.rs:1575-1701` (merge_config), `1282` (load) | KriaConfig::load | High |
| **Restart required unnecessarily** | Only 4 knobs have live-apply; all other sections snapshot at boot | `runtime.rs` (init_runtime build order) | all subsystems | Medium |
| **Unexpected overrides** | `KRIA_*` env applied last with no UI signal (except LLM) | `config.rs` env block, `apply_provider_env_overrides`, `VoiceConfig::apply_env_overrides` | load_config | Medium |
| **Inconsistent provider behavior** | Provider config lives in `providers` **and** mirrored into `llm.*`; env can override either | `config.rs` (providers + sync), `providers.rs` | ModelRouter | High |
| **Secrets leaking / lost** | Plaintext save; redaction only on read; keys also injected via many env names | `config.rs:706` (redact), `config.rs:1354` (save) | get/update_settings, save | High (security) |

---

## SECTION 13 — Current Strengths (do NOT redesign away)

1. **Single canonical struct `KriaConfig`** with `#[serde(default)]` everywhere —
   forward/backward-compatible parsing; missing fields never crash.
2. **Atomic save** (tmp + rename) — no partial-write corruption.
3. **Redaction on the read/UI path** (`get_settings`) — keys aren't shipped to the
   frontend.
4. **Provider-apply has real validation + rollback** (`apply_local_runtime_selection`
   restores previous runtime on failure) and a concurrency lock.
5. **`get_diagnostics`-style config validation** exists for voice
   (`VoiceConfig::validate`) — a good pattern to generalize.
6. **`kria.db` already shared with WAL** — solid base for a config table.
7. **Skill registry has a real versioned migration system** (`PRAGMA user_version`)
   — a proven template for schema evolution.
8. **Hardware tune-for-tier safety clamps** (`OrchestratorConfig::tune_for_tier`)
   prevent OOM freezes — keep this adaptive layer regardless of storage backend.
9. **`.env` still useful for true secrets and deploy/ops** — should remain for that
   narrow purpose.

---

## SECTION 14 — Future Recommendation (post-investigation)

Goal restated: kill UI-vs-file drift and the "settings change unexpectedly"
class of bugs, without breaking deploy/provisioning.

**Recommended target: layered ConfigService with SQLite as the user layer.**

```
┌──────────────────────────────────────────────────────────────┐
│ ConfigService (kria-core)                                      │
│  - single reader/writer, RwLock<KriaConfig> + change event bus │
│  - resolve(): defaults < default.toml < DB(user) < secrets < env│
│  - subscribe(section) → live updates                            │
└───────────────┬───────────────────────────┬───────────────────┘
   reads         │                            │  writes
                 ▼                            ▼
   config/default.toml (git, ops)     kria.db `config` table (user layer)
   secrets: OS keychain / secret file  (key TEXT, value JSON, updated_at)
   env: KRIA_* (ops/CI + secrets only, narrowed)
```

- **Keep `config/default.toml`** — file-based, git-tracked, ops/dev baseline.
  Do NOT remove file config entirely; deployment/fleet/server need it.
- **Move the user layer (`~/.kria/config.toml`) into SQLite** (`kria.db`, new
  `config` table, JSON per section + `updated_at`). This is the layer the UI
  writes and where all drift/conflicts happen. Atomic, queryable, versionable,
  auditable.
- **OS keychain (or an existing `SecretsVault`) for secrets** — remove plaintext
  keys from disk; env secret vars become fallback only.
- **Narrow env to ops + secrets.** Voice/agent/colab/classifier/tier/GPU/image env
  vars should stop shadowing UI fields (or become explicit, UI-surfaced overrides
  like the existing LLM "locked by env" banner).
- **Introduce a `ConfigService` with an event bus** so subsystems `subscribe`
  instead of snapshotting — expands the hot-reload set beyond today's 4 knobs and
  removes the "restart required" surprises.
- **One source of truth per screen.** Fold the ~20 per-feature load/save commands
  behind `get_config`/`patch_config(section)` so the UI reads/writes one contract;
  re-fetch (or push via event) after every save (kill optimistic-only save).
- **Resolve the `llm.*` vs `providers` duality** — pick `providers` as canonical,
  derive `llm.*` as a read-only view, stop force-preserving in `update_settings`.
- **Unify audit DBs** — point headless/remote-desktop `AuditLogger` at
  `paths.db_path`; retire `audit.db`.

Why not "SQLite only, delete all files": breaks `config/default.toml` dev/ops
baseline, fleet/server file-mount configs, dotfile sync, and stateless container
deploys. Layered model keeps those while removing the drift source.

---

## SECTION 15 — Migration Plan

**Estimated complexity: Medium-High (multi-phase, mostly additive).**

### Phase 1 — ConfigService seam (no storage change) — LOW risk
- Introduce `ConfigService` wrapping the existing `Arc<RwLock<KriaConfig>>` +
  a `tokio::sync::broadcast` change bus. All commands go through it.
- Add `get_config`/`patch_config(section, json)` Tauri commands; keep old
  commands as thin shims.
- UI: switch Settings to re-fetch (or event) after save; remove optimistic-only.
- **Rollback:** revert to direct `state.config` access; no data migrated.

### Phase 2 — SQLite user layer + secrets vault — MEDIUM risk
- Add `config` table to `kria.db` (JSON per section + `updated_at`), versioned via
  `PRAGMA user_version` (reuse skill-registry migration pattern).
- One-time importer: read existing `~/.kria/config.toml` → DB; move secrets to
  keychain/vault; keep the TOML as a read-only backup (`config.toml.bak`).
- `load()` order becomes: defaults < default.toml < **DB** < secrets < env.
- **Rollback:** feature-flag `KRIA_CONFIG_BACKEND=toml|sqlite`; on `toml`, ignore
  DB. Keep the `.bak` for manual restore.

### Phase 3 — reactive subsystems + cleanup — MEDIUM risk
- Convert snapshot consumers to `ConfigService::subscribe(section)` so more fields
  hot-reload; document the remaining true-restart set.
- Narrow env (secrets/ops only); prune stale `.env.example` vars.
- Unify audit DBs; collapse per-feature commands.
- **Rollback:** subsystems fall back to boot snapshot if no subscription;
  audit-DB change is additive (dual-write during transition).

### Risk register
| Risk | Mitigation |
|------|-----------|
| Secret loss during vault migration | Dual-read (vault → env → old file) during Phase 2; verify before deleting plaintext |
| DB corruption blocks all config | WAL + atomic tx; fallback to `default.toml` + defaults if DB open fails (mirror current graceful degrade) |
| Fleet/server relies on file config | Keep `default.toml` authoritative for `kria-server`; DB layer is desktop-user-only |
| Behavior change from fixed precedence | Ship behind `KRIA_CONFIG_BACKEND` flag; canary on dev builds first |
| Hidden env dependencies (evals/CI) | Keep `KRIA_EVAL_*` untouched; only narrow user-facing knobs |

---

*End of audit. No code was modified.*
