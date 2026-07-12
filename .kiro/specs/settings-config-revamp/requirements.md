# Requirements Document

## Introduction

K.R.I.A.'s configuration today is file-based (`config/default.toml` + `~/.kria/config.toml`)
with a partial, environment-dependent merge, an `Arc<RwLock<KriaConfig>>` live handle, and
~20 independent per-feature load/save commands feeding the Settings UI. This causes **config
drift**: the UI, the file, and the running subsystems disagree; user overrides for ~19 of 28
sections are silently dropped when `config/default.toml` exists; secrets are stored plaintext;
and most settings need an app restart to take effect.

This spec re-architects configuration into a **layered model with a SQLite-backed user layer
and a single `ConfigService`**, then adds **prompt-driven settings control** with a strict
temporary-vs-permanent split, safety gating, and query-vs-action-vs-discussion disambiguation.

The work is designed to be **decoupled from the main memory subsystem** (verified independent)
and **future-proof** (schema derived from the `KriaConfig` struct so added/removed settings
propagate automatically).

Scope is the desktop app: `kria-core` (`config.rs`, new `config/` service module, `safety/*`,
`tools/*`, `automation/event_bus.rs`), `kria-desktop` (`commands/*`), and the SolidJS UI
(`ui/src/stores/app.ts`, `ui/src/components/SettingsModal.tsx`, `ui/src/components/ProviderSettings.tsx`).
`kria-server` stays file/env-driven (fleet determinism) — out of scope for prompt control.

Every behavioural change is **flag-gated** (default: legacy behaviour byte-for-byte unless the
`KRIA_CONFIG_BACKEND` / feature flags are enabled). New struct fields are `#[serde(default)]`.
The `get_settings` / `update_settings` Tauri JSON contract (the `KriaConfig` serde shape) is
**preserved** — SQLite is an internal storage detail. No fabricated results.

## Glossary
- **User layer**: the config values a user changes (today `~/.kria/config.toml`; target: `kria.db` `config` table).
- **Baseline**: `config/default.toml` — git-tracked dev/ops/fleet defaults, read-only at runtime.
- **ConfigService**: the single serialized reader/writer + change event bus wrapping `Arc<RwLock<KriaConfig>>`.
- **Effect**: a runtime action triggered by a config change (e.g. provider switch, MCP reconcile).
- **Temporary override**: a turn-scoped setting change that is never persisted and auto-reverts.
- **Permanent change**: a persisted config change requiring risk-appropriate approval.
- **Self-reference gate**: the classifier deciding whether a prompt is about KRIA itself vs the topic under discussion.
- **Wired field**: a config field that a subsystem actually reads (opposite of a dead/ignored field like `memory.embedding_dim`).

---

## Requirements

### Requirement 1: SQLite-backed user config layer
**User Story:** As a user, I want my settings stored in KRIA's database so the UI and runtime
never disagree with a stale file, and my changes are atomic and durable.

#### Acceptance Criteria
1. WHEN the config backend flag `KRIA_CONFIG_BACKEND=sqlite` is set THEN the system SHALL read
   the user layer from a `config` table in `kria.db` instead of `~/.kria/config.toml`.
2. WHEN `KRIA_CONFIG_BACKEND` is `toml` or unset THEN the system SHALL retain the current
   file-based behaviour byte-for-byte.
3. WHEN a config value is written THEN the system SHALL persist it as a discrete
   `(section, key, value_json, updated_at, source)` row so a single field can be changed
   without rewriting unrelated sections.
4. WHEN the user layer is read THEN the effective config SHALL be
   `code default < config/default.toml < DB(user) < secrets < env`.
5. WHEN `config/default.toml` is absent THEN precedence SHALL remain identical (DB user layer
   over code defaults) with no behavioural inversion.
6. IF the `config` table cannot be opened or is corrupt THEN the system SHALL fail closed to
   `config/default.toml` + code defaults and log a warning, never crash.

### Requirement 2: ConfigService as single source of truth
**User Story:** As a developer, I want one service that owns config reads/writes and broadcasts
changes, so subsystems never hold divergent snapshots.

#### Acceptance Criteria
1. WHEN any component reads config THEN it SHALL do so through `ConfigService::get()` /
   `get_section()` over the single `Arc<RwLock<KriaConfig>>`.
2. WHEN a config field is changed THEN `ConfigService::patch(section, field, value, source)`
   SHALL be the only write path, serialized so concurrent writers cannot interleave.
3. WHEN a patch is committed THEN the system SHALL publish a `config.<section>.changed` event on
   the `EventBus` with the changed field(s) and a monotonically increasing config version.
4. WHEN a subsystem subscribes via `subscribe_filtered("config.")` THEN it SHALL receive change
   events and be able to reconcile by re-reading current config (guarding against lossy broadcast).
5. WHEN the `get_settings` Tauri command is called THEN the returned JSON SHALL be the identical
   `KriaConfig` serde shape as today (frontend contract preserved).
6. WHEN two writers (UI and prompt) target the same field concurrently THEN the system SHALL
   apply last-write-wins with a version check and reject/re-base stale writes.

### Requirement 3: Secrets isolation
**User Story:** As a user, I want my API keys stored securely, not in a plaintext config file.

#### Acceptance Criteria
1. WHEN a secret field (API keys, tokens, HMAC keys, passphrases, `jwt_secret`) is saved THEN
   the system SHALL store it in the OS keychain / `SecretsVault`, not in the config table or TOML.
2. WHEN config is persisted THEN the config record SHALL contain only a secret *reference*, never the value.
3. WHEN `get_settings` returns config THEN all secret values SHALL be redacted (as today).
4. WHEN a secret is provided via a prompt in chat THEN the system SHALL redact it from chat
   history and logs before persistence.
5. WHEN migrating an existing `~/.kria/config.toml` with plaintext secrets THEN the system SHALL
   move them into the vault and leave only references, with a verifiable dual-read fallback during migration.

### Requirement 4: Versioned schema and safe migrations
**User Story:** As a developer, I want config schema changes to migrate old data safely so
future field additions/renames never brick a user's stored config.

#### Acceptance Criteria
1. WHEN the config DB is opened THEN the system SHALL check a `config_version` (via
   `PRAGMA user_version` or a `config_meta` row) and run pending additive migrations in order,
   each in its own transaction (reusing the OpenClaw registry migration pattern).
2. WHEN a new `KriaConfig` field is added THEN the system SHALL absorb it via `#[serde(default)]`
   with no migration required.
3. WHEN a field is renamed/removed THEN a migration SHALL map old stored values forward without data loss.
4. WHEN a one-time import from `~/.kria/config.toml` runs THEN it SHALL populate the DB user
   layer and retain the TOML as a `.bak` backup.
5. IF a migration fails THEN the system SHALL abort that migration transaction, keep the prior
   version, and fall back to defaults rather than persisting partial state.

### Requirement 5: Self-updating configuration schema
**User Story:** As a developer, I want the UI, prompt-agent, and validation to update
automatically when I add or remove a setting, so I maintain one source of truth.

#### Acceptance Criteria
1. WHEN the `KriaConfig` struct changes THEN a machine-readable schema SHALL be derivable from it
   (via `schemars`) without hand-maintained parallel lists.
2. WHEN a field is annotated THEN the schema SHALL carry its `risk` level, `hot_reload` capability,
   `valid_values`, and optional `synonyms`.
3. WHEN a field has NO risk/prompt annotation THEN the schema SHALL treat it as high-risk and
   NOT prompt-changeable (fail-closed).
4. WHEN the prompt-agent needs to change a setting THEN it SHALL only be able to target fields
   present in the derived schema and marked prompt-changeable.
5. WHEN the Settings UI renders THEN it SHOULD be able to consume the same schema for field
   metadata (types, valid values, restart-required badge).

### Requirement 6: Complete and deterministic precedence/merge
**User Story:** As a user, I want every setting I change to actually take effect regardless of
environment, so overrides are never silently dropped.

#### Acceptance Criteria
1. WHEN a user override exists for ANY of the 28 config sections THEN it SHALL be applied (fix
   the current ~19-section merge gap).
2. WHEN both `config/default.toml` and a user override set the same field THEN the user value SHALL win.
3. WHEN merging a section THEN the system SHALL merge at field level, not replace whole sections,
   so newer baseline fields are not masked by a single user-changed field.
4. WHEN precedence is evaluated THEN the outcome SHALL be identical whether or not
   `config/default.toml` is present on disk.
5. WHEN a config field is exposed to the UI or prompt THEN it SHALL be verified as *wired*
   (read by a subsystem); dead fields SHALL be either wired or removed/marked non-functional.

### Requirement 7: Expanded live hot-reload
**User Story:** As a user, I want most settings to take effect immediately so I don't restart the app.

#### Acceptance Criteria
1. WHEN a hot-reloadable field changes THEN the owning subsystem SHALL apply it live via a
   `config.<section>.changed` subscription or an `apply_settings`-style mutator (per the gpu_policy pattern).
2. WHEN a restart-required field changes THEN the system SHALL clearly signal "restart required"
   and NOT pretend the change is live.
3. WHEN a change requires an LLM/provider restart THEN it SHALL route through the existing
   `apply_provider_selection` service (stop/start + rollback + concurrency lock), not a raw config write.
4. WHEN a change fails to apply live THEN the persisted config and the running runtime SHALL be
   reconciled (rollback the persist or retry), never left divergent.
5. WHEN the schema marks a field `hot_reload=true/false` THEN the approval preview SHALL show the
   correct "instant" vs "needs restart" outcome.

### Requirement 8: Temporary (per-request) settings override via prompt
**User Story:** As a user, I want to say "generate this one using local AI" and have it apply
only to that request without changing my saved defaults.

#### Acceptance Criteria
1. WHEN a prompt requests a temporary change on a whitelisted field THEN the system SHALL apply
   it as a turn-scoped override at the top of precedence, above env.
2. WHEN the turn completes (success OR error/crash) THEN the temporary override SHALL be reverted
   and SHALL NOT persist to the DB or leak into subsequent turns.
3. WHEN a temporary override targets a non-whitelisted field (auth, network, safety, secrets)
   THEN the system SHALL refuse the temporary path and require a permanent, approved change.
4. WHEN a temporary override is applied THEN it SHALL NOT require an approval popup (the user
   explicitly requested it in-prompt) and SHALL be recorded in the audit log as temporary.
5. WHEN multiple temporary overrides are requested in one prompt THEN all SHALL apply for that
   turn and all SHALL revert together.

### Requirement 9: Permanent settings change via prompt (approved)
**User Story:** As a user, I want to say "change theme to dark" and have it permanently applied
after I approve it.

#### Acceptance Criteria
1. WHEN a prompt requests a permanent change THEN the system SHALL route it through a
   `config_patch` tool that emits a grammar-constrained `{section, field, value, scope}` payload
   validated against the derived schema.
2. WHEN the target field maps to a valid, prompt-changeable schema entry THEN the system SHALL
   classify its risk via `PolicyEngine` and gate accordingly.
3. WHEN the field is GREEN risk THEN the change MAY apply without a popup; WHEN YELLOW/RED THEN
   the system SHALL require `HitlGateway` approval before applying.
4. WHEN a permanent change is approved and applied THEN the system SHALL persist it, apply live
   (or signal restart), publish the change event, and record it in the `AuditLogger` with the prior value.
5. WHEN the LLM cannot map the request to a valid schema field THEN the system SHALL treat it as
   a query/answer and make NO config change.
6. WHEN the field is an auth/network/safety/remote-desktop/secret field THEN it SHALL be gated at
   RED/BLACK, never auto-applied, and never applied from external/injected content.

### Requirement 10: Query vs action vs discussion disambiguation
**User Story:** As a user, I want KRIA to tell the difference between me asking about a setting,
commanding a change, and merely discussing a topic (e.g. code), so it never changes settings by mistake.

#### Acceptance Criteria
1. WHEN a prompt does not score on the settings domain THEN the config tool SHALL NOT be offered
   and no config change SHALL occur.
2. WHEN a prompt is interrogative ("what is…", "can I…", "should I…") THEN the system SHALL treat
   it as a query and make NO config change.
3. WHEN a prompt is imperative but the subject is the topic/code under discussion (self-reference
   gate: "my code", "this project" vs "your"/"the app"/"KRIA") THEN the system SHALL treat it as
   discussion and make NO config change.
4. WHEN the settings intent is ambiguous THEN the system SHALL ask exactly ONE clarifying question
   rather than guessing.
5. WHEN acting on a settings change THEN the decision SHALL require ALL of: settings-domain match,
   self-reference to KRIA, schema-grounded field, and imperative mood (AND-rule).
6. WHEN in doubt THEN the system SHALL fail toward query/answer, never toward a config mutation.
7. WHEN the active conversation topic is a non-settings domain THEN a bare ambiguous phrase
   (e.g. "use gemini") SHALL trigger clarification, not a silent change.

### Requirement 11: Audit, undo, and read-back
**User Story:** As a user, I want to review, undo, and query my settings changes.

#### Acceptance Criteria
1. WHEN any permanent config change is applied THEN the system SHALL record action, field, prior
   value, new value, decision, and actor (ui/prompt/env) in the hash-chained `AuditLogger`.
2. WHEN the user says "undo my last settings change" THEN the system SHALL restore the prior value
   from the audit record (same-session) and apply it through the normal patch path.
3. WHEN the user asks "what is my current <setting>?" THEN the system SHALL read it back from
   ConfigService without making a change.
4. WHEN the user requests cross-session recall ("same as yesterday") THEN v1 SHALL respond that
   this requires the memory upgrade and MAY offer same-session/audit-based alternatives.
5. WHEN a settings change is applied via prompt THEN an open Settings UI SHALL reflect the new
   value live (via event subscription or re-fetch).

### Requirement 12: UI contract preservation and sync
**User Story:** As a user, I want the Settings UI to always show the true current values.

#### Acceptance Criteria
1. WHEN storage moves to SQLite THEN `get_settings`/`update_settings` command names, arguments,
   and JSON shape SHALL remain unchanged.
2. WHEN the UI saves a setting THEN it SHALL patch at field/section granularity and SHALL NOT
   round-trip the entire config blob in a way that can drop unknown fields.
3. WHEN a save completes THEN the UI SHALL reflect the persisted (not merely optimistic) value,
   via re-fetch or a change event.
4. WHEN a field is locked by an environment variable THEN the UI (and the prompt path) SHALL
   surface a "locked by env" indication and refuse to change it.
5. WHEN provider/model selection changes THEN it SHALL continue to flow through the dedicated
   apply service and its `llm-runtime:apply` event, not the generic settings save.

### Requirement 13: Decoupling and non-regression
**User Story:** As a developer, I want the settings system isolated so future changes (especially
the memory revamp) don't break it and vice-versa.

#### Acceptance Criteria
1. WHEN the memory subsystem is later revamped THEN it SHALL integrate as a ConfigService
   subscriber/reader only, requiring no changes to the settings core.
2. WHEN settings storage changes THEN MemoryStore and other subsystems SHALL be unaffected
   (they open by path and subscribe to events, not hold config internals).
3. WHEN any feature flag in this spec is falsy THEN the corresponding legacy behaviour SHALL be
   preserved byte-for-byte.
4. WHEN the spec ships THEN a "dead config" test SHALL assert every schema-exposed field has a consumer.
5. WHEN config is serialized and deserialized THEN a round-trip test SHALL prove the
   `KriaConfig` JSON shape is stable (frontend contract guard).
