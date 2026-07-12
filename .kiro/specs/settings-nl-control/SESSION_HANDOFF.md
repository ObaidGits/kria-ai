# Session Handoff — settings-nl-control (resume at Wave 4)

> Permanent memory for the NEXT session. Read this + `requirements.md` + `design.md`
> (esp. **Wave 4 C1–C9** and **Wave 5 F1–F17**) + `tasks.md` before coding. Do NOT re-investigate.

## Where we are
Spec `.kiro/specs/settings-nl-control/` is the ACTIVE spec. It fixes the live-chat failure where
settings prompts misrouted to browser/search/marketplace/recall and read-backs hallucinated.
**ALL WAVES 0–6 COMPLETE. Spec DONE — backend suite + real-frontend WebDriver validation green.**

Milestones: M1=Wave0 (done), M2=Waves1–2 (done), M3=Waves3–4 (done),
M4=Wave5 (backend suite done) + Wave6 (real-frontend WebDriver validation — DONE, 14 pass / 1
honestly skipped). No pending tasks.

## Flag
`KRIA_NL_SETTINGS` is now **ON by default** (fully integrated — no env var needed to use NL
settings control). Opt OUT only with an explicit falsy value: `KRIA_NL_SETTINGS=0|false|no|off`.
Single source of truth: `pub fn config::nl::nl_settings_enabled()` (default true), called by the
loop gate (`run_settings_stage`), the `config_prompt` command, and the `config_patch` tool. Legacy
`KRIA_CONFIG_PROMPT_CONTROL` truthy still enables (folded in) but can no longer force-off. Storage
uses `KRIA_CONFIG_BACKEND=sqlite` (already default sqlite).

## DONE — Wave 0 (safety hotfixes, always-on)
- **Task 1 (NEW-1/4/8):** `KriaConfig::preserve_secrets_from(&current)` in `config/mod.rs`
  (derived from `is_secret_field`, JSON-driven). `update_settings` (`app_commands.rs`) calls it →
  redacted whole-blob save can't wipe jwt/telegram/planner/hf secrets. `patch_config` command
  refuses `is_secret_field`. Tests: `config::secret_preservation_tests`.
- **Task 2 (NEW-3):** `ToolRegistry::is_external_content_tool(&self, name)` is now `&self`,
  category-driven (internet/web/news/knowledge/marketplace/rag + `mcp_` prefix + `is_file_read_op`).
  No invented name list.
- **Task 3 (NEW-5):** removed global `AtomicBool` taint from `ToolRegistry`. Per-turn taint is a
  local `Arc<AtomicBool>` in `run_with_profile`; `make_tool_context_with_provenance(cancel, prov)`
  added; `make_tool_context` defaults `User`. `reset_turn_provenance`→`clear_turn_override`.
  ReAct dispatch (~9452) decides `call_provenance` per-turn then taints later calls.
  Tests: `tools::registry::provenance_tests`.

## DONE — Wave 1 (shared handler)
- **Task 4/5:** `crates/kria-core/src/config/nl/{mod,handler}.rs`.
  - `SettingsRequest{kind,section,field,value:Option,scope,provenance,session_id}` →
    `SettingsOutcome{Applied|Answer|NeedsApproval{change_set_id}|Clarify|Refused|TempApplied|Undone|NothingToUndo}`.
  - `SettingsHandler::new(Arc<ConfigService>).with_audit(Arc<AuditLogger>)`; `handle()`,
    `apply_approved(change_set_id)`, `resolve(req, &dyn ApprovalDriver)`.
  - Change: injection wall (provenance!=User→Refused) → secret guard → schema validate (grounded
    reject w/ allowed values) → env-lock → risk (GREEN auto `ConfigService.patch`; else NeedsApproval).
  - ReadBack: `ConfigService.read_field` (wires the previously-dead fn — NEW-2); secret→set/unset.
  - Undo: in-memory ring → **durable fallback** to `AuditLogger::config_change_history` (F12).
  - `ApprovalDriver` trait (no HitlSink in handler — F1); caller drives approval.
  - Tests: `config::nl::handler::tests` (11) incl. durable-undo-after-restart.

## DONE — Wave 2 (intent classifier, "intent not keywords")
- **Task 15:** `config/nl/golden_set.jsonl` (41 cases, `include_str!`ed by the classifier test).
  Expanded FieldMeta synonyms in `config/schema.rs` (search.engine, server.enable_auth,
  safety.emergency_mode, remote_desktop.enabled, mobile.enabled — split shared match arms).
- **Task 6:** `config/nl/entity_index.rs` — `SchemaEntityIndex::build()` (from FieldMeta synonyms +
  field/section tokens + valid_values; secrets excluded). `resolve()` ranks; `resolve_value()`
  schema-bounded (valid_values, generic boolean on/off, night/local/cloud hints, raw "to <word>"
  fallback so invalid values reach the handler's grounded reject). Tier-A lexical (offline). Tests: 5.
- **Task 7:** `config/nl/conversation.rs` — `ConversationContext` (strings, decoupled).
  `subject_signal`→{KriaDirected|UserArtifact|Neutral}; `code_topic_active()`. Tests: 4.
- **Task 8:** `config/nl/pipeline.rs` — `SettingsIntentPipeline::new(Arc<SchemaEntityIndex>)`;
  `classify(text,&conv)` / `classify_traced`. Staged: undo-intent → entity (value-grounded
  disambiguation) → conv-vs-config → intent-kind (question+self-ref→ReadBack; "what is X"
  definitional→NotSettings; imperative→Change/Temp) → implicit-command fallback (grounded value +
  ≥3 words + not-complaint → hinglish) → bare-noun→Clarify. Tunable `IntentThresholds`
  (act .72/clarify .45/floor .40). `SettingsDecision{Change{scope}|ReadBack|Undo|Clarify|NotSettings}`.
  Golden 41/41 green.

## DONE — Wave 3 (turn integration)
- **Task 9:** `AgentLoop::run_settings_stage(session_id, last_user_text, messages, event_tx)` in
  `loop_engine/mod.rs`. Called AFTER `TurnAccepted` (~5310), BEFORE IntentGate(5341)/forcing/
  deterministic(5741), gated by `nl_settings_enabled()`. Cached `SETTINGS_ENTITY_INDEX`
  (`once_cell::Lazy`). Runs shared `SettingsHandler` through REAL HITL via
  `ChatSettingsApprovalDriver` (emits `StreamEvent::ApprovalRequired` + `HitlGateway
  .request_approval_with_id`). Renders `SettingsOutcome` (Token/Done). `SettingsStageResult{Claimed|
  ContinueWith(String)|Pass}`. `last_user_text` made `mut`; multi-intent rewrites it.
  Helpers: `render_settings_outcome`, `settings_multi_intent_remainder`,
  `build_settings_conversation_context`, `ChatSettingsApprovalDriver`, `nl_settings_enabled`.
  `ToolRegistry::config_service()` getter added.
- **Task 10:** `Scope::Temp` → `set_turn_override` + `Pass` (turn continues; `generate_image` reads
  `ctx.effective_config()` → force_local/force_cloud, already wired). Per-turn, auto-revert.
- Tests: `loop_engine::tests` (multi_intent split/none, conversation-context, flag). 119 loop tests green.

## DONE — Wave 4 (Tasks 11, 16, 12)
- **Task 11 — Command-surface parity:** `commands/config_prompt.rs` rewritten as a THIN caller of
  the shared `SettingsIntentPipeline` + `SettingsHandler` (cached `command_entity_index()`).
  Deleted duplicate undo/keyword/`PromptAnalyzer`/`evaluate` logic. `CommandSettingsApprovalDriver`
  (emits `agent:approval_required` + `state.hitl.request_approval_with_id`) mirrors
  `ChatSettingsApprovalDriver`; approval via `handler.resolve()`. `outcome_to_json` maps
  `SettingsOutcome`→UI shape (applied/answer/clarify/refused/temp_requested/undone/
  nothing_to_undo/not_a_change). Gate = `nl_settings_enabled()`. Cross-session recall branch kept.
- **Task 16 — Retired duplicate deciders (F15/F16):** removed `config_prompt_control_enabled`,
  `try_config_prompt_dispatch`, `build_turn_override` + both call sites from `loop_engine/mod.rs`
  (settings intent + temp overrides now flow ONLY through `run_settings_stage`). `config/prompt/mod.rs`
  reduced to the `Scope` type only; `config/prompt/patch.rs` DELETED (`evaluate`/`PatchOutcome`/
  `apply_approved`); `PromptAnalyzer`/`ConfigIntent`/`ConfigIntentTrace`/`TopicHint` gone. Migrated
  `tests/settings_e2e.rs` to `SettingsHandler`; removed obsolete loop tests. Flag migration:
  `config_patch` tool + command + loop accept `KRIA_NL_SETTINGS` OR legacy `KRIA_CONFIG_PROMPT_CONTROL`.
- **Task 12 — RC7:** `fts5_match_query()` in `memory/store.rs` sanitizes raw query into a quoted-token
  MATCH expr (fixes `recall_fact` crash on `"?"`); applied to `search_facts` + `search_conversations`
  (NOT `search_chunks` — `rag.rs` already sanitizes). 3 unit tests. Settings read-backs are claimed
  by the settings gate so marketplace/recall no longer fire (e2e-verified in Task 14).
- Green: `config::nl` (22), `agent::loop_engine::tests` (111), `tools::config_patch` (4),
  `memory::store::fts5_sanitize_tests` (3), `settings_e2e` (1). `cargo check -p kria-core -p
  kria-desktop --tests` clean. (Pre-existing clippy-only lint: `since = "batch-1"` in
  `automation/workflows.rs` — unrelated, not touched.)

## DONE — Wave 5 (Task 13)
- `crates/kria-core/src/config/nl/properties.rs` (`#[cfg(test)] mod properties`) — P1–P10 through
  the SHARED `SettingsIntentPipeline` + `SettingsHandler` (11 tests, all green). Added
  `SchemaEntityIndex::push_synthetic_field` (cfg(test) seam) for the P8 no-hardcoding proof.
  Golden set (41/41) + 500-field scaling/latency already green.
- `cargo test --workspace`: kria-core lib 2997 pass / 1 pre-existing flake
  (`agent::continuation_reentry::tests::duplicate_continuation_is_rejected` — shared process-global,
  passes with `--test-threads=1`, UNRELATED to this spec); kria-desktop 133, kria-server + others green.

## DONE — Wave 6 (Task 14 — real-frontend WebDriver)
- Harness: `tests/gui-cognition-e2e/specs/settings_nl_control.e2e.ts` + `scripts/run_settings_nl_e2e.sh`.
- Built with `cargo tauri build --debug --no-bundle` (embedded frontend); launched via tauri-driver
  + WebKitWebDriver (DISPLAY=:1), isolated `HOME=/tmp/kria-e2e-home`, `KRIA_NL_SETTINGS=1`,
  `KRIA_CONFIG_BACKEND=sqlite`. `withGlobalTauri` toggled on to run, then REVERTED (committed off).
- 14 passing / 1 skipped: GREEN apply (theme dark/light); YELLOW search.engine → HITL approve →
  applied (NOT browser — the live-failure prompt, now correct); read-backs; false-positives (no
  mutation); invalid → grounded reject; YELLOW voice approve+deny via real gate; temp no-persist;
  undo synonym; CHAT via `send_message` (apply + false-positive); persistence-on-disk (python3
  sqlite3: search.engine=duckduckgo, ui.theme=dark, voice.enabled=true).
- Honest gaps (Req 13.2, not fabricated): CHAT-UI textarea DOM typing SKIPPED (webview textarea not
  reachable via WebDriver DOM in this webkit build — chat path validated via `send_message` IPC, the
  exact command the Send button calls); fetched-webpage injection not driven multi-turn (backend
  P6-proven); OS keychain on-box manual (unverified).
- To re-run: set `withGlobalTauri: true` in `crates/kria-desktop/tauri.conf.json`, rebuild, then
  `bash scripts/run_settings_nl_e2e.sh`; revert the flag after.

## (archived) NEXT pointers — all tasks complete

- **Task 13:** full backend suite + property tests P1–P10 (incl. no-hardcoding synthetic-field test,
  500-field scaling/latency, flag-off parity, LLM-optional). `cargo test --workspace`.
- **Task 14 (MANDATORY):** real-frontend WebDriver validation. Rebuild
  `cargo tauri build --debug --no-bundle` (plain `cargo build` has NO embedded frontend →
  "Connection refused"). Launch via tauri-driver + WebKitWebDriver, isolated `HOME=/tmp/kria-e2e-home`,
  `KRIA_NL_SETTINGS=1 KRIA_CONFIG_BACKEND=sqlite`. Harness: `tests/gui-cognition-e2e/`. Temporarily
  set `withGlobalTauri: true` in tauri.conf.json for WebDriver `invoke`, then REVERT. Run the full
  mandatory suite from `tasks.md` Task 14 through the REAL chat UI. `sqlite3` CLI NOT installed →
  use `python3` sqlite3 module for DB inspection. Verify + report honestly (keychain on-box manual).

## Verify commands (per task)
- `cargo check -p kria-core` / `-p kria-desktop` / `--workspace`
- `cargo test -p kria-core --lib config::nl::` (pipeline/handler/entity/conversation)
- `cargo test -p kria-core --lib config::` (94 tests) ; `agent::loop_engine::tests::` (119)
- Known pre-existing flake: `agent::continuation_reentry::tests::*` fails under full parallel run
  (shared process-global); passes with `--test-threads=1`. UNRELATED to this spec.

## Hard rules (do not violate)
One shared handler (no second decider); no raw mutation path (all go through PolicyEngine+HITL);
intent not keywords (schema-driven, no `contains("theme")` field/value branches); fail toward
conversation; secrets never clobbered/leaked (preserve-set = `is_secret_field`); injection wall via
live-registry capability + per-turn provenance; real-frontend validation before declaring complete.

## Env / box notes
- Rust workspace; `cargo tauri` available; tauri-driver + WebKitWebDriver installed; DISPLAY=:1.
- Work in Hinglish for chat explanations; keep CodeScout `Caveman mode: ON` + `GraphMode: ON`
  indicator lines (per AGENTS.md / .kiro/steering).
- Non-stop execution authorized; stop only on major issue / completion / manual-interference-needed.
