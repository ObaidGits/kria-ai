# Settings & Configuration Revamp — Chat Analysis & Decision Log

> **Purpose:** Permanent, self-contained reference distilled from the full design
> conversation. Future sessions should read THIS instead of re-researching the
> codebase or re-reading the chat. Every architectural decision, constraint,
> pain point, enhancement, edge case, trade-off, and the verified current-state
> facts are captured here.
>
> Companion documents:
> - `/media/obaid/SSD/KRIA/CONFIGURATION_ARCHITECTURE.md` — full 15-section audit of the CURRENT system.
> - `requirements.md`, `design.md`, `tasks.md` — the actionable spec.

---

## 1. Origin & Intent

**User's core problem:** UI Settings and file settings conflict. User asked whether
to remove file-based config entirely and move to a DB-based config configurable from
the KRIA UI. Investigation grew into a full re-architecture of configuration plus a
new capability: **change settings by natural-language prompt**.

**Two headline goals:**
1. **Kill config drift** — one source of truth per layer; UI, prompt, and runtime never disagree.
2. **Prompt-driven settings** — user can change settings conversationally, with a clear
   split between *temporary* (per-request) and *permanent* (approved) changes.

**Guiding constraints from the user:**
- Do it all at once so the work is "locked done."
- Architecture must survive future settings being added/removed with minimal effort.
- Must NOT be coupled to the main memory subsystem (being revamped separately).
- Must clearly distinguish **query prompts** from **settings-change prompts** — zero conflict.
- Production-grade, future-proof, scalable, intelligent.

---

## 2. Verified Current-State Facts (from code audit — do not re-verify)

### 2.1 Config structure
- Single canonical struct `KriaConfig` — `crates/kria-core/src/config.rs:8` — **28 sections**,
  `#[derive(Serialize, Deserialize, Default)]`, `#[serde(default)]` on every section.
- Separate unrelated `KriaSystemConfig` (`config.rs:806-882`) — infra QoS/pool/snapshot,
  loaded from `kria_config.toml` via the `config` crate. NOT merged into `KriaConfig`.

### 2.2 Load / save / merge
- `KriaConfig::load(None)` (`config.rs:1282`): find `config/default.toml` (walk up from
  exe+CWD) as base → merge `~/.kria/config.toml` via `merge_config` → apply `KRIA_*` env →
  `sync_legacy_llm_from_active_provider`.
- `KriaConfig::save()` (`config.rs:1354`): serialize **whole** struct to pretty TOML,
  atomic tmp+rename → `~/.kria/config.toml`. **Secrets saved in plaintext.**
- `merge_config` (`config.rs:1575-1701`): **INCOMPLETE** — only ~9 of 28 sections merge
  user overrides (`llm` selected fields, `providers`, `voice`, `classifier`,
  `safety.emergency_mode`, `agent`, `hardware`, `colab`, `n8n`, `openclaw`). ~19 sections
  (`memory`, `search`, `server`, `ui`, `routing`, `image_generation`, `mcp`, `telegram`,
  `orchestrator`, `executive`, `planner`, `mobile`, `remote_desktop`, `ntfy`, `capability`,
  etc.) have NO merge path → user overrides silently dropped **when default.toml exists**.
- **Environment-dependent precedence inversion:** if `default.toml` found, unmerged sections
  come from it (user override ignored); if not found, user file becomes sole base (overrides apply).
  Behaviour differs between dev checkout and installed app.
- Whole-section replace (`voice`, `agent`, `colab`, `classifier`): one non-default field pulls
  the ENTIRE section from the user file, masking newer default fields.

### 2.3 Ownership & propagation
- Single live handle: `AppState.config: Arc<RwLock<KriaConfig>>` (`app_state.rs:60`), tokio RwLock.
- Startup is `crates/kria-desktop/src/commands/runtime.rs::init_runtime()` (NO `lib.rs`).
  `main.rs:82` spawns it. Config is an owned `mut KriaConfig` for nearly all of init_runtime;
  wrapped into `Arc<RwLock<>>` LATE (~runtime.rs:1613) AFTER all subsystems built from
  **clones/snapshots**. So subsystems do NOT observe later edits unless an explicit apply path re-pushes.
- `kria-server`: `KriaConfig::load(None)`, stored as **immutable owned** `ServerState.config`
  (`lib.rs:21`). No RwLock, no reload, no watcher. `update_settings` route is a no-op stub.
- `KRIA_CONFIG` env var is referenced in docs but **NOT read** by Rust code. Only
  `KRIA_SYSTEM_CONFIG_PATH` (for KriaSystemConfig) and `KRIA_MODELS_DIR` (paths) are honored.
- NO config file watcher (notify) exists.

### 2.4 What hot-reloads today (only these)
- GPU policy (`gpu_policy::apply_settings` — atomics, re-pushed on every save & boot).
- MCP servers (`apply_mcp_runtime_from_config`, `mcp.rs:306`).
- OpenClaw trust (`set_live_trust_config`).
- Google runtime env.
- Voice: re-reads config from disk each `start_voice` (per-session).
Everything else = snapshot at construction → **restart required**.

### 2.5 Provider/model = a runtime EFFECT, not plain config
- `update_settings` (`app_commands.rs:751`) deliberately force-preserves `providers` +
  `llm.{active_model,local_api_url,cloud_*,routing_mode,models}` from the live config, then
  `save()`, then re-applies GPU/MCP/Google.
- Model switching goes through `apply_provider_selection` (`providers.rs`): validated
  stop/start of orchestrator with rollback + concurrency lock (`llm_runtime_apply_lock`);
  rejects switch while a local turn is running. Emits `llm-runtime:apply` event.
- `sync_legacy_llm_from_active_provider` overwrites `llm.*` from the active provider →
  editing `llm.*` directly has no effect when a provider is active.

### 2.6 Frontend coupling
- UI reads nested config fields directly off `get_settings` JSON (no adapter):
  `settings().llm.routing_mode` (`App.tsx:121`), `settings().ui.theme`, `settings().server.*`,
  plus SettingsModal draft over `llm/voice/safety/ui/search/hardware/orchestrator`.
- `saveSettings` sends the **whole** object back via `update_settings` and is **optimistic**
  (no re-fetch) — UI can show values the backend didn't accept.
- `get_settings`/`update_settings` JSON shape is a **HARD Tauri contract** (steering rule:
  don't change command/event names). SQLite must stay behind the identical serde shape.
- ~20 independent per-feature load/save commands feed the Settings UI (providers, hardware,
  memory-toggle, telegram, google, colab, n8n, mobile, ironclad, openclaw, gui-automation).
  `get_settings` itself covers only 6 sections effectively.
- Pure-frontend prefs in localStorage (`kria_assistant_frontend_prefs`, `kria_labs_frontend_prefs`,
  `kria_mcp_catalog` — the last is a HARDCODED mock list, never persisted).

### 2.7 SQLite reality
- **No configuration is in SQLite today.** All user config = `~/.kria/config.toml`. Only
  config-adjacent tables: `preferences(key,value)` and `briefing_config(id,json)` in `kria.db`.
- `kria.db` is the shared DB (MemoryStore + TaskStore + WorldModelStore + desktop AuditLogger,
  all WAL). Satellites: `skills.db`, `cpp_grants.db`, `devices.db`, `audit.db`.
- Redundancy: `audit_log` written to both `kria.db` (desktop) and `audit.db` (headless/remote).
- OpenClaw registry (`openclaw/registry.rs`) has a REAL versioned migration system
  (`PRAGMA user_version` + `MIGRATIONS`) — the pattern to copy for a config DB.

### 2.8 Config ↔ Memory independence (CONFIRMED)
- `config.rs` has ZERO `MemoryStore` refs. `MemoryStore::open` takes a path (`runtime.rs:85`),
  not `KriaConfig`. `embedding_dim` hardcoded 384 (`runtime.rs:667`) — ignores
  `config.memory.embedding_dim` (a DEAD config field).
- `MemoryConfig` fields consumed in exactly ONE non-test place: `analytics.rs:348-349`
  (telemetry snapshot only, not behaviour).
- **Verdict: settings and memory are decoupled. The memory revamp cannot break settings, and
  settings can be built now.**

### 2.9 Reusable infrastructure (all already exists — hook points)
| Need | Component | File | Hook |
|------|-----------|------|------|
| Risk classification | `PolicyEngine` / `RiskLevel{Green,Yellow,Red,Black}` | `safety/policy.rs` | `evaluate()`; unknown action ⇒ RED (fail-safe) |
| HITL approval | `HitlGateway` | `safety/hitl.rs` | `request_approval_with_id()`, `respond()`, `subscribe()` |
| Audit (hash-chained) | `AuditLogger` | `safety/audit.rs` | `log(session_id, action, params, decision, …)` |
| Grammar-constrained JSON | `chat_with_grammar` + `capability_schema`/`validate_capability_json` | `llm/local.rs`, `platform/intent/grammar.rs` | build JSON schema for patch payload; toggle `routing.grammar_enabled` |
| Tool registry | `ToolRegistry::register` | `tools/registry.rs` | register `config_patch` tool |
| Event bus (pub/sub) | `EventBus` (tokio broadcast) | `automation/event_bus.rs` | `publish("config.…")`, `subscribe_filtered("config.")` |
| Hot-reload reference pattern | `gpu_policy::apply_settings` (atomics + apply on save) | `llm/orchestrator/gpu_policy.rs` | replicate for live-apply subsystems |
| In-session context (topic/self-reference) | `RoutingContext` | `routing/context.rs` | `last_domain`, `turn_count_in_topic`, `correction_pending` — NOT persistent memory |
| Cross-platform secrets | existing `SecretsVault` (devices.db) + n8n secret-file migration; `keyring-rs` standard | — | store secret reference in config, value in vault/keychain |

### 2.10 Env var inventory
- ~150 unique `KRIA_*` vars across crates (many test/eval-only: `KRIA_EVAL_*`).
- Config-affecting load-time vars: `KRIA_LLM_MODE`, `KRIA_CLOUD_API_KEY`, `KRIA_ACTIVE_PROVIDER`,
  `KRIA_ACTIVE_MODEL`, `KRIA_PROVIDER_*_API_KEY`, `KRIA_TIER`, `KRIA_AGENT_*`, `KRIA_COLAB_*`,
  `KRIA_ENABLE_ONNX_L0`, `KRIA_VOICE_*`, `KRIA_MODELS_DIR`, GPU policy vars, `KRIA_IMAGE_MODE`.
- `.env.example` DRIFT: documents ~20 vars the code never reads (`KRIA_SQLITE_PATH`,
  `KRIA_AUDIT_LOG_PATH`, `KRIA_ROLLBACK_DIR`, `KRIA_MAX_CONTEXT_TURNS`, `KRIA_EMERGENCY_MODE`,
  `KRIA_WAKE_WORD`, `KRIA_TELEGRAM_*`, etc.). Misleads users.

---

## 3. Design Decisions (agreed in chat)

### 3.1 Storage model — LAYERED, not "delete all files"
- **KEEP `config/default.toml`** — file-based, git-tracked, dev/ops/fleet baseline, read-only at runtime.
- **MOVE the user layer** (`~/.kria/config.toml`) **into SQLite** (`kria.db`, new `config` table).
  This is the layer the UI writes and where all drift happens.
- **Secrets → OS keychain / SecretsVault**; config stores a secret *reference*, never the value.
- **Narrow env** to ops + secrets only; stop shadowing UI-editable fields.
- Rejected "SQLite only, delete all files" because it breaks default.toml dev/ops baseline,
  fleet/server file-mount configs, dotfile sync, and stateless container deploys.

### 3.2 Precedence (fixed & complete)
```
code default < config/default.toml < DB(user) < secrets < env(ops) < request-override(temporary)
```
- Fix `merge_config` to cover ALL sections (or become obsolete once DB is field-level).
- Precedence must be deterministic and NOT depend on whether default.toml exists.

### 3.3 ConfigService (the decoupling firewall)
- Single serialized writer wrapping `Arc<RwLock<KriaConfig>>` + change event bus.
- API: `get()`, `get_section()`, `patch(section, field, value, source)`, `subscribe()`.
- **Keeps `KriaConfig` serde shape identical** → `get_settings`/`update_settings` unchanged →
  zero frontend breakage. SQLite is an internal detail.
- Subsystems SUBSCRIBE to `config.<section>.changed` instead of snapshotting → memory/any
  future subsystem is just another subscriber → nothing to rewire when memory is revamped.
- Monotonic config **version number** so laggy (lossy broadcast) subscribers reconcile by
  re-reading current config rather than trusting they saw every delta.

### 3.4 Self-updating schema
- Derive schema from `KriaConfig` via `schemars` (`#[derive(JsonSchema)]`).
- One schema feeds: UI render, agent tool spec, validation, approval preview.
- Per-field annotations: `risk`, `hot_reload?`, `valid_values`, `synonyms`.
- **Fail-closed:** any un-annotated field = high-risk + NOT prompt-changeable. Opt IN, never out.
- Add a field to the struct → UI, prompt-agent, validation all update automatically.

### 3.5 Prompt-driven settings
- **Temporary override** (e.g. "generate this using local AI") = turn-scoped param at TOP of
  precedence, whitelisted safe fields only, auto-reverts after the turn. NEVER a DB write.
- **Permanent change** (e.g. "change theme to dark") = `config_patch` tool →
  risk-gate (`PolicyEngine`) → HITL approve popup → persist → event bus → live apply.
- LLM emits grammar-constrained JSON `{section, field, value, scope}` against the derived
  schema; **LLM never writes disk** — deterministic funcs validate/apply/save/revert.
- Provider/model changes route through the EXISTING apply service (`apply_provider_selection`),
  NOT a raw config write (they get clobbered otherwise).

### 3.6 Intent disambiguation (query vs action vs discussion)
Layered, cheapest-first:
1. **Settings-domain gate** (semantic router centroid) — is it even about config?
2. **Self-reference gate** — about KRIA itself, or about the topic/code being discussed?
   Uses `RoutingContext` (topic continuation + subject markers "your/the app" vs "I/my/this code"
   + schema-grounding). This resolves the "should I use gemini" in a code chat problem.
3. **Action-vs-query** (imperative vs interrogative).
4. **Temp-vs-permanent scope** (cue words; default temp when unclear, else ask).
5. **Schema-grounded extraction** — no valid field maps ⇒ it was a query, fall back to answer.
6. **Ambiguity ⇒ ONE clarifying question, never guess.**
- **AND-rule to act:** imperative + self-referential + schema-grounded + topic-break. Miss any ⇒ answer/clarify.
- **Fail toward query, not action** (wrong answer is recoverable; wrong config change is the original pain).

---

## 4. Non-negotiable Hard Rules (security & correctness)

1. Config write is **only ever an explicit tool call** — answering a question can never mutate config.
2. **External content (files, web, tool output) can NEVER trigger a config change** — prompt-injection wall.
3. Auth / network / safety / remote-desktop / secrets fields → hard-gated (RED/BLACK), never
   auto-applied, never temp-overridable, never LLM-decided without explicit human approval.
4. **Field-level patch only** — never round-trip the whole config blob (silent data-loss risk).
5. Provider/model changes go through the existing runtime-apply service, not raw config writes.
6. Versioned schema + migrations required before any DB-backed storage.
7. Every field the UI/prompt exposes must be **verified wired** (no dead config like `embedding_dim`).
8. Single serialized writer + version-checked concurrency; UI re-fetches/subscribes after save
   (kill optimistic-no-refetch).
9. Secrets never stored plaintext; never echoed into chat history/logs (redact on input too).
10. Temporary overrides auto-revert even on turn error/crash; must not leak to the next turn.

---

## 5. Edge-Case Catalogue (test-set source of truth)

Grouped; each is a required test case. Full matrix in `design.md` §Testing.

- **A. Query vs action:** "what is dark mode?" (query) vs "turn on dark mode" (action);
  "I don't like dark mode" (complaint, no-op); "dark mode" (too vague ⇒ clarify).
- **B. Self-reference/topic collision:** "should I use Gemini for my project?" (advice);
  "use gemini" mid-code-chat (ambiguous ⇒ clarify); "change the API key in my code" (code edit,
  not KRIA's key); "set temperature to 0.7" (LLM config OR code).
- **C. Multi-intent:** "switch to dark mode and generate a cat" (settings + task);
  "turn on voice, dark mode, and wake word" (3 settings — batch approve?).
- **D. Temp vs permanent:** "generate this one using local AI" (temp) vs "always use local AI"
  (permanent); no cue ⇒ default temp or ask; crash mid-turn ⇒ must revert; scope leak to
  follow-up prompt must not happen.
- **E. Value mapping:** "make it faster" (ambiguous target); "use the good model" (subjective);
  synonyms (dark/night/lights off ⇒ dark); out-of-range ("font scale 50" ⇒ clamp/reject);
  nonexistent provider ("Llama 5" ⇒ error not silent).
- **F. Security/privilege:** "turn off authentication" (RED); "enable remote desktop" (high);
  key typed in chat (redact from history/logs); "disable safety" (BLACK); injection from a
  document (must NOT trigger); "approve all future changes automatically" (dangerous meta).
- **G. Confirmation flow:** stale "yes" binding; ignored/timed-out popup; "no wait" cancel;
  rapid double-command stacking; "do it" with nothing pending.
- **H. State sync/concurrency:** prompt change while UI open (UI must reflect live); UI+prompt
  race (version check); save ok but live-apply fails (rollback must cover prompt path);
  env-locked field (honor "locked by env" banner in prompt path too).
- **I. Referential:** "change it back", "same as yesterday" (cross-session ⇒ needs memory),
  "undo that" (audit-based, works now), "revert everything today" (bulk scope).
- **J. Negation/conditional:** "don't use cloud AI" (⇒ local); "use Gemini unless it's down"
  (conditional, no field); "never turn on voice" (setting or preference?); "stop using local AI".
- **K. Voice:** STT mishears (validation catches); "turn on voice" when already on (no-op);
  no popup hands-free (voice confirmation flow); dictation vs command.
- **L. Multilingual:** "theme ko dark karo" (Hinglish ⇒ same mapping); mixed-language.
- **M. Timing/lifecycle:** change during active turn (reject like local runtime does);
  restart-required change (tell user, don't fake live); temp override + turn error ⇒ clean revert.
- **N. False positives (MUST NOT trigger — top regression guard):** "I'll change my approach to
  dark themes in CSS"; "enable the feature flag in the code"; "set up a meeting"; "turn on the
  lights"; "switch branches".
- **O. Idempotency:** "switch to dark" when already dark (graceful no-op).
- **P. Discovery/read-back:** "what are my current settings?"; "what can I change by voice?".
- **Q. Recovery:** bad change bricks something (recovery path without UI); corrupt config ⇒
  fail-closed to defaults.

---

## 6. Scope Boundaries

**In scope (build now):** DB user layer, ConfigService + event bus, complete precedence,
versioned migrations, derived schema, secrets vault, `config_patch` tool, intent
disambiguation (query/action/discussion, self-reference gate), temp vs permanent, risk gating,
audit + undo, UI contract preservation, dead-config cleanup, `.env.example` cleanup, audit-DB unify.

**Deferred (needs main memory revamp — bolt on later, additive, same `kria.db`):**
Cross-session referential recall ("same as yesterday", "revert everything last week"). v1
handles same-session + audit-based undo only.

**Explicitly out of scope for v1:** prompt-driven reconfiguration of `kria-server` (stays
file/env-driven for fleet determinism); making risk tiers config-tunable (kept compile-time for safety).

---

## 7. Trade-offs Recorded

- **DB vs file:** DB for user-mutable per-user config (web/SO consensus); file for boot/ops/fleet.
  Layered keeps both; avoids losing git-diffable config and stateless deploys.
- **Whole-blob save vs field patch:** field patch chosen — avoids data loss, concurrency races,
  and stale-draft rollbacks. Cost: more granular commands (mitigated by generic `patch_config`).
- **LLM freedom vs safety:** LLM only fills a grammar-constrained form; deterministic funcs
  enforce. Fail-closed on unknown fields. Accepts that value-mapping semantics ("dark"→theme)
  still need LLM knowledge or per-field synonym hints.
- **Hot-reload everywhere vs restart:** expand hot-reload via event bus, but explicitly document
  the true-restart set (tier, embedding, bind host) rather than fake live application.
- **Event bus lossiness:** accepted (lightweight) + mitigated with config version reconciliation.

---

## 8. Pre-Implementation Verification Findings (code-verified — corrections to §2.9)

A second verification pass checked every infra assumption against real code. Results below
**supersede** the optimistic claims in §2.9 where they conflict. Legend: PASS / PARTIAL / FAIL.

1. **config module — PARTIAL.** `crates/kria-core/src/config.rs` is a **single 2331-line file**.
   Adding `config/service.rs`, `config/store.rs`, etc. requires converting `config.rs` →
   `config/mod.rs` FIRST (Rust forbids both). Module path `crate::config::` is unchanged ⇒ no
   import churn. Treat as an explicit Wave-0 mechanical refactor verified by `cargo check`.

2. **EventBus — FAIL (docs were wrong).** `AppState.event_bus` (`app_state.rs:73`,
   `runtime.rs:239`) is `infra::EventBus` — a **typed `KriaEvent` enum**, capacity **256**,
   with `publish(KriaEvent)` / `subscribe()` only. It has **NO string topics and NO
   `subscribe_filtered`**. The topic/string `automation::EventBus` that the design describes
   **exists but is unused and NOT in AppState**. Decision required: (a) extend `KriaEvent` with a
   `ConfigChanged { section, version }` variant (least wiring, reuses the wired bus), or (b) add
   `automation::EventBus` to AppState. Either way the bus is a **bounded lossy broadcast** ⇒ the
   monotonic config-version reconciliation is **mandatory**, not optional.

3. **HITL — PARTIAL.** Full API confirmed (`safety/hitl.rs:46-150`); stored in AppState
   (`runtime.rs:1210`, timeout **300s**); callable from background/commands (precedent:
   `local_api.rs:3139`). UI delivery is via the agent stream emitting
   `StreamEvent::ApprovalRequired` → Tauri `agent:approval_required`, answered by
   `approve_action`/`deny_action` (`app_commands.rs:144/171`). **Caveat:** the gateway's own
   request channel is NOT drained to the UI outside the agent stream, so a `config_patch`
   approval triggered outside an agent turn must **emit its own `agent:approval_required`** event.

4. **Grammar decode — PARTIAL (local-only).** `chat_with_grammar` (`local.rs:878`) posts real
   `response_format:{json_schema,strict}` for **local llama.cpp only**. The default trait impl
   (`llm/mod.rs:497`) and CloudBackend **ignore the schema and fall back to unconstrained chat**.
   ⇒ On a cloud provider, prompt-driven config extraction is NOT constrained. Use
   `chat_structured` / `chat_structured_with_mode` (json_schema→json_object→tool-calling) +
   **strict validate + reject-and-reask**; never apply an unvalidated patch.

5. **Secrets vault — PARTIAL.** Existing `SecretsVault` at `crates/kria-core/src/auth/vault.rs`
   (`auth::{SecretsVault, SecretEntry}`). Write method is **`set`** (not `put`); `open_default()`
   → `~/.kria/vault.enc`. Master key: `KRIA_VAULT_PASSPHRASE` (Argon2id) else a **random
   `vault.key` (0600) beside the vault**. ⇒ without a passphrase, "encrypted at rest" only stops
   other-user reads, not a local attacker. `keyring` crate is **NOT** a dependency (must be added
   for true OS keychain). Prefer passphrase/keychain; document the weak-mode caveat.

6. **RoutingContext — FAIL.** `RoutingContext` (`routing/context.rs:216`) is **not persisted**;
   the router builds `RoutingContext::default()` **fresh every call** (`routing/mod.rs:97`).
   Desktop AppState has no owner. ⇒ the self-reference/topic gate has NO existing per-conversation
   state; the spec must introduce and own a per-session `RoutingContext` (keyed by session/scope).

7. **schemars — PARTIAL.** `schemars 0.8` is in `kria-core/Cargo.toml:57` only (NOT workspace
   deps). `KriaConfig` does **not** derive `JsonSchema`. Deriving it needs the derive on **~60–90
   structs/enums** across `config.rs` + `openclaw`/`n8n`/`providers`/`capability` (files owned by
   OTHER active specs — coordinate). schemars gives **shape only**; risk/hot_reload/prompt
   annotations need a **hand-authored field registry** (do not rely on attribute sprawl).

8. **ToolContext — FAIL (security-critical).** `ToolContext = {env, shell_state, cancellation}`
   (`tools/mod.rs:55`). It has **no ConfigService/AppState handle and no user-vs-external
   provenance flag**. ⇒ Hard Rule 2 (injection wall) is **not implementable** and `config_patch`
   cannot read config as-is. MUST extend `ToolContext` with a config/state handle + a
   `provenance` field and thread it through `execute_with_context` **before** enabling prompt
   control. This is a prerequisite, not a nicety.

### 8.1 New/insufficiently-covered items to add
- **Injection provenance** is a hard prerequisite (Task added — see tasks.md Task 0b).
- **Cloud structured-output** path (not `chat_with_grammar`) for prompt extraction on cloud providers.
- **Per-session RoutingContext ownership** for the disambiguation gate.
- **Voice hands-free approval** flow for RED changes (no GUI popup available).
- **Multi-client write authority** (desktop vs mobile gateway) — define desktop ConfigService as
  the single authority; mobile writes route through it.
- **Temp-override cost tiers:** image-path overrides (`image_generation.image_mode/tier`) are
  cheap per-request; an LLM-runtime temp override is a heavy orchestrator swap (rejected mid-turn)
  — keep it OUT of the initial temp whitelist; ship image temp first.
- **Audit redaction:** secret field values must be redacted in the `AuditLogger` path too.

### 8.2 Verdict
Plan is **production-grade in structure** (phased, flag-gated, decoupling verified, correctness
properties, security rules, test matrix) but **NOT execution-ready** until the six corrections
above are absorbed. Two are security-blocking (injection wall via ToolContext provenance; cloud
output constraint). Green-light the architecture; correct the design before Wave 1.

---

## 9. External Architecture Review — Resolutions (7 concerns)

A first-principles review was run against the code + spec. Outcomes (see design.md for the edits):

1. **Runtime ownership — ⚠️ partial → clarified.** Runtime state is owned by `AppState`
   (`app_state.rs`: orchestrator, mcp_manager, image_orchestrator, voice, skill_registry,
   container_pool, gui_orchestrator, model_router). A full `RuntimeManager` is scope creep + high
   risk and was rejected. Instead, C5 effects are formalized as a typed `ConfigEffect`
   (`apply()`/`rollback()`); runtime state stays in AppState.

2. **God object — ✅ mostly (already decomposed) + 🐞 real bug fixed.** Storage/schema/secrets/
   effects/prompt/tool/override are already separate (C2–C8). But C1 previously implied
   ConfigService (core) "resolves effect", while C5 lives in kria-desktop — a **layering
   inversion**. Fixed: ConfigService (core) = validate→persist→version→publish only; effects run in
   desktop via subscription / dedicated apply services. No re-split needed beyond this.

3. **Rule-heavy prompt pipeline — ⚠️ acceptable → clarified.** Already hybrid (semantic domain
   gate + LLM extraction). Not an architectural cage. Hard AND-rule replaced with a **scored
   confidence + threshold**; stages declared pluggable (strategy pattern) so a trained classifier
   can replace stages 1–4 later. No rewrite.

4. **Transaction safety — ⚠️ underspecified → fixed.** Added the C1.1 **Transaction Model**:
   infallible effects persist-then-apply; **fallible effects apply-before-persist** via the
   dedicated service that owns rollback, persisting only on success. Prevents DB-ahead-of-runtime
   divergence (Property 8 now has a mechanism, not just a goal).

5. **Capability awareness — ⚠️ missing → fixed.** Schema is the correct static capability source
   but doesn't check runtime availability. Added C4.1 **availability resolver** (reuses existing
   provider/sidecar/MCP status; no new registry) as a stage between validation and apply.

6. **Plugin extensibility — ❌ not solved, correctly a v1 NON-GOAL.** Compile-time `KriaConfig`
   can't surface `my_plugin.settings.*` without recompiling core. No such requirement exists today
   (OpenClaw/MCP manage own config). Fix = C4 **composable schema seam** (`derived + registered
   fragments`) so it's a documented future extension without painting into a corner. No v1 code.

7. **Implementation risk / sequencing — ⚠️ good → improved.** Added shippable **milestones** so
   value lands incrementally and the novel prompt work is last: **M1 = Waves 0–3** (fixes the
   original drift pain; no prompt control), **M2 = Wave 4** (schema + UI), **M3 = Waves 5–6**
   (prompt control), M-final = Wave 7. Task 10 (schemars derive) flagged for cross-spec merge
   coordination and isolated so it can't block storage.

**Overall verdict:** design kept largely intact. Two genuine fixes (layering inversion #2,
transaction ordering #4) + three refinements (#1 effect trait, #5 availability, #6 schema seam) +
sequencing milestones (#7). No RuntimeManager, no ConfigService re-split, no pipeline rewrite.

---

## 10. Final Architecture Review (pre-implementation) — verdicts, new issues, readiness

Independent first-principles pass. Verdicts:

| # | Concern | Verdict | Resolution |
|---|---------|---------|-----------|
| 1 | Runtime dependency graph | **Rejected** (graph/RuntimeManager) | KRIA subsystems loosely coupled (embeddings dim=384 independent of provider `runtime.rs:667`; memory independent §2.8). Real residual (related fields) handled by `restart_group` + batching, not a graph engine. |
| 2 | Effect ordering/batching | **Accepted (Modified)** | `patch_batch` groups by `restart_group`, orders by `depends_on`, collapses one effect/group (design C1.2). |
| 3 | ConfigEffect registration | **Accepted (Modified)** | `ConfigEffectRegistry` (mirrors `ToolRegistry`), not a central match (design C5). |
| 4 | Pipeline observability | **Accepted** | `ConfigIntentTrace` (scores, matched fields, rejected candidates, decision) → diagnostics ring + golden tests (design C6). |
| 5 | Schema metadata | **Accepted (Modified)** | Add `restart_group`, `requires_backend`, minimal `depends_on`; DEFER `conflicts_with` (VoiceConfig::validate covers) + `capability_tags` (design C4). |
| 6 | RuntimeStatus cache | **Accepted (Modified)** | Event-updated `RuntimeStatus` from existing `HealthRegistry`/events; backs C4.1 + UI; no new poller (design C4.2). |
| 7 | Undo model | **Accepted (Modified)** | `change_set_id` transaction undo; forward-patch, preserves audit chain (design C1.2). |

New issues found:
- **N1 (HIGH)** startup barrier — register subscribers before processing config changes (design Lifecycle).
- **N2 (HIGH)** fallible-effect timeout — bounded, fail⇒no-persist (design C1.1).
- **N3 (HIGH)** change-during-active-turn — defer/reject cleanly (design C1.1); verified `providers.rs` rejects mid-turn swaps.
- **N4 (LOW)** Tauri single-backend ⇒ no multi-window race; external edits out of scope (design Lifecycle).
- **N5 (MEDIUM)** backend downgrade strands DB changes — documented + optional export (design Lifecycle).
- **N6 (LOW)** reconcile-on-lag = re-apply current value (design C5).

Risk ranking: **Critical:** none remaining. **High:** N1, N2, N3, batching inconsistency (all now specified). **Medium:** concerns 6/7, N5. **Low:** concerns 3/4/5-metadata, N4/N6.

**Implementation-readiness: YES.** No remaining high-severity architectural issue is unresolved in
the spec. All High items (N1/N2/N3/batching) have concrete mechanisms; the two prior Criticals
(layering inversion, transaction ordering) were fixed in §9. Remaining work is implementation, not
architecture. Rejected items (RuntimeManager, dependency-graph engine, conflicts_with/capability_tags,
plugin settings) are justified as speculative for current KRIA and preserved as documented seams
where cheap. **Proceed to Wave 0 when ready.**
