# Design Document — Settings & Configuration Revamp

## Overview

This design replaces KRIA's file-based user config with a **layered configuration
architecture** fronted by a single **`ConfigService`**, backed by a **SQLite user layer** in
`kria.db`, with **secrets in the OS keychain**, a **schema derived from `KriaConfig`**, and a
**prompt-driven settings control** layer that reuses existing safety, HITL, audit, grammar, and
event-bus infrastructure. It is explicitly decoupled from the memory subsystem and preserves the
`get_settings`/`update_settings` Tauri contract.

See `analysis.md` for the full verified current-state facts and decision log, and
`/media/obaid/SSD/KRIA/CONFIGURATION_ARCHITECTURE.md` for the 15-section current-system audit.

> **⚠ Pre-implementation corrections (see `analysis.md` §8 — code-verified).** Several infra
> assumptions below were refined after verification. Binding decisions:
> - **Event bus:** `AppState` holds `infra::EventBus` (typed `KriaEvent`, cap 256, NO string
>   topics / NO `subscribe_filtered`). The topic bus in C1/C3 is NOT wired. **Decision:** extend
>   `KriaEvent` with `ConfigChanged { section, version }` and reuse the wired bus; because it is a
>   bounded lossy broadcast, version-based reconciliation is **mandatory**.
> - **ToolContext (security-critical):** today `{env, shell_state, cancellation}` — no config
>   handle, no provenance. MUST be extended with a `ConfigService` handle + `provenance`
>   (`User|ExternalContent|Tool`) and threaded through `execute_with_context` BEFORE prompt control.
>   The injection wall is unenforceable otherwise.
> - **Grammar on cloud:** `chat_with_grammar` constrains local llama.cpp ONLY. On cloud providers
>   use `chat_structured` + strict validate + reject-and-reask; never apply an unvalidated patch.
> - **RoutingContext:** not persisted today (router uses `::default()` each call). The gate must
>   own a per-session `RoutingContext`.
> - **SecretsVault:** at `auth/vault.rs`; write method is `set` (not `put`); opens without a
>   passphrase via a 0600 keyfile (weak-at-rest). `keyring` crate is absent. Prefer passphrase/keychain.
> - **config module:** `config.rs` is one 2331-line file → convert to `config/mod.rs` first
>   (path unchanged, no import churn).
> - **schemars:** in kria-core only; `KriaConfig` doesn't derive `JsonSchema`; ~60–90 structs
>   across other modules need it. schemars = shape only; risk/annotations via a hand-authored registry.

## Architecture

```
                         ┌───────────────────────────────────────────────┐
                         │                 ConfigService                   │
                         │  Arc<RwLock<KriaConfig>>  +  version:AtomicU64   │
                         │  get() get_section() patch(sec,field,val,source) │
                         │  subscribe()  →  EventBus "config.<sec>.changed"  │
                         └───────▲───────────────▲───────────────┬─────────┘
            resolve (load)       │ write          │ read          │ publish
      ┌──────────────────────────┴───┐            │      ┌─────────▼──────────────┐
      │ Layered resolver              │            │      │ Subscribers (live apply)│
      │ code<default.toml<DB<secret<env│           │      │  gpu_policy, mcp, voice, │
      └───▲─────────▲────────▲────────┘            │      │  google, trust, future…  │
          │         │        │                     │      └──────────────────────────┘
   config/default   │   kria.db `config`     OS keychain / SecretsVault
   .toml (baseline) │   table (user layer)   (secret values; config stores refs)
                    │
        ┌───────────┴───────────────────────────────────────────────┐
        │ Access paths                                                │
        │  (a) UI:   get_settings / patch_config (field-level)        │
        │  (b) Prompt: config_patch tool → risk → HITL → patch        │
        │  (c) Temp:  turn-scoped RequestOverride (top of precedence) │
        └─────────────────────────────────────────────────────────────┘
```

### Precedence (fixed, deterministic)
```
code Default < config/default.toml < DB(user) < secrets(refs resolved) < env(ops) < RequestOverride(temp)
```
Identical whether or not `default.toml` exists (Requirement 6.4).

## Components and Interfaces

### C1. ConfigService (`crates/kria-core/src/config/service.rs` — new)
- Owns `Arc<RwLock<KriaConfig>>`, an `AtomicU64` version, an `Arc<EventBus>` handle, and a
  `ConfigStore` (storage backend) + `SecretStore`.
- API:
  - `get() -> KriaConfig` (clone), `get_section::<T>(name)`.
  - `patch(section, field, value_json, source) -> Result<AppliedChange>` — the single writer.
  - `subscribe_filtered("config.") -> FilteredSubscriber`.
  - `resolve() -> KriaConfig` — full layered load (used at startup and on external reload).
- Serialized writes via an internal async mutex; optimistic concurrency using the version number
  (reject stale writes from the UI carrying an old version).
- **Preserves serde shape**: `get_settings` calls `service.get()` and serializes as today.
- **Layering rule (fixes review concern 2):** `ConfigService` lives in `kria-core` and MUST NOT
  call `kria-desktop` apply services (C5). It cannot "resolve effect" itself. Its write path is:
  `validate (schema + availability) → [class-dependent order, see Transaction Model] → persist →
  bump version → publish`. Effects are executed by the desktop-side effect executor (C5) that
  subscribes to change events. This keeps core→desktop dependency-free.

### C1.1 Transaction Model (fixes review concern 4)
Config fields fall into two effect classes; the ordering differs to prevent persist⇔runtime
divergence (Property 8):
- **Infallible live-apply** (e.g. `ui.*`, `orchestrator.gpu_*` atomics): `persist → publish →
  subscriber applies`. The apply cannot fail, so persist-first is safe.
- **Fallible effect** (e.g. provider/model switch, MCP reconcile): **apply-before-persist**.
  `ConfigService.patch` delegates to the dedicated apply service (`apply_provider_selection`,
  `apply_mcp_runtime_from_config`) which validates + applies + **owns its own rollback**, and
  persists ONLY on reported success. If the effect fails, nothing is persisted and the prior
  value stands. ConfigService never persists a fallible change ahead of a successful apply.
- The effect class per `(section, field)` comes from the schema annotation (`hot_reload` +
  an `effect_kind: infallible|fallible|none`). Ownership of apply/rollback lives in the effect
  executor (C5), NOT in ConfigService.
- **Fallible-effect timeout (N2, HIGH):** every fallible effect runs under a bounded timeout
  (reuse `orchestrator.health_check_timeout_secs`). On timeout ⇒ treat as failed ⇒ do NOT persist
  ⇒ surface the error; never block `config_patch`/HITL/UI indefinitely.
- **Change-during-active-turn (N3, HIGH):** `apply_provider_selection` rejects runtime swaps while
  a local turn runs (`orchestrator_active_turns > 0`). Runtime-affecting permanent changes
  (provider/model/tier) requested via prompt or UI mid-turn MUST defer to turn-end or return a
  clear "will apply after this turn" — never deadlock or silently drop.

### C1.2 Batching & transaction grouping (fixes review concern 2 + 7)
- `patch_batch(changes: Vec<Change>, source) -> Result<AppliedChangeSet>`: the UI "Save" and any
  multi-field prompt change send ONE batch, not N patches.
- The batch is grouped by `restart_group` (C4 metadata), ordered by minimal `depends_on`
  (e.g. provider before model), and **collapses to one effect per group** (provider+model ⇒ a
  single `apply_provider_selection`, not two restarts). Avoids redundant restarts and the
  intermediate-inconsistency window (this is the real residual of review concern 1).
- A batch gets a `change_set_id`; each audit row carries it. **Undo operates at change-set
  granularity** (field-undo = batch of 1) and is a FORWARD patch (new audit entry), never a
  history deletion — preserving audit-chain integrity (concern 7).
- Batch atomicity is best-effort per group: if a fallible group fails, its own effect rolls back
  and the batch reports partial success with the failed group named; already-applied infallible
  groups are not reverted (they can't fail) — the response lists per-group outcomes.

### C2. Storage backend (`crates/kria-core/src/config/store.rs` — new)
- Trait `ConfigStore { load_user_layer(), put(section,key,value_json,source), delete(section,key), all() }`.
- `SqliteConfigStore` over `kria.db`:
  ```sql
  CREATE TABLE IF NOT EXISTS config (
    section    TEXT NOT NULL,
    key        TEXT NOT NULL,
    value_json TEXT NOT NULL,
    source     TEXT NOT NULL DEFAULT 'ui',   -- ui | prompt | env | migration | import
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (section, key)
  );
  CREATE TABLE IF NOT EXISTS config_meta ( config_version INTEGER NOT NULL );
  ```
  WAL (inherits kria.db). Field-level `put`/`delete` (Requirement 1.3, 12.2).
- `TomlConfigStore` = current behaviour, selected when `KRIA_CONFIG_BACKEND != sqlite` (Req 1.2).
- Migrations: `PRAGMA user_version` + ordered additive `MIGRATIONS` (copy
  `crates/kria-core/src/openclaw/registry.rs` `run_migrations` pattern). (Requirement 4.)

### C3. SecretStore (`crates/kria-core/src/config/secrets.rs` — new)
- Wraps the EXISTING `SecretsVault` (`crates/kria-core/src/auth/vault.rs`; API `open_default()`,
  `get(key)`, **`set(key, value)`** — note `set`, NOT `put`). Optionally add the `keyring` crate
  (absent today) for OS-keychain-backed key material.
- Config stores `"vault://kria/<id>"` refs; real values live in the vault. `redact_from(config)`
  for `get_settings` AND for the `AuditLogger` path (secret values must never hit the audit log).
- **Weak-at-rest caveat:** without `KRIA_VAULT_PASSPHRASE` the vault key is a random 0600 keyfile
  beside `vault.enc` — protects other-user reads only. Recommend passphrase or OS keychain;
  document the limitation. Never claim "encrypted" without qualifying the key source.
- Migration: detect plaintext secret in TOML/DB → `vault.set(...)` → replace with ref (dual-read
  fallback until verified). Reuse n8n `migrate_literal_*` precedent. (Requirement 3.)

### C4. Config schema (`crates/kria-core/src/config/schema.rs` — new)
- `#[derive(JsonSchema)]` on `KriaConfig` (schemars) → base schema.
- Field annotation registry (hand-authored declarative table) providing per-field:
  `risk: RiskLevel`, `hot_reload: bool`, `effect_kind: none|infallible|fallible`,
  `prompt_changeable: bool`, `valid_values`, `synonyms`,
  `restart_group: Option<String>` (batch grouping — C1.2), `depends_on: Vec<field>` (minimal,
  e.g. model→provider; NOT a general graph — review concern 1/5), `requires_backend:
  Option<Backend>` (drives the C4.1 availability check declaratively, e.g. `ComfyUI`, `KokoroSidecar`).
  Deferred as unnecessary-for-now: `conflicts_with` (covered by `VoiceConfig::validate`),
  `capability_tags` (add with the voice-discovery feature) — review concern 5.
- **Fail-closed default**: any field absent from the annotation registry ⇒ RED + not
  prompt-changeable + restart-required. (Requirement 5.3.)
- Exposed to: `config_patch` tool spec, validation, approval preview, (optionally) UI metadata.
- **Composable schema seam (future plugin extensibility — review concern 6, v1 NON-GOAL):** model
  the schema as `effective = derived(KriaConfig) + registered_fragments`. v1 registers only the
  derived schema, but the registry API is built to accept dynamic JSON-Schema fragments later so a
  future plugin's `my_plugin.settings.*` can become UI-visible / prompt-changeable / validated /
  persisted (the `config` table already stores arbitrary `(section,key)`) / hot-reloadable WITHOUT
  core changes. No plugin settings exist today; this is a design seam, not v1 work.

### C4.1 Capability availability resolver (prior review concern 5; declarative via `requires_backend`)
- Schema validation answers "is this a real, prompt-changeable field with a valid value". It does
  NOT answer "is the target achievable right now". An availability stage between schema validation
  and apply consults the C4.2 `RuntimeStatus` snapshot (NOT ad-hoc I/O per call):
  - provider/model → is that provider configured/enabled (`ProvidersConfig`), env-locked?
  - voice engine (kokoro), image local (ComfyUI), MCP server → is the `requires_backend` present?
- On unavailable target ⇒ informative error / clarification, never a silent schema-valid apply
  that fails at runtime.

### C4.2 RuntimeStatus cache (fixes review concern 6)
- A `RuntimeStatus` snapshot (provider availability/enabled/env-lock, sidecar/backend presence
  — ComfyUI/kokoro, MCP server up, `llm_runtime_apply_status`) maintained from EXISTING signals:
  `HealthRegistry` (already in `AppState`), the `KriaEvent` bus, and `llm_runtime_apply_status`.
  **No new poller** — it is event-updated from what already exists.
- Read by: C4.1 availability (avoids repeated expensive checks), the Settings UI (live status),
  and the prompt availability stage. Single source for "is X currently possible".

### C5. Effect executor + registry (`crates/kria-desktop/src/commands/config_effects.rs` — new)
- **Lives in kria-desktop** (owns the runtime handles). Subscribes to `ConfigChanged` events for
  infallible effects; is invoked directly by `config_patch`/`patch_batch` for fallible effects
  (apply-before-persist, per C1.1). This is the ONLY place that touches runtime — it does not
  live in, and is not called by, the core `ConfigService`. On a dropped/lagged event an infallible
  subscriber **re-applies the current value** (not just notes the version) (N6).
- **Registration, not a central match (review concern 3):** effects register into a
  `ConfigEffectRegistry` (mirrors `ToolRegistry::register`) keyed by `restart_group`/section, each
  a typed `ConfigEffect { apply(); rollback(); }`. Subsystems colocate/own their effect; avoids a
  god-`match` and is the host for future plugin-provided effects (C4 seam). Runtime state itself
  stays owned by `AppState` — no RuntimeManager (review concern 1 rejected: KRIA subsystems are
  loosely coupled; embeddings/memory/openclaw do not depend on the chat provider — §2.8).
- Maps `(section, field)` → an apply effect. Effects reuse existing services:
  - provider/model → `apply_provider_selection` (providers.rs) [NOT raw write].
  - `orchestrator.gpu_*` → `gpu_policy::apply_settings`.
  - `mcp.servers` → `apply_mcp_runtime_from_config`.
  - `openclaw.trust` → `set_live_trust_config`.
  - google → `apply_google_runtime_env_from_config`.
  - voice → per-session re-read (unchanged) / future subscription.
  - restart-required fields → return `RestartRequired` marker (Requirement 7.2).
- On live-apply failure → reconcile (rollback persist or retry) so config ≠ runtime never persists
  (Requirement 7.4).

### C6. Prompt-control pipeline (`crates/kria-core/src/config/prompt/` — new)
Stages (cheapest-first), all in-session, NO MemoryStore dependency. **This module must OWN a
per-session `RoutingContext`** (keyed by session/scope) — the router does not persist one today
(`routing/mod.rs:97` uses `::default()` each call), so the gate builds its own topic state.
1. **Settings-domain gate** — semantic router centroid score (reuse `routing/semantic.rs`).
2. **Self-reference gate** — the pipeline's own persisted `RoutingContext` (topic continuation) +
   subject markers ("your/the app/KRIA" vs "I/my/this code/this project") + schema grounding.
   v1 uses deterministic lexical/grammar rules (imperative/interrogative + subject markers); a
   trained `settings.change`-vs-`settings.query` ONNX head is a LATER add (needs labeled data,
   don't block on it). (Requirement 10.3, 10.7.)
3. **Action-vs-query** — imperative vs interrogative (reuse/extend ONNX intent classifier;
   `settings.change` vs `settings.query` labels).
4. **Scope** — temp vs permanent (cue words; default temp when unclear, else clarify).
5. **Extraction** — `config_patch` tool emits structured `{section,field,value,scope}`. Use
   `chat_with_grammar` on LOCAL llama.cpp (real constraint) but **`chat_structured` on cloud
   providers** (grammar does NOT bind on cloud — see analysis.md §8 item 4). Always **strict-validate against the
   derived schema and reject-and-reask**; never apply unvalidated output. No valid field ⇒ query.
   (Requirement 9.5, 10.)
6. **Availability check (C4.1)** — even a schema-valid field is refused/clarified if the target
   value is not currently achievable (provider not configured, sidecar absent, env-locked).
7. **Decision to act** — combine the stage signals (settings-domain, self-reference-KRIA,
   schema-grounded, imperative) as a **scored confidence with a threshold**, NOT a brittle hard
   AND. High confidence ⇒ act; mid ⇒ one clarifying question; low ⇒ answer as query. This keeps the
   stages pluggable/replaceable (strategy pattern) so a future trained classifier can supplant
   stages 1–4 without redesign (addresses review concern 3). **Fail toward query.** (Req 10.5, 10.6.)

> Pipeline stages are an implementation strategy, not an architectural cage: each stage is a
> swappable scorer behind a common interface; the extraction stage is already LLM-based.

**Observability (fixes review concern 4):** each decision emits a `ConfigIntentTrace`
{ per-gate scores, matched schema field(s), rejected candidates, availability result, final
decision (act|clarify|answer) + confidence }. Written to the existing diagnostics ring and
asserted by the golden-set tests (Req 10 needs to prove *why* a decision was made) and used for
future model tuning. Low cost, high debugging/eval value.

## Lifecycle & Consistency (newly audited)

- **Startup barrier (N1, HIGH):** subsystems today are built from snapshots before `AppState`/the
  event bus exist (`runtime.rs:~1613`). Ordering MUST be: create `ConfigService` + event bus →
  register all effect subscribers → emit a `config-ready` barrier → only then process external
  config changes. Prevents early `ConfigChanged` events being missed by not-yet-registered subscribers.
- **Shutdown:** in-flight fallible effects hold the `llm_runtime_apply_lock`; shutdown waits for or
  cancels them via the existing `CancellationToken`; no partial persist (apply-before-persist means
  an interrupted effect simply never persisted).
- **Crash recovery:** temp overrides are in-memory only ⇒ vanish on crash (safe by construction).
  Persisted config is atomic per row (SQLite) ⇒ no partial-write. On restart, `resolve()` rebuilds
  effective config; effects re-derive from persisted state.
- **Backend downgrade (N5, MEDIUM):** sqlite→toml downgrade strands post-migration DB changes
  (`.bak` is stale). Documented limitation; provide an optional DB→TOML export on downgrade.
- **External / multi-window edits (N4):** Tauri windows share ONE backend `AppState` ⇒ writes are
  serialized by the ConfigService mutex (no cross-window race). External `sqlite3`/`.bak` edits
  bypass ConfigService (stale in-memory) — OUT OF SCOPE; optional `reload_config` command.

### C7. `config_patch` tool (`crates/kria-core/src/tools/config_patch.rs` — new)
- Registered via `ToolRegistry::register` (Requirement 9.1).
- Input: `{section, field, value, scope: temp|permanent}` (schema-validated per C6).
- **Requires the extended `ToolContext` (C10)** — needs the `ConfigService` handle to read/apply
  and the `provenance` field to enforce the injection wall. Cannot function on today's ToolContext.
- Flow: schema validate → **provenance check (refuse unless `User`)** → risk classify
  (`PolicyEngine`) → if temp & whitelisted ⇒ apply RequestOverride; if permanent ⇒ risk gate →
  `HitlGateway` approve (YELLOW/RED, emitting `agent:approval_required` when outside an agent
  stream) → `ConfigService.patch` → effect dispatch → `AuditLogger.log` (prior+new value,
  **secrets redacted**). (Requirements 8, 9, 11.)
- **Injection wall:** refuse invocation when `ctx.provenance != User` (content from
  external/tool/web/file). (Requirement 9.6, hard rule 2.)

### C10. Extended `ToolContext` + HITL delivery (prerequisite for prompt control)
- Extend `ToolContext` (`crates/kria-core/src/tools/mod.rs`) from `{env, shell_state,
  cancellation}` to also carry an optional `config: Option<Arc<ConfigService>>` and
  `provenance: TriggerProvenance {User|ExternalContent|Tool}`; thread through
  `execute_with_context`. Existing handlers ignore the new fields (default = safe).
- HITL delivery for non-agent triggers: when `config_patch` runs outside an agent stream, emit
  the same `agent:approval_required` Tauri event and accept `approve_action`/`deny_action`
  (both exist, `app_commands.rs:144/171`).

### C8. RequestOverride (turn-scoped) (`crates/kria-core/src/config/request_override.rs` — new)
- A per-turn overlay consulted at the top of precedence for whitelisted fields only.
- Attached to the turn/agent-loop context; dropped at turn end (success or error). (Requirement 8.)
- Whitelist (initial, **cheap per-request only**): `image_generation.image_mode`,
  `image_generation.tier_override`, response verbosity. Never auth/network/safety/secrets.
- **Cost caveat:** an LLM-runtime temp override ("use local AI" for chat) is a heavy orchestrator
  stop/start (rejected mid-turn, slow) — it is NOT a cheap per-turn param and is EXCLUDED from the
  initial whitelist. "Generate an image using local AI" maps to the image-generation local/cloud
  path (cheap `image_mode`), not an LLM swap. LLM-runtime temp override is a later, explicit feature.

### C9. Frontend (`ui/src/stores/app.ts`, `SettingsModal.tsx`)
- Replace whole-blob `update_settings` round-trip with field/section `patch_config` calls.
- After save: re-fetch or subscribe to a `config-changed` Tauri event (kill optimistic-only).
- Honor env-locked fields (extend the existing ProviderSettings "locked by env" pattern globally).
- Keep provider/model on the dedicated `set_active_llm_selection` + `llm-runtime:apply` path.
- Optionally consume the derived schema for field metadata/restart badges.

## Data Models

- `config` table (C2) — user layer, field-level rows.
- `config_meta.config_version` — migration marker.
- Secrets — keychain/vault, referenced by `keyring://kria/<id>`.
- Audit — existing `audit_log` (hash-chained) records config mutations with `action="config_patch"`.
- `KriaConfig` struct — unchanged shape; new fields must be `#[serde(default)]` + annotated.

## Risk Classification Map (initial)

| Field group | Risk | Prompt | Temp | Apply |
|-------------|------|--------|------|-------|
| `ui.*` (theme, font, contrast, motion) | GREEN | yes | no | live |
| `search.*`, `memory.*` (wired), `agent.*` | YELLOW | yes | no | live/restart |
| `voice.*` | YELLOW | yes | no | per-session |
| provider/model select | YELLOW | yes | temp(1 gen) | apply service |
| `image_generation.image_mode/tier` | GREEN | yes | **yes** | live/per-request |
| `orchestrator.gpu_*` | YELLOW | yes | no | live |
| `mcp.servers`, `openclaw.*`, integrations (telegram/google/colab/n8n) | YELLOW/RED | yes | no | live/reconcile |
| `server.*`, `mobile.*`, `remote_desktop.*`, `safety.*` | RED/BLACK | gated | no | restart + strong approval |
| secrets (all keys/tokens/passphrases) | RED | vault only | no | vault write |
| unannotated / unknown | RED (fail-closed) | no | no | restart |

## Correctness Properties

### Property 1: Contract stability
For any `KriaConfig`, `deserialize(serialize(c)) == c`, and the `get_settings` JSON shape is
byte-identical regardless of storage backend.
**Validates: Requirements 12.1, 13.5**

### Property 2: Precedence determinism
The effective value of any field is a pure function of the layers
(`code < default.toml < DB < secrets < env < temp`) and is identical whether or not
`config/default.toml` exists on disk.
**Validates: Requirements 6.2, 6.4**

### Property 3: Field-write isolation
`patch(section, field, v)` changes only that row; all other fields' persisted values are
unchanged.
**Validates: Requirements 1.3, 12.2**

### Property 4: Single writer / no lost update
Concurrent patches are serialized; a stale-version write is rejected, never silently overwriting
a newer value.
**Validates: Requirements 2.2, 2.6**

### Property 5: Temp isolation
A temporary override affects exactly one turn and is fully reverted on success or failure, with
no persistence and no leak to later turns.
**Validates: Requirements 8.2, 8.5**

### Property 6: No unauthorized mutation
Config is mutated only via an explicit tool/command call; answering a query or processing
external/injected content never mutates config.
**Validates: Requirements 9.5, 9.6, 10.6**

### Property 7: Secret confinement
No secret value is ever written to the config table, TOML, logs, or chat history; only references
are stored.
**Validates: Requirements 3.1, 3.2, 3.4**

### Property 8: Persist-runtime consistency
After any change, persisted config and running runtime agree, or the change is rolled back; they
are never left divergent.
**Validates: Requirements 7.4**

### Property 9: Fail-closed schema
Any config field without a prompt/risk annotation is non-prompt-changeable and treated as
high-risk.
**Validates: Requirements 5.3**

### Property 10: Legacy equivalence
With all feature flags falsy, behaviour is byte-for-byte the current system.
**Validates: Requirements 13.3**

## Error Handling
- Corrupt/unopenable config DB ⇒ fail closed to default.toml + code defaults, warn (Req 1.6).
- Migration failure ⇒ abort txn, keep prior version, defaults (Req 4.5).
- Live-apply failure ⇒ reconcile persist vs runtime (Req 7.4); provider path already has rollback.
- Invalid/out-of-range value ⇒ clamp or reject with a clear message (edge case E).
- Stale UI write (old version) ⇒ reject + prompt re-fetch (Req 2.6).
- Ambiguous prompt ⇒ one clarifying question, never guess (Req 10.4).

## Testing Strategy

### Unit / property
- Precedence & merge property test: every section, both `default.toml` present/absent (Req 6).
- `KriaConfig` JSON round-trip stability (frontend contract guard) (Req 13.5).
- Field-level `put`/`delete` isolation (changing one field leaves others intact) (Req 1.3).
- Migration test: old DB/TOML → new schema, additive, no data loss (Req 4) — reuse
  `kria-eval/openclaw_eval/upgrade.rs` pattern.
- Secret migration: plaintext → vault ref, dual-read fallback (Req 3.5).
- "Dead config" test: every schema-exposed field has a consumer (Req 13.4).

### Intent disambiguation golden set (Req 10 — top regression guard)
From `analysis.md` §5. Must include, at minimum:
- Query vs action: "what is dark mode?" (no change) / "turn on dark mode" (change).
- Self-reference/topic: "should I use Gemini for my project?" (no change) / "use gemini" mid-code
  (clarify) / "change the API key in my code" (no change).
- False positives (MUST NOT trigger): "I'll change my approach to dark themes in CSS",
  "enable the feature flag in the code", "turn on the lights", "switch branches".
- Temp vs permanent: "generate this using local AI" (temp, reverts) / "always use local AI" (permanent).
- Negation: "don't use cloud AI" ⇒ local. Idempotency: "switch to dark" when already dark (no-op).
- Multilingual: "theme ko dark karo" ⇒ same as English.

### Integration
- Prompt → `config_patch` → HITL approve → persist → event → UI reflects live.
- Temp override applies for one turn, reverts on success AND on error/crash (Req 8.2).
- Injection: config-change instruction embedded in a fetched document/tool output ⇒ NO change (Req 9.6).
- Env-locked field: prompt attempt refused with "locked by env" (Req 12.4).
- Concurrent UI + prompt write to same field ⇒ version check resolves cleanly (Req 2.6).

### Verification honesty
Features not verifiable on the current box (e.g. keychain backend on a headless CI) are marked
clearly; no fabricated pass results.

## Rollout & Flags
- `KRIA_CONFIG_BACKEND=toml|sqlite` (default `toml` until validated) — gates storage (Req 1).
- `KRIA_CONFIG_PROMPT_CONTROL` — gates the prompt-control pipeline + `config_patch` tool.
- `KRIA_CONFIG_SERVICE` — gates routing reads through ConfigService (vs direct `state.config`).
- All falsy ⇒ legacy behaviour byte-for-byte (Req 13.3). Canary on dev builds first.

## Out of Scope (v1)
- Cross-session referential recall ("same as yesterday") — needs memory revamp; bolt on later
  as a ConfigService reader (additive, same `kria.db`).
- Prompt-driven reconfiguration of `kria-server` (fleet stays file/env-driven).
- Config-tunable risk tiers (kept compile-time for safety).
