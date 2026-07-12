# Requirements Document

Natural-Language Settings Control (Production Architecture)

## Glossary
- **Configuration Intent:** the user wants to read/change KRIA's own settings.
- **Conversation Intent:** the user is talking about their own code/topic/project (not KRIA config).
- **SettingsIntentPipeline:** the schema-driven classifier that produces a `SettingsDecision`.
- **SettingsHandler:** the single shared executor for all settings operations (chat + command).
- **GREEN/YELLOW/RED/BLACK:** risk tiers from `PolicyEngine`; GREEN auto-applies, others gate.
- **Read-back:** answering "what is my current <setting>?" from `ConfigService`.
- **Temp override:** a turn-scoped `RequestOverride` that never persists.

## Introduction

The previous spec (`settings-config-revamp`) delivered the **storage + safety backbone**:
`ConfigService`, SQLite user layer, secrets vault, derived schema + `FieldMeta` registry,
`config_patch` tool, `RequestOverride`, audit ledger, and a `config_prompt` desktop command.
Live testing from the **real chat UI** then proved the *understanding + routing* layer is broken:
only "change theme to X" (GREEN, permanent, value-extractable) works; every other settings
prompt (read-back, undo/revert, YELLOW/RED changes, temporary overrides, ambiguous phrasing)
either misroutes to unrelated tools (browser, web search, marketplace, recall_fact) or hallucinates.

Root cause (code-verified, see `analysis.md`): settings understanding lives in a **thin
deterministic pre-router** that (a) only covers GREEN permanent changes, (b) runs tools **raw with
no PolicyEngine/HITL gate**, (c) competes with other regex/forcing layers that hijack the prompt,
and (d) is a **separate implementation** from the `config_prompt` command. The general chat planner
(semantic centroid router + FastEmbed + LLM tool choice) has **zero settings awareness**.

This spec re-architects settings understanding into a **single, first-class, schema-driven
Settings Intent pipeline** shared by chat and the command surface, with a strict separation between
**Configuration Intent** and **Conversation Intent**, no hardcoded prompt/keyword lists, and
verification through the **real desktop frontend**. It also fixes every issue discovered during
investigation (RC1–RC7, NEW-1…NEW-13).

The system must feel intelligent to a non-technical user: it understands *intent*, not keywords,
and scales to 500+ settings and thousands of phrasings without per-prompt code.

### Guiding principles
- **One shared handler.** Chat and command surfaces call the same Settings pipeline; no divergence.
- **Intent, not keywords.** Classification is semantic + schema-grounded + context-aware; adding a
  setting must not require new routing code.
- **Strict intent separation.** Configuration Intent vs Conversation Intent is resolved by context,
  entity resolution, conversation state, and confidence — never by assuming a word means config.
- **Fail toward conversation.** A wrong answer is recoverable; a wrong config mutation is the
  original pain. Ambiguity ⇒ one clarifying question, never a guess.
- **One safety gate.** Every mutation flows through PolicyEngine + HITL + audit — no raw path.
- **No fabricated results.** Completion requires real-frontend validation, not just unit tests.

### Flag & compatibility
- Gated by `KRIA_NL_SETTINGS` (default off ⇒ legacy behaviour byte-for-byte). Reuses
  `KRIA_CONFIG_BACKEND`/`KRIA_CONFIG_SERVICE` for storage. The `get_settings`/`update_settings`
  Tauri contract and `KriaConfig` serde shape remain unchanged.

---

## Requirements

### Requirement 1: Unified Settings Intent Pipeline (single source of truth)

**User Story:** As a user, I want settings prompts to behave identically whether I type them in
normal chat or a settings box, so that there is one predictable, intelligent behaviour.

#### Acceptance Criteria
1. WHEN a user message is processed for settings intent THEN chat and the command surface SHALL
   invoke the SAME `SettingsIntentPipeline` + `SettingsHandler`, with no duplicate classification
   or apply logic.
2. WHEN the pipeline classifies a message THEN it SHALL produce exactly one `SettingsDecision`
   of: `Change`, `ReadBack`, `Undo`, `TempOverride`, `Clarify`, or `NotSettings`.
3. WHEN `KRIA_NL_SETTINGS` is falsy THEN the pipeline SHALL be inert and behaviour SHALL be
   byte-for-byte the legacy path (Property: legacy equivalence).
4. WHEN the same prompt is issued from chat and from the command surface THEN the resulting
   `SettingsDecision`, gating, and persisted effect SHALL be identical.
5. WHERE the pipeline needs config truth THEN it SHALL read/write ONLY through `ConfigService`
   (never `state.config` directly, never a raw file/DB write).

---

### Requirement 2: Configuration Intent vs Conversation Intent separation

**User Story:** As a user, I want KRIA to know when I'm talking about ITS settings versus talking
about my own project/code/topic, so "what is the current theme?" is answered correctly in both a
settings context and a normal conversation.

#### Acceptance Criteria
1. WHEN a message contains settings-like words (theme, voice, mode, search, model) THEN the system
   SHALL NOT assume Configuration Intent; it SHALL resolve intent using semantic similarity,
   schema grounding, conversation state, and subject/entity resolution.
2. WHEN the subject refers to the user's own artifacts ("the API key in my code", "my CSS theme",
   "switch branches") THEN the decision SHALL be `NotSettings` (Conversation Intent).
3. WHEN the subject refers to KRIA itself ("your theme", "the app's search engine", or a schema-
   grounded field in an imperative/interrogative about KRIA) THEN it SHALL be Configuration Intent.
4. WHEN prior conversation established a topic (e.g. discussing CSS) and a settings-like phrase
   appears THEN the classifier SHALL use that conversation state to bias toward Conversation
   Intent unless there is a clear KRIA-directed signal.
5. WHEN intent is genuinely ambiguous THEN the system SHALL ask ONE clarifying question and SHALL
   NOT mutate config.
6. WHEN classification confidence is below the act threshold THEN the system SHALL fail toward
   Conversation Intent (answer/clarify), never toward a mutation.

---

### Requirement 3: Settings understanding must generalize (no hardcoding, schema-driven)

**User Story:** As a maintainer, I want to add a new setting by declaring it once, so the NL
control understands it without new routing/keyword code.

#### Acceptance Criteria
1. WHEN a field is added to `KriaConfig` + the `FieldMeta` registry (risk, synonyms, valid_values,
   prompt_changeable) THEN the NL pipeline SHALL recognize prompts targeting it with NO new
   per-prompt or per-field routing code.
2. WHEN classifying/extracting THEN the system SHALL NOT use hardcoded prompt equality or literal
   `contains("theme"|"dark"|"voice"|...)` branches for field/value resolution; matching SHALL be
   driven by the schema (synonyms + semantic similarity + value sets).
3. WHEN the schema exposes 500+ fields THEN entity resolution SHALL scale (indexed/embedded
   lookup), not linear keyword chains, and SHALL remain correct.
4. WHERE a value must be mapped (e.g. "night mode" → theme=dark) THEN mapping SHALL come from
   schema synonyms/valid_values and/or LLM extraction constrained by the schema, not inline code.
5. WHEN a field is not in the registry THEN it SHALL be fail-closed (not prompt-changeable,
   high-risk).

---

### Requirement 4: Correct routing for every risk tier (RC1, RC5)

**User Story:** As a user, "turn off voice" or "enable remote desktop" must ask for approval, and
"switch to dark mode" must apply instantly — none should trigger a browser or web search.

#### Acceptance Criteria
1. WHEN a settings `Change` is recognized THEN it SHALL be routed to the Settings pipeline BEFORE
   any web/browser/GUI forcing layer or generic tool router can claim it.
2. WHEN the change targets a GREEN field THEN it SHALL apply immediately (no approval) via
   `ConfigService` and confirm to the user.
3. WHEN the change targets a YELLOW/RED/BLACK field THEN it SHALL require HITL approval through the
   SAME PolicyEngine + HitlGateway used by all tools; on approve it applies, on deny it does not.
4. WHEN a settings change is being decided THEN it SHALL NEVER be dispatched RAW (bypassing
   PolicyEngine/HITL); the single safety gate applies to all mutations regardless of entry path.
5. WHEN a settings prompt is recognized THEN `browser_search`, `searxng_search`,
   `search_marketplace`, `recall_fact`, and GUI tools SHALL NOT be triggered for it.

---

### Requirement 5: Read-back queries answered from ConfigService (RC2, NEW-2, NEW-6)

**User Story:** As a user, "what is my current theme / image mode / search engine?" must return my
actual setting, not a hallucination.

#### Acceptance Criteria
1. WHEN a message is classified `ReadBack` for a schema-grounded field THEN the answer SHALL be
   read from `ConfigService` (live effective value), never from memory/search/LLM guessing.
2. WHEN a `ReadBack` targets a secret field THEN the value SHALL be reported as set/unset only,
   never revealed.
3. WHEN `ReadBack` is ambiguous about which field THEN the system SHALL ask one clarifying
   question or list the closest schema-grounded candidates.
4. WHEN `ReadBack` runs THEN it SHALL NOT mutate config and SHALL work identically from chat and
   the command surface.
5. WHEN a read-back has no matching schema field THEN it SHALL fall to Conversation Intent
   (answer normally), not fabricate a setting.

---

### Requirement 6: Undo / revert / restore (RC3)

**User Story:** As a user, I want to reverse a settings change with natural phrasing — "undo that",
"revert", "change it back", "restore previous" — from chat or the command box.

#### Acceptance Criteria
1. WHEN the user expresses undo intent (any natural synonym) about a recent settings change THEN
   the system SHALL restore the prior value as a FORWARD patch (new audit entry, never a history
   deletion), via the shared handler.
2. WHEN undo intent is detected THEN synonym coverage SHALL be intent-based (semantic), not a fixed
   `contains("undo")` list.
3. WHEN there is nothing to undo THEN the system SHALL say so clearly and make no change.
4. WHEN undo targets a change that requires approval to reverse (e.g. re-enabling a RED field) THEN
   the reversal SHALL pass the same risk gate.
5. WHEN a cross-session recall ("same as yesterday") is requested THEN the system SHALL respond
   that referential memory is unavailable and offer the audit-based change history instead.

---

### Requirement 7: Temporary (turn-scoped) overrides via natural language (RC-temp)

**User Story:** As a user, "generate this image using local AI / using cloud" must affect only this
request and never permanently change my configuration.

#### Acceptance Criteria
1. WHEN a message expresses a temporary, whitelisted override THEN it SHALL apply only for the
   current turn via `RequestOverride` and SHALL NOT persist.
2. WHEN the turn ends (success, error, or crash) THEN the override SHALL be reverted with no leak
   to later turns.
3. WHEN a temp override targets a non-whitelisted field (auth/network/safety/secret) THEN it SHALL
   be refused.
4. WHEN the live turn's tools read config THEN they SHALL observe the override via the turn context
   (the override must be threaded into the actual turn, not just stored).

---

### Requirement 8: Validation, availability, and grounded rejection (RC7, invalid values)

**User Story:** As a user, "theme rainbow" or "use Google search engine" should be politely
rejected with the valid options, not silently applied or misrouted.

#### Acceptance Criteria
1. WHEN an extracted value is invalid for the field THEN the system SHALL reject it and state the
   allowed values (grounded reask), on both local and cloud LLM backends.
2. WHEN the target is schema-valid but not currently achievable (provider not configured, sidecar
   absent, env-locked) THEN the system SHALL refuse/clarify with the reason, not a silent apply.
3. WHEN a field is locked by an environment variable THEN both chat and command paths SHALL refuse
   with "locked by env: <VAR>".
4. WHEN value extraction on a cloud provider is required THEN it SHALL use structured output +
   strict schema validation + reject-and-reask; an unvalidated value SHALL never be applied.

---

### Requirement 9: Prompt-injection wall (NEW-3, security)

**User Story:** As a user, content from a web page, file, or tool output must never be able to
change my KRIA settings.

#### Acceptance Criteria
1. WHEN a settings change is attempted and the triggering content is not direct user input THEN it
   SHALL be refused (injection wall).
2. WHEN the turn has ingested external content (web/file/MCP/tool output) THEN provenance SHALL be
   marked such that subsequent config mutations in that turn are refused.
3. WHEN determining "external content" THEN the classification SHALL cover the ACTUAL tool names in
   the running system (e.g. `searxng_search`, `search_marketplace`, browser/fetch/file/MCP tools),
   verified against the live registry — not an outdated hardcoded list.
4. WHEN provenance is tracked THEN it SHALL be per-request/per-turn isolated (no global shared flag
   that bleeds across concurrent turns or sessions) (NEW-5).

---

### Requirement 10: Secret safety on every write path (NEW-1, NEW-4)

**User Story:** As a user, saving settings must never wipe or leak my secrets (JWT, Telegram token,
planner/HF keys, provider keys).

#### Acceptance Criteria
1. WHEN a whole-config save occurs THEN it SHALL preserve ALL secret fields (every field in
   `is_secret_field`), not just a subset; a redacted incoming blob SHALL NEVER overwrite a real
   stored secret with an empty/redacted value.
2. WHEN any field-level write path (UI `patch_config`, prompt `config_patch`, batch) targets a
   secret field THEN it SHALL be guarded (routed to the vault flow or refused), never written as
   plaintext/redacted into the config store.
3. WHEN a settings change is audited THEN secret values SHALL be redacted in the audit record.
4. WHEN secrets are preserved THEN the preserve-set SHALL be derived from `is_secret_field` (single
   source), so adding a secret field cannot silently reintroduce clobbering.

---

### Requirement 11: Consistency, persistence, and live sync (RC4, NEW-8, restart)

**User Story:** As a user, a settings change must persist across restart, reflect live in the UI,
and read back consistently everywhere.

#### Acceptance Criteria
1. WHEN a setting is changed by prompt THEN the UI SHALL reflect it live (config-changed event →
   re-fetch) and it SHALL survive an app/backend restart.
2. WHEN a change is applied THEN chat, the command surface, `get_settings`, and read-back SHALL all
   report the same value (no divergence).
3. WHEN a change is recorded THEN it SHALL appear in the durable audit/history with prior→new value
   and be undoable; whole-blob and field-level writes SHALL both be audited consistently.
4. WHEN a field requires restart to take effect THEN the system SHALL say so honestly (no faked
   live apply).

---

### Requirement 12: Performance and scale (NEW-7)

**User Story:** As a user, settings understanding must be fast and must not degrade as settings and
sessions grow.

#### Acceptance Criteria
1. WHEN classifying a message THEN expensive artifacts (schema synonym index, embeddings) SHALL be
   built once and reused (cached), not rebuilt per message/turn.
2. WHEN there are 500+ fields and many concurrent sessions THEN classification latency SHALL remain
   bounded and per-session state SHALL be isolated.
3. WHEN the pipeline runs on every turn THEN its added latency on a NON-settings message SHALL be
   negligible (cheap gate first, expensive stages only on likely-settings messages).

---

### Requirement 13: Real-frontend validation (mandatory for completion)

**User Story:** As the owner, I want proof that this works from the actual desktop UI, not just
backend tests.

#### Acceptance Criteria
1. WHEN the implementation is declared complete THEN the mandatory prompt suite (GREEN apply,
   read-back, ambiguity, false-positives, YELLOW approve/deny, invalid values, injection, temp
   overrides, undo synonyms, persistence, frontend history/badges) SHALL be verified through the
   REAL frontend (WebDriver/tauri-driver driving the actual chat UI + IPC), reproducing the human
   path: prompt → UI → backend → planner → handler → persistence → UI update.
2. WHEN a feature cannot be verified on the box (e.g. OS keychain) THEN it SHALL be marked honestly,
   never fabricated as passing.
3. WHEN validation runs THEN it SHALL use natural, human-style prompts (not only artificial ones)
   and cover the Conversation-vs-Configuration ambiguity in both directions.

---

### Requirement 14: No regressions to existing chat behaviour

**User Story:** As a user, adding settings understanding must not break normal chat, web search,
GUI automation, or n8n workflows.

#### Acceptance Criteria
1. WHEN a message is `NotSettings` THEN the existing chat/tool pipeline SHALL behave exactly as
   before (web search, GUI, n8n, knowledge, conversation).
2. WHEN the settings gate runs first THEN it SHALL only claim messages it is confident are settings
   intent; all others pass through untouched.
3. WHEN `KRIA_NL_SETTINGS` is off THEN none of the new stages SHALL execute.
