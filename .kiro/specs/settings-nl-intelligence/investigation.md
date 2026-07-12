# Investigation — Settings Intelligence (production hardening, fresh pass)

> This is a full re-investigation (NOT assuming prior conclusions complete). It audits the entire
> path User→Frontend→Chat→Planner→Intent→Context→Memory→ToolSelection→SettingsDetection→
> Validation→Permission→Execution→Persistence→UI→Restart→History→Undo→Read-back→Audit→Recovery.
> Every claim is code-verified with file:symbol evidence. Prior findings (RC*/NEW*/B*/C*/L*/G*)
> are re-evaluated at the end.

## Verdict
The **storage/safety/persistence backbone is production-grade** (ConfigService single-writer lock +
optimistic version + atomic batch + durable audit + secret vault + HITL + durable undo). The
**understanding layer is not**: it is a lexical (keyword-list) classifier with narrow value
extraction and no memory/planner/semantic evidence — the exact opposite of the "intent, not
keywords" requirement. Below are the issues, grouped, newest first.

---

## A. Intent understanding is lexical, not intent-based (ARCHITECTURAL)

- **A1 (blocker for the core requirement).** `config/nl/conversation.rs` resolves
  Configuration-vs-Conversation with **hardcoded marker lists** (`KRIA_MARKERS`,
  `USER_ARTIFACT_MARKERS`, `CODE_TOPIC_TERMS`). This is disguised keyword matching. "What is the
  current theme?" during a slide-deck / website / research discussion has no code-terms → the
  classifier can score it as a KRIA read-back and answer from `ConfigService` — a wrong-target
  answer. The spec's own promised model (Conversation Context + Memory + Semantic Intent + Planner
  Reasoning + Entity + Evidence → Confidence) is **not implemented**.
- **A2.** No **memory** signal: `memory` (facts/RAG) is never consulted to decide whether a
  settings-like phrase refers to an ongoing topic. `ConversationContext` only sees the last few raw
  message strings (loop `build_settings_conversation_context`), not embeddings or recalled facts.
- **A3.** No **planner** signal: `run_settings_stage` runs BEFORE the planner and decides in
  isolation; there is no "planner reasoning" input to the confidence as the requirement describes.
- **A4.** No **semantic** tier: `SchemaEntityIndex` is tier-A lexical only (phrase-substring + token
  overlap). Design promised tier-B embeddings; `SettingsIntentPipeline::new` takes no embedder/LLM.
- **A5.** The documented weighted-confidence contract (design F8: domain .35/entity .30/intent
  .20/conv .15 + `IntentThresholds`) is **not implemented** — `pipeline.rs` uses ad-hoc per-branch
  additions and `SettingsIntentTrace.confidence` is set inconsistently. There is no real
  "domain gate" first stage; entity resolution is the de-facto gate → every turn pays it.

## B. Value extraction is narrow (blocks "set any variable")

- **B1.** `entity_index.rs::resolve_value` returns a value ONLY when `meta.valid_values.is_some()`.
  Every free/numeric field (`ui.font_scale`, `agent.max_tool_rounds`, `orchestrator.cuda_reserve_mb`,
  `image_generation.tier_override`, `voice.language`, `search.searxng_url`, ports, URLs, paths) →
  always `None` → `Change{value:None}` → `Clarify`. Numbers/URLs/paths/durations are unsettable.
- **B2.** Enum match is substring `norm.contains(v)`, so underscored variants (`push_to_talk`,
  `local_with_cloud_fallback`) never match natural spacing.
- **B3.** No value aliases: "French"→`fr`, "English"→`en`, "on/off"→bool for non-enum, "gpt5"→
  `gpt-5`. `raw_value_after_to` yields the raw word → handler rejects valid intent as invalid.
- **B4.** No **type/`ValueKind`** in `FieldMeta` (no int/float/bool/enum/string/url/path/duration/
  list), so no range or format validation (`font_scale=999` would pass; `port=70000` would pass
  serde as u16? no — overflow fails with an opaque error).
- **B5.** The NL handler passes values straight to `ConfigService.patch` with **no coercion**;
  only the `config_patch` tool has `coerce_value`. A string "8" for a `usize` field → serde error →
  generic "Failed to apply".

## C. Missing capabilities the requirement demands

- **C1.** **Multi-turn slot-filling** for complex/provider config is entirely absent. `providers`
  is a `Vec<ProviderConfig>` (nested array); `is_secret_field` blocks api keys via the generic
  path. "Connect my OpenAI account… key is…, use GPT-5, endpoint…, temperature 0.2" has no
  conversational state machine, no "which provider", no missing-field tracking, no partial-config
  accumulation, no "ask only for what's missing".
- **C2.** **Catalog / discovery**: "what settings can I configure?", "list all voice settings",
  "show all providers", "what options exist for image generation?" — no intent; `full_schema_json()`
  exists but chat cannot reach it.
- **C3.** **Help / Explain**: "how do I change theme?", "explain emergency mode", "what does this
  do?", "what are valid values?", "why is this locked?" — routed to the LLM (which the live log
  shows can be DOWN) instead of answered from schema metadata. `FieldMeta` has no `label`/
  `description`/help text.
- **C4.** **Read-back breadth**: "which values require restart?", "which are env-locked?", "what
  changed today?", "show recent changes" — only single-field read-back exists; the audit history +
  `full_schema_json` restart/env-lock flags are not exposed to chat.
- **C5.** **Coverage**: only ~22 of 50+ `all_fields()` are `prompt_changeable`; the rest are
  fail-closed → cannot be set by chat at all.

## D. Tool routing / interference (planner-level)

- **D1.** `search_marketplace` + `recall_fact` fire on general questions (live log: "capital of
  India"). Root cause is the semantic router surfacing knowledge tools + LLM tool choice, NOT the
  settings module. The planner has no negative-evidence / minimum-path gate → unnecessary tools run
  even when the final answer is correct. Explainable, minimal routing is missing.
- **D2.** The settings stage claims settings turns (good), but there is no symmetric guarantee that
  a NON-settings turn is cheap and that the classifier cannot pre-empt the planner on a
  false-positive (A1/A5 make this brittle).

## E. Concurrency / locking / lifecycle

- **E1.** NL handler always calls `patch(expected_version=None)` → last-writer-wins vs a concurrent
  UI edit (no optimistic-concurrency use, though the core supports it).
- **E2.** `SettingsHandler.pending` is only cleared on `apply_approved`; `resolve()` deny/timeout
  does not remove it (leak in the documented handle()+external-apply pattern; benign today because
  the handler is per-turn).
- **E3.** `ChatSettingsApprovalDriver` blocks the turn awaiting HITL; a never-answered approval
  holds the turn until HITL timeout (need sane timeout + cancel).
- **E4.** `SettingsIntentTrace` is computed but never persisted → prod misroutes are undiagnosable.
- **E5.** Read-back uses `read_field` over the in-memory `inner` snapshot, not `resolve()`; a
  provider-sync/env change not mirrored into `inner` could read stale (minor).

## F. Prior findings re-evaluation (RC/NEW/B/C/L/G)
- **Fixed & verified:** RC1/RC5 (raw dispatch removed; single gate), RC2/NEW-2 (read_field wired),
  RC3 (undo synonyms), RC4/NEW-13 (one decider; command is a thin caller), RC7 (fts5 sanitized),
  NEW-1/4/8 (secret preserve), NEW-3 (capability provenance), NEW-5 (per-turn taint),
  NEW-9/10/11 (settings stage first; no browser regex hijack), NEW-12 (ConversationContext).
- **Still open (carried into this spec):** B1–B8, C1–C9, L1–L6, G1–G9 from the prior list, plus the
  NEW architectural items A1–A5, C1–C5, D1–D2, E1–E5 above.
- **Obsolete:** the old `try_config_prompt_dispatch`/`build_turn_override`/`PromptAnalyzer`/
  `evaluate` deciders (removed in Wave-4 Task 16) — no longer a source of divergence.

## G. No-hardcoding audit (mandate compliance)
- Current `conversation.rs` marker lists and `entity_index.rs` value hints
  (`night→dark`, `local→local_only`) are **borderline hardcoding**. The target architecture must
  move ALL field/value/subject knowledge into **schema-declared metadata** (synonyms, aliases,
  value-kinds, subject-scope) + semantic similarity, so adding a setting/provider/model needs zero
  routing-code edits.
