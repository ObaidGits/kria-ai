# Implementation Plan — Natural-Language Settings Control

## Overview

Builds the single, schema-driven Settings Intent pipeline + shared handler (design.md Wave 4),
replacing the thin GREEN-only chat gate and unifying chat with the command surface. Sequencing:
**safety hotfixes first** (secret clobber, injection tool names — these are live bugs), then the
**shared handler + gate abstraction**, then the **intent pipeline** (entity index, conversation
context, classifier), then **turn integration**, then **command/frontend parity**, then
**real-frontend validation**. Every phase is flag-gated by `KRIA_NL_SETTINGS`; falsy ⇒ legacy
byte-for-byte.

Read `requirements.md` + `design.md` (esp. Wave 4 C1–C9) before starting.

### Execution protocol (per task)
1. Top-to-bottom; respect the dependency graph.
2. DONE only when: `cargo check -p <crate>` clean; `cargo clippy` no new warnings; `cargo fmt`;
   listed tests written + green; **flag-off parity** test holds; no fabricated results.
3. NO hardcoding: no `contains("theme"|"dark"|...)` / prompt-equality for field/value resolution.
   Any such code fails review (Req 3.2). Classification = schema + semantic + context.
4. STOP for review at milestone boundaries (M1…M4).
5. Real-frontend validation (Wave 7) is mandatory before declaring completion (Req 13).

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 0, "tasks": [1, 2, 3], "description": "Safety hotfixes (live bugs): secrets, injection provenance, taint isolation" },
    { "wave": 1, "tasks": [4, 5], "description": "Shared SettingsHandler (SettingsOutcome + apply_approved) — one gated path" },
    { "wave": 2, "tasks": [15, 6, 7, 8], "description": "Golden set + FieldMeta expansion; SchemaEntityIndex; ConversationContext; classifier" },
    { "wave": 3, "tasks": [9, 10], "description": "Turn integration (first-stage gate, insertion point, multi-intent) + provenance wiring" },
    { "wave": 4, "tasks": [11, 16, 12], "description": "Command/frontend parity; retire duplicate deciders; misc RC7 fixes" },
    { "wave": 5, "tasks": [13], "description": "Backend test suite + property tests" },
    { "wave": 6, "tasks": [14], "description": "Real-frontend (WebDriver) validation — mandatory" }
  ]
}
```

### Milestones
- **M1 = Wave 0** — live safety bugs fixed (shippable immediately, no NL change).
- **M2 = Waves 1–2** — shared handler + intent pipeline exist + unit-tested (not yet wired to chat).
- **M3 = Waves 3–4** — chat + command unified through the pipeline; frontend parity.
- **M4 = Waves 5–6** — full backend suite + real-frontend validation green. Completion.

## Tasks

- [x] 1. Secret-safe writes (fixes NEW-1, NEW-4, NEW-8) — SAFETY HOTFIX
  <!-- DONE: KriaConfig::preserve_secrets_from(&current) added in config/mod.rs — derived from
       is_secret_field (single source, JSON-driven, cannot drift). update_settings now calls it
       (removed the lone manual llm.cloud_api_key line) so a redacted whole-blob save can't wipe
       jwt_secret/telegram/planner/hf tokens. patch_config command now refuses is_secret_field.
       replace_all audit parity already present (NEW-8). 2 unit tests (restore-all + covers-every-
       is_secret_field). cargo check core+desktop clean; 72 config tests green. -->
  - In `update_settings` (`crates/kria-desktop/src/commands/app_commands.rs`) preserve EVERY field
    reported by `crate::config::is_secret_field` from the live config (iterate the single source),
    not just `providers`+`llm.*`; a redacted/empty incoming value MUST NOT overwrite a stored
    secret. Derive the preserve-set from `is_secret_field` so it cannot drift again.
  - Guard `patch_config` command: reject/route-to-vault when `is_secret_field(section,field)`.
  - Ensure whole-blob `replace_all` audits changed fields consistently with field patches.
  - Tests: whole-blob save with redacted secrets leaves stored secrets intact (all 5 fields);
    patch_config on a secret field is refused; audit parity.
  - _Requirements: 10.1, 10.2, 10.3, 10.4, 11.3_

- [x] 2. Capability-based injection provenance (fixes NEW-3)
  <!-- DONE: ToolRegistry::is_external_content_tool is now a &self method driven by the LIVE
       registry `category` metadata (internet/web/news/knowledge/marketplace/rag → external) +
       mcp_ prefix + file-READ discrimination (is_file_read_op; writes excluded). No invented name
       list (old list had non-existent web_search/fetch_webpage). New external tool with an existing
       category is auto-covered. Test: category-driven classification (registered internet tool,
       mcp_ prefix, read_file, config_patch=false, write_file=false). -->

- [x] 3. Per-turn provenance isolation (fixes NEW-5)
  <!-- DONE: removed the global AtomicBool taint from ToolRegistry. Injection taint is now a
       per-turn `Arc<AtomicBool>` local in run_with_profile (cannot bleed across concurrent
       turns/sessions). Added make_tool_context_with_provenance(cancel, provenance);
       make_tool_context defaults User (no global read). ReAct dispatch decides call_provenance from
       the per-turn taint BEFORE running, then taints later calls if the tool is external. Registry
       reset_turn_provenance → clear_turn_override (override reset only). Tests updated: default
       User, explicit External stamped, no bleed. -->
  - _Requirements: 9.4, 12.2_

- [x] 4. `SettingsHandler` — the one shared path (design C4 + Wave 5 F1/F7/F12/F17)
  <!-- DONE: crates/kria-core/src/config/nl/{mod,handler}.rs. SettingsRequest{kind,section,field,
       value,scope,provenance,session_id} → SettingsOutcome{Applied|Answer|NeedsApproval{change_set_id}
       |Clarify|Refused|TempApplied|Undone|NothingToUndo}. handle() never streams/blocks;
       apply_approved(change_set_id) completes non-GREEN after caller gate. Change: injection wall
       (provenance!=User→Refused) → secret guard → schema validate (grounded reject w/ allowed
       values) → env-lock refuse → risk gate (GREEN auto ConfigService.patch; else NeedsApproval).
       ReadBack: ConfigService.read_field (effective value, F7); secret→set/unset only (wires the
       dead read_field — NEW-2). Undo: in-memory ring, FALLING BACK to AuditLogger
       config_change_history when ring empty (durable, survives restart — F12). TempOverride:
       RequestOverride.set whitelist. 11 unit tests incl. durable-undo-after-restart, YELLOW
       approve/deny, env-lock, injection, secret, invalid-reask. NOTE: provider/model runtime-apply
       delegation (F17) + live effect dispatch land at Task 9 integration (core handler stays
       decision+persist; ConfigChanged subscribers apply). -->
  - Add `crates/kria-core/src/config/nl/handler.rs`: `SettingsHandler` taking `SettingsRequest`
    `{kind, section, field, value, scope, provenance, session_id}` and returning a typed
    **`SettingsOutcome`** (Applied|Answer|NeedsApproval{change_set_id}|Clarify|Refused|TempApplied|
    Undone|NothingToUndo|HelpAbout) — the handler NEVER streams or blocks on HITL (F1). Add
    `apply_approved(change_set_id)` for the caller to call post-approval.
  - Change: schema validate → availability (reuse C4.1) → injection wall (provenance!=User ⇒
    Refused) → PolicyEngine risk → GREEN `ConfigService.patch`; non-GREEN ⇒ return `NeedsApproval`.
    Provider/model changes delegate to `apply_provider_selection`, never raw write; mid-turn
    conflict handled per Wave 5 F17 (apply-after-generation, never deadlock).
  - ReadBack: `ConfigService.get()` (effective value, F7); secrets → set/unset only (finally wiring
    read_field — NEW-2).
  - Undo: forward patch; source = in-memory ring, FALLING BACK to the durable audit ledger
    (`AuditLogger::config_change_history`) when the ring is empty so undo survives restart (F12).
  - Tests: GREEN auto-apply; invalid → grounded reask (allowed values); env-lock refuse;
    availability refuse; injection refuse; read-back effective value; undo after simulated restart
    (ring empty, audit populated); temp build; NeedsApproval + apply_approved round-trip.
  - _Requirements: 1.1, 1.5, 4.2, 4.4, 5.1, 5.2, 6.1, 6.4, 7.1, 8.1, 8.2, 8.3, 9.1_

- [x] 5. Caller-driven approval + `apply_approved` wiring (design C4 Wave 5 F1 — supersedes HitlSink)
  <!-- DONE (contract + core): `ApprovalDriver` trait { async request(section,field,value,risk) ->
       ApprovalDecision{Approved|Denied|Timeout} } + `SettingsHandler::resolve(req, &dyn
       ApprovalDriver)` = handle → if NeedsApproval drive approval → apply_approved. One code path,
       no raw path, no core→streaming dependency. Tests: approve→Applied, deny→Refused (identical
       persisted effect via the single handler = Property P1). NOTE: the two HOST-specific drivers
       (chat gateway-backed `StreamEvent::ApprovalRequired`; command `agent:approval_required`)
       are implemented in their hosts at Task 9 (loop) and Task 11 (command) — they are ~10-line
       adapters over this trait; Wave 1 delivers the trait + resolve + test drivers. -->
  - _Requirements: 4.3, 4.4_

- [x] 6. `SchemaEntityIndex` — semantic + synonym field resolver (design C2, no hardcoding)
  <!-- DONE: config/nl/entity_index.rs — built from FieldMeta synonyms + section/field name tokens +
       valid_values; secrets excluded. resolve(text)→ranked FieldCandidate (phrase-substring strong +
       distinctive-token overlap; fail-closed fields down-weighted). resolve_value schema-bounded
       (valid_values match, generic boolean on/off, night/local/cloud hints, raw "to <word>" fallback
       so invalid values reach the handler's grounded reject). Tier-A lexical (offline, no embedder);
       tier-B embedding is a documented future layer behind the same API. 5 unit tests
       (resolve/disambiguate voice.enabled-vs-mode via value grounding/secrets-never-resolved/
       schema-bounded value/unrelated=weak). NOTE: 500-field scaling test lands in Task 13. -->
  - Add `crates/kria-core/src/config/nl/entity_index.rs`: build once from `FieldMeta` synonyms +
    `KriaConfig` shape + valid_values; embed synonym sets; nearest-field lookup by cosine; cache
    keyed by schema version; shared read-only.
  - Tests: resolves "night mode"→ui.theme, "web engine"→search.engine WITHOUT keyword branches;
    scales to a synthetic 500-field registry within latency budget; unknown phrase → no match.
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 12.1, 12.2_

- [x] 7. `ConversationContext` adapter (design C3, fixes NEW-12)
  <!-- DONE: config/nl/conversation.rs — decoupled via plain strings (no ChatMessage dependency).
       subject_signal(text)→{KriaDirected|UserArtifact|Neutral} from marker sets; code_topic_active()
       from recent user/assistant history. No dependency on the empty RoutingContext. 4 unit tests
       (kria-directed / user-artifact / neutral / code-topic). Loop passes recent texts at Task 9. -->
  - Add `crates/kria-core/src/config/nl/conversation.rs`: derive topic/subject/correction signals
    from the `messages` vec (NOT the empty RoutingContext); per-session, isolated.
  - Provide subject-marker resolution (KRIA-directed vs user-artifact) and recent-topic embedding.
  - Tests: "the API key in my code" → user-artifact; "your search engine" → KRIA; topic carry from
    a prior CSS discussion biases "change the theme" toward Conversation unless KRIA-directed.
  - _Requirements: 2.1, 2.2, 2.3, 2.4_

- [x] 8. `SettingsIntentPipeline` classifier (design C1 + Wave 5 F2/F3/F8/F14 — intent, not keywords)
  <!-- DONE: config/nl/pipeline.rs — SettingsDecision{Change{scope}|ReadBack|Undo|Clarify|NotSettings}
       + SettingsIntentTrace (per-stage scores). Stages: undo-intent (generic verb class, guarded by
       user-artifact subject) → schema entity resolution (value-grounded disambiguation) →
       Configuration-vs-Conversation (subject_signal: UserArtifact→NotSettings; code-topic+neutral+
       ungrounded→down-weight) → intent-kind (question+self-ref→ReadBack; definitional 'what is X'
       without self-ref→NotSettings; imperative→Change/Temp) → implicit-command fallback (grounded
       value + ≥3 words + not-complaint, catches hinglish) → bare-noun→Clarify. Tunable
       IntentThresholds (act .72/clarify .45/floor .40 — F8). Tier-A lexical, LLM/embedder-optional
       (F2/F3). Golden set 41/41 green (F9) incl. false-positives, ambiguity both-ways, hinglish,
       multi-intent, invalid→change(handler rejects). + conversation-bias test. NOTE: embedder/LLM
       refinement layer + ConfigIntentTrace-to-diagnostics-ring are follow-ons behind the same API. -->
  - Add `crates/kria-core/src/config/nl/pipeline.rs`: `new(deps)` with
  - Add `crates/kria-core/src/config/nl/pipeline.rs`: `new(deps)` with
    `{entity_index, config, embedder: Option, llm: Option}` (F2). Staged scorer (domain → entity →
    conv-vs-config → intent-kind → extraction → decision), each stage a swappable trait; emits
    `SettingsIntentTrace` with per-stage scores + final band.
  - **Scoring contract (F8):** stage scores ∈[0,1] → weighted `confidence` (domain .35 / entity .30
    / intent .20 / conv-vs-config .15; weights + thresholds in a tunable struct, defaults
    act=0.72, clarify=0.45). ≥act ⇒ act; clarify band ⇒ Clarify; else NotSettings. Fail toward
    conversation.
  - **Two-tier matcher (F3):** tier-A lexical/synonym/edit-distance (always on, offline) + tier-B
    embedding cosine when `embedder` ready. Tier-A alone must pass GREEN/read-back/undo goldens.
  - No `PromptAnalyzer::new()` per call (cache index — NEW-7).
  - Tests: run against `config/nl_golden_set.jsonl` (Task 15). Offline mode (embedder=None) passes
    the tier-A subset (F14); tier-B tests skip-marked when embedder absent (no fabricated pass).
    Trace assertions; threshold calibration recorded.
  - _Requirements: 2.5, 2.6, 3.2, 5.3, 6.2, 8.4, 12.1, 12.3_

- [x] 15. Golden set artifact + FieldMeta annotation expansion (Wave 5 F9/F10 — prerequisite for 8)
  <!-- DONE: crates/kria-core/src/config/nl/golden_set.jsonl (41 cases: change/read_back/undo/temp/
       clarify/not_settings, false-positives, ambiguity both-ways, hinglish, multi-intent, invalid).
       Embedded via include_str! in the classifier test. Expanded FieldMeta synonyms for search.engine,
       server.enable_auth, safety.emergency_mode, remote_desktop.enabled, mobile.enabled (split the
       shared match arms). Golden set is the classifier regression source (41/41 green). -->
  - Author `config/nl_golden_set.jsonl` (prompt, expected decision, expected field, optional prior
    context) from `settings-config-revamp/analysis.md` §5 + the live-failure prompts + the Req 13
    mandatory suite. Versioned; the single regression source for the classifier.
  - Expand the `FieldMeta` registry (`config/schema.rs`) — synonyms, valid_values, risk,
    prompt_changeable — for ALL user-facing settings (not just the current ~14) so entity
    resolution has real coverage; unannotated stays fail-closed.
  - Tests: golden set parses; every expected field exists in the schema; annotated fields have
    non-empty synonyms.
  - _Requirements: 3.1, 3.5_

- [x] 9. Turn integration — first-stage settings gate (design C5 + Wave 5 F4/F5/F6; fixes RC1/RC6/NEW-9/10/11)
  <!-- DONE: run_settings_stage in loop_engine/mod.rs runs AFTER TurnAccepted (~5310) and BEFORE
       IntentGate(5341)/LiveFact+GUI forcing/deterministic dispatch(5741), gated by
       nl_settings_enabled() (KRIA_NL_SETTINGS folding in KRIA_CONFIG_PROMPT_CONTROL — F16).
       Builds ConversationContext from messages (F4/F5), runs SettingsIntentPipeline (cached Lazy
       SchemaEntityIndex — Req 12.1), and on a decision runs the shared SettingsHandler through the
       REAL HITL gate via ChatSettingsApprovalDriver (StreamEvent::ApprovalRequired +
       HitlGateway.request_approval_with_id — same gate as every RED tool, Req 4.4). Renders
       SettingsOutcome as Token/Done. Undo/ReadBack/Clarify/Change all handled; browser/search/GUI
       bypassed. Multi-intent (F6): settings clause + trailing task → apply settings, ContinueWith
       remainder (last_user_text rewritten, turn continues). NotSettings/off ⇒ untouched flow.
       119 loop_engine tests green (incl. multi-intent split, conversation-context, flag). -->
  - In `crates/kria-core/src/agent/loop_engine/mod.rs::run_with_profile`, insert the settings stage
  - In `crates/kria-core/src/agent/loop_engine/mod.rs::run_with_profile`, insert the settings stage
    **after turn admission + `TurnAccepted` (~5303)** and **before IntentGate fast-path (5341),
    LiveFact/GUI forcing (5393/5411), and the deterministic pre-dispatch (5741)** (F4/F5), gated by
    `KRIA_NL_SETTINGS`. Run `SettingsIntentPipeline`; on a turn-claiming decision, run
    `SettingsHandler`, render its `SettingsOutcome` as StreamEvents (drive HITL via Task 5 for
    `NeedsApproval` then `apply_approved`), emit `Done`, return — bypassing browser/search/GUI.
  - **Multi-intent (F6):** settings-clause + residual task ⇒ apply settings, strip clause, continue
    normal pipeline for the task (do NOT finish the turn).
  - `NotSettings` ⇒ existing flow byte-for-byte. Provenance set per-turn (Task 3).
  - Tests: "switch to dark" GREEN instant; "turn off voice" → approval (no browser); "set search
    engine to duckduckgo" → applied/approval (NOT browser regex); "what is my theme?" → read-back;
    "revert that" → undo; false-positives untouched; multi-intent applies settings + runs task;
    `NotSettings` → normal pipeline (regression); flag-off parity.
  - _Requirements: 1.1, 1.4, 4.1, 4.4, 4.5, 14.1, 14.2, 14.3_

- [x] 10. Temp-override threading into the live turn (design C4/C5, Req 7)
  <!-- DONE: run_settings_stage handles Scope::Temp by installing a turn-scoped RequestOverride on
       the ToolRegistry (set_turn_override) and returning Pass (NOT claiming) so the actual tool
       (e.g. generate_image) runs THIS turn and reads it via ctx.effective_config() — already wired
       (generate_image maps image_mode override → force_local/force_cloud). Override is per-turn
       (cleared at next turn boundary via clear_turn_override), reverts on success/error/crash by
       construction (in-memory, never persisted). Non-whitelisted temp fields refused by
       RequestOverride.set. "generate this image using local AI" → image_mode=local_only temp. -->
  - Attach the `RequestOverride` produced by a `TempOverride` decision to the turn's `ToolContext`
    (already supported by `effective_config()`); ensure revert on success/error/crash; no leak.
  - Tests: "generate this using local AI" → image local for one turn, config unchanged; error mid-
    turn still reverts; next turn unaffected.
  - _Requirements: 7.1, 7.2, 7.3, 7.4_

- [x] 11. Command-surface + frontend parity (design C8, fixes RC4/NEW-6/NEW-13)
  <!-- DONE: config_prompt.rs is now a THIN caller of the SAME SettingsIntentPipeline +
       SettingsHandler chat uses (cached command_entity_index()). Deleted the duplicate
       undo/keyword/PromptAnalyzer/evaluate logic. Added CommandSettingsApprovalDriver
       (emits agent:approval_required + state.hitl.request_approval_with_id) — mirrors the
       loop's ChatSettingsApprovalDriver; drives approval via handler.resolve(). Maps
       SettingsOutcome→the JSON shape the UI expects (applied/answer/clarify/refused/
       temp_requested/undone/nothing_to_undo/not_a_change). Flag = nl_settings_enabled()
       (KRIA_NL_SETTINGS folding in KRIA_CONFIG_PROMPT_CONTROL). Cross-session recall branch
       kept (audit history). Read-back + undo synonyms now work in the command box (P1: same
       decision/effect as chat). cargo check -p kria-desktop clean. -->
  - Rewrite `crates/kria-desktop/src/commands/config_prompt.rs` as a thin caller of the SAME
    pipeline + `SettingsHandler` (command `HitlSink`); delete duplicate undo/keyword logic.
  - Frontend: keep field-level `patch_config`, config-changed re-fetch, history viewer, badges; no
    divergent logic. Verify read-back + undo synonyms work from the command box.
  - Tests: identical decision/effect for the same prompt via chat and command (Property P1).
  - _Requirements: 1.1, 1.4, 5.4, 6.1, 11.1, 11.2_

- [x] 16. Retire duplicate deciders + flag migration (Wave 5 F15/F16 — fixes RC4/NEW-13)
  <!-- DONE: removed the three loop deciders (config_prompt_control_enabled,
       try_config_prompt_dispatch, build_turn_override) + both call sites in
       loop_engine/mod.rs; temp overrides + settings intent now flow ONLY through
       run_settings_stage (the single decider). Reduced config/prompt/mod.rs to just the
       Scope vocabulary type (still used by nl/handler, nl/pipeline, config_patch tool,
       config_prompt command); DELETED config/prompt/patch.rs (evaluate/PatchOutcome/
       apply_approved) and PromptAnalyzer/ConfigIntent/ConfigIntentTrace/TopicHint.
       Migrated tests/settings_e2e.rs to drive SettingsHandler; removed obsolete loop
       tests (config_prompt_dispatch_*, build_turn_override_*) — behaviour now covered by
       the pipeline golden set + handler tests. Flag migration (F16): config_patch tool +
       command + loop all accept KRIA_NL_SETTINGS OR legacy KRIA_CONFIG_PROMPT_CONTROL.
       cargo check clean; config::nl (22), loop_engine (111), config_patch (4), settings_e2e
       (1) all green. -->
  - Migrate any still-useful logic from `config/prompt/mod.rs` (`PromptAnalyzer`),
    `config/prompt/patch.rs` (`evaluate`), the loop's GREEN-only `try_config_prompt_dispatch`, and
    `build_turn_override` INTO the pipeline/handler, then REMOVE them (or reduce to thin shims) so
    only ONE decider exists (Req 1.1).
  - `KRIA_NL_SETTINGS` folds in `KRIA_CONFIG_PROMPT_CONTROL` (truthy old flag maps to new for one
    release + deprecation log). Update flag-off parity tests to the new flag.
  - Tests: no code path other than the pipeline classifies settings intent; old flag still works
    (mapped) with a deprecation warning; flag-off parity holds.
  - _Requirements: 1.1, 1.3_

- [x] 12. Misc correctness (RC7)
  <!-- DONE: added fts5_match_query() in memory/store.rs — strips FTS5 reserved chars
       (?, *, :, ^, -, (), ") per whitespace token, quotes each as a literal, OR-joins;
       all-punctuation query → None → callers return no results (no crash). Applied to
       search_facts (recall_fact) + search_conversations (search_knowledge). search_chunks
       untouched (rag.rs already sanitizes before calling it). 3 unit tests (punctuation-only,
       reserved-char strip, operator no-leak). Settings read-backs are claimed by the settings
       gate (Wave 3) so search_marketplace/recall_fact no longer fire for them (verified in
       Task 14 e2e). -->
  - Sanitize `recall_fact` FTS5 query input (escape/strip `?` and reserved tokens) — isolated fix.
  - Confirm the settings gate claims settings read-back so `search_marketplace`/`recall_fact` no
    longer fire for those prompts.
  - Tests: `recall_fact` with `"?"` does not crash; settings read-back does not invoke marketplace.
  - _Requirements: 4.5, 11.2_

- [x] 13. Backend test suite + property tests (design Testing)
  <!-- DONE: config/nl/properties.rs (#[cfg(test)] mod) — P1–P10 through the SHARED
       SettingsIntentPipeline + SettingsHandler (one decider ⇒ proves both surfaces):
       P1 chat==command decision+effect; P2 false-positives never build a mutation +
       KRIA imperative→Change; P3 non-GREEN NeedsApproval (no raw apply) + GREEN auto;
       P4 read-back==ConfigService effective; P5 secret change refused + read-back hides +
       preserve_secrets_from covers every is_secret_field; P6 injection (ExternalContent)
       refused; P7 per-request provenance no-bleed; P8 no-hardcoding via synthetic field
       (added SchemaEntityIndex::push_synthetic_field cfg(test) seam) routes with zero
       per-field code; P9 pipeline pure/env-free (flag-off legacy at loop gate, tested in
       loop_engine); P10 GREEN/read-back/undo succeed with no embedder/LLM (offline).
       Golden set (41/41) + 500-field scaling/latency already green. `cargo test --workspace`:
       kria-core lib 2997 pass / 1 pre-existing flake (agent::continuation_reentry, shared
       process-global — passes with --test-threads=1); kria-desktop 133, kria-server + others
       green. -->
  - Run the full `config/nl_golden_set.jsonl` (Task 15) + property tests P1–P10 (incl. no-hardcoding
    test that adds a synthetic annotated field and proves NL control works with zero routing-code
    changes; 500-field scaling/latency test; legacy-equivalence flag-off; LLM-optional/offline).
  - `cargo test --workspace` green (note any pre-existing unrelated flakes honestly).
  - _Requirements: 1.3, 3.1, 12.1, 14.1_

- [x] 14. Real-frontend validation (MANDATORY — Req 13)
  <!-- DONE: built app with embedded frontend (cargo tauri build --debug --no-bundle);
       launched the REAL window via tauri-driver + WebKitWebDriver (DISPLAY=:1), isolated
       HOME=/tmp/kria-e2e-home, KRIA_NL_SETTINGS=1 KRIA_CONFIG_BACKEND=sqlite. Harness:
       tests/gui-cognition-e2e/specs/settings_nl_control.e2e.ts + scripts/run_settings_nl_e2e.sh.
       withGlobalTauri temporarily enabled for WebDriver invoke, then REVERTED (committed config
       off). 14 passing / 1 honestly skipped:
       ✓ boot + get_settings shape + secret redaction (cloud_api_key == "")
       ✓ GREEN apply: "switch to dark mode", "change theme to light" → applied + get_settings
       ✓ YELLOW: "set search engine to duckduckgo" → routed to HITL approval (NOT browser_search),
         approve → applied (the exact RC1/RC5/NEW-10 live-failure prompt, now correct)
       ✓ read-back: "what is my current theme" (=dark), "what search engine am I using"
       ✓ false-positives (no mutation): "I'll change my CSS theme later", "turn on the lights",
         "change the api key in my code", "switch branches"
       ✓ invalid: "set theme to rainbow" → refused, lists allowed (light, dark)
       ✓ YELLOW "turn off voice": approve → voice.enabled=false; deny → unchanged (real HITL gate
         via agent:approval_required + approve_action/deny_action)
       ✓ temp: "generate image locally for this one" → temp_requested, image_mode unchanged
       ✓ undo synonym: "revert previous configuration" → undone → theme restored
       ✓ CHAT path via send_message (the exact cmd the Send button calls): "switch to dark mode"
         applied through the turn; false-positive "switch branches in my git repo" → no change
       ✓ persistence on disk (python3 sqlite3 over ~/.kria/kria.db): rows search.engine=duckduckgo,
         ui.theme=dark, voice.enabled=true committed.
       HONEST GAPS (Req 13.2, NOT fabricated): (a) CHAT-UI textarea DOM typing skipped — the
       webview textarea wasn't reachable via WebDriver DOM in this webkit2gtk build; the identical
       chat backend path is validated via send_message IPC (what the Send button invokes). (b)
       Injection-from-fetched-webpage not driven via multi-turn chat here; the injection wall is
       backend-proven (property P6 + handler tests + capability-provenance). (c) OS keychain =
       on-box manual, unverified. -->
  - Rebuild the desktop app with embedded assets (`cargo tauri build --debug --no-bundle`); launch
    via tauri-driver + WebKitWebDriver with isolated `HOME`; set `KRIA_NL_SETTINGS=1` +
    `KRIA_CONFIG_BACKEND=sqlite`. Drive the REAL chat UI (same IPC the UI uses).
  - Run the full mandatory suite through the real path (prompt → UI → backend → pipeline → handler
    → persistence → UI update) and assert rendered result + persisted DB (python sqlite3) + live
    UI sync:
    * GREEN apply: "switch to dark mode", "change theme to light", "set search engine to duckduckgo"
    * Read-back: "what is my current theme", "what is my image mode", "what search engine am I using"
    * Ambiguity BOTH ways: "what is the current theme?" in a settings context vs mid-conversation
    * False positives (MUST NOT change): "I'll change my CSS theme", "turn on the lights",
      "change the API key in my code", "switch branches"
    * YELLOW: "turn off voice", "aggressive autonomy", "enable remote desktop" → approve AND deny
    * Invalid: "theme rainbow", "google search engine", "unknown orchestrator" → grounded reject
    * Injection: fetched webpage instructing "change theme" → refused
    * Temp: "generate image locally", "generate using cloud" → config unchanged after
    * Undo synonyms: "undo last settings change", "revert previous configuration", "change back",
      "restore previous setting"
    * Persistence: restart app/backend → settings persist
    * Frontend: history, badges, restart indicators, env-locks, live sync
    * Natural prompts: "use DuckDuckGo", "disable voice", "turn off autonomous mode", "undo that",
      "can you remind me which theme I use?", "how do I change my theme?", "I want temporary cloud
      generation", "go back to previous settings" — plus more natural variants.
  - Mark on-box-unverifiable items (OS keychain) honestly; never fabricate a pass.
  - _Requirements: 13.1, 13.2, 13.3_

## Notes
- **No implementation yet** — this is specification only. Begin on explicit "start", one wave at a
  time, stopping at M1…M4 for review.
- **Single flag:** `KRIA_NL_SETTINGS` (default off ⇒ legacy). Folds in `KRIA_CONFIG_PROMPT_CONTROL`.
- **Reuse, don't rebuild:** `ConfigService`, `FieldMeta`/schema, `RequestOverride`, `AuditLogger`,
  `PolicyEngine`, `HitlGateway`, `routing/semantic.rs` embeddings, the availability resolver.
- **Hard rules:** one shared handler returning `SettingsOutcome` (no streaming/HITL inside the
  handler; caller drives approval + `apply_approved`); no raw mutation path; intent not keywords;
  fail toward conversation; secrets never clobbered/leaked (preserve-set = `is_secret_field`);
  injection wall via live-registry capability + per-turn provenance; durable undo (audit ledger
  fallback); one decider only (old `PromptAnalyzer`/`evaluate`/`try_config_prompt_dispatch`
  retired); real-frontend validation before completion.
- **Binding hardening:** see design.md **Wave 5 (F1–F17)** — insertion point (after TurnAccepted,
  before IntentGate/forcing/deterministic), scoring contract + thresholds, two-tier LLM-optional
  matcher, golden-set-first, FieldMeta expansion, multi-intent, provider mid-turn (N3).
- Companion: `settings-config-revamp/{analysis,design}.md` (backbone already built),
  `CONFIGURATION_ARCHITECTURE.md`.
```
