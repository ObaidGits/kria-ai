# Session Handoff — settings-nl-intelligence

> Read `investigation.md` + `requirements.md` + `design.md` + `tasks.md` first. This spec makes the
> settings UNDERSTANDING layer production-grade on top of the shipped `settings-nl-control` backbone.
> Flag: `KRIA_NL_SETTINGS` is ON by default (opt out `=0`). Storage `KRIA_CONFIG_BACKEND=sqlite`.

## DONE (backend + LIVE validated)
- **Value engine** `config/nl/value.rs` — type INFERRED from real config serde shape
  (Bool/Int/Float/Enum/Str/List). Numeric/float/bool/enum (spacing/underscore/case/alias) extraction,
  ZERO per-field code. Wired into `SchemaEntityIndex::resolve_value` (old hardcoded hints removed).
- **Numeric bounds** `schema::field_bounds` + `validate_range` + `SchemaError::OutOfRange`, enforced
  in the handler (grounded range reject).
- **Coverage batch 1** (`schema::field_meta` arms, curated risk + bounds): agent (min_confidence_to_act,
  clarify_threshold, require_plan_for_complex_tasks, require_evidence_for_completion), memory
  (max_context_turns, max_facts, retrieval_top_k, decay_threshold), ui (window_width/height GREEN),
  safety (hitl_timeout_secs), image_generation (enabled), llm (temperature, context_window, max_tokens).
- **Catalog/Help/Explain/Recent** `config/nl/catalog.rs` + `SettingsDecision::Info(InfoQuery)` +
  `SettingsHandler::info` — answer-from-system, NO LLM. Pipeline `detect_info` = generic intent verbs
  + schema-driven field/group resolution, guarded vs user-artifact + content. Wired chat + command.
- **Humanized read-back / Applied** — friendly label + on/off + restart/env-lock notes
  (`catalog::label/render_value/status_note`).
- **No-interference hardening** (self-critique fixes): `generate` removed from settings imperatives;
  verb-less implicit path requires the FIELD referenced (`field_name_referenced`), not just a value
  word; bare-noun clarify gated to ≤4 words. Locked by **P15** property test (14 general prompts →
  zero settings engagement).
- **Observability** `config/nl/diagnostics.rs` — bounded ring + `tracing` line per decision; recorded
  from `run_settings_stage` (chat) and `config_prompt` (command) via `classify_traced`.
- Tests: `config::nl` 47 (golden 62 incl. numeric/float/toggle/catalog/help/adversarial), `config` 101,
  `loop_engine` 133, desktop compiles. LIVE e2e (tauri-driver): **19 pass / 1 skip**, persistence OK.
  Fixed a latent parallel env race in handler tests.

## DONE — Production audit (Task 4 / Task 13) — LARGE real-app campaign, LIVE
- Harness `tests/gui-cognition-e2e/specs/settings_audit.e2e.ts`: 55 negative + 30 positive + DB
  verification + real-HITL YELLOW approval, driven via `config_prompt` (the exact classifier/handler
  chat uses; fast, deterministic) against an isolated HOME copying the real config.
- First live run found **9 real bugs**: 8 negative false-positives (definitional/knowledge questions
  weakly resolving a field → Help/read-back) + 1 positive misroute ("I want dark mode" → clarify).
  All root-caused + fixed GENERICALLY (no hardcoding). Re-run GREEN: **negative 0/55, positive 0/30**,
  DB reflects applied changes, YELLOW applies after approval (4/4 tests pass).
- Fixes (all `config/nl/pipeline.rs`, schema/intent-driven):
  1. `DESIRE_MARKERS` → `is_imperative` (acts only with resolved field+value).
  2. `detect_info` help split: settings-directed (domain floor) vs definitional (act threshold).
  3. Content-authoring guard `CONTENT_LEAD_VERBS` (exempts "generate … using" image routing).
  4. Declarative-statement guard `STATEMENT_COPULAS` on the bare-noun clarify path.
  5. `contains_word_marker` — word-boundary self-ref match (fixes "my" ⊂ "autonomy").
  6. Provider definitional narrowed to "explain <provider>" only.
  Regression test `audit_false_positives_are_fixed` added (permanent).
- Verified: `config::nl` 72, `config` 129, `loop_engine` 120 green; desktop build clean; fmt clean;
  `withGlobalTauri` reverted OFF in committed `tauri.conf.json`.
- Toolchain: rustc 1.95.0-dev ICEs ANSI-rendering some unused-import warnings in the lib-TEST build →
  use `cargo test … --message-format=short` (source is fine; `cargo tauri build` unaffected).

## DESIGN EVOLUTION (living docs)
- Task 1 "FieldMeta v2 struct" → **companion functions** (`field_bounds`, catalog formatters, inferred
  value_kind) to avoid rewriting 22 arms. Same single-source module. tasks.md updated (3/6/7 done).

## DONE — Wave 2 (evidence-based intent) — backend + LIVE validated
- `config/nl/evidence.rs`: `TextEmbedder` + `MemoryEvidenceSource` seams (Option, graceful), `cosine`,
  `EvidenceWeights` (documented, golden-preserving defaults), `EvidenceDeps`.
- `ConversationContext::topic_affinity(text, Option<embedder>)` — semantic cosine to recent-turn
  centroid when embedder present, else lexical code-topic fallback (markers DEMOTED from authority).
- `pipeline.rs`: Configuration-vs-Conversation is a WEIGHTED evidence fusion (suppress weak/value-less/
  neutral guesses by conversation+memory topic; UserArtifact decisive-negative; KRIA-directed positive).
  Embedding/memory calls run ONLY inside the ambiguity guard (cheap domain gate → fast for normal chat).
  Trace enriched (conversation_topic/memory_topic/embeddings_used) + PERSISTED (`config/nl/diagnostics.rs`).
- Real embedder wired: `RoutingTextEmbedder` (FastEmbed via `routing::embed::embed_batch`, fails fast
  when cold) attached in `run_settings_stage`.
- Tests: `config::nl` 51 (golden 62 + P11 evidence-separation/memory/degradation + P15), `loop_engine`
  133, desktop clean. LIVE e2e 19 pass / 1 skip, no regression, no hang.
- NOTE: per-request image routing ("generate this using local AI") remains not_settings by default
  (no-interference); a future image-context evidence signal can safely re-enable it.

## DONE — Wave 4 (conversational multi-turn provider configuration) — backend + LIVE
- `llm/provider/config.rs`: `ProviderType::all()/synonyms()/resolve()` — schema-driven provider
  catalog (new provider = metadata only).
- `config/nl/flow.rs`: generalized slot-filling — `FlowStore` (per-session, TTL, isolated),
  `FlowEngine::{detects_start, step}`, `ProviderDraft`, `FlowOutcome`. Handles start/ask-only-missing/
  confirm/commit/correct(switch+forget)/defer-key/cancel/validate/resume. 17 backend scenario tests.
- `SettingsHandler::commit_provider` → `ConfigService::replace_all` (redacts key from store + vaults
  via SecretStore = secret-safe). Wired into chat (`run_settings_stage` + `SETTINGS_FLOW_STORE`) and
  command (`config_prompt` + `command_flow_store`).
- LIVE e2e (21 pass/1 skip): multi-turn OpenAI configure→confirm→activate (api_key redacted in
  persisted config), cancel-saves-nothing. Backend: `config::nl` 68, `loop_engine` 133, desktop clean.
- Residual (9b): provider RUNTIME apply (connection test / model swap) is the desktop ConfigChanged
  effect — not asserted live here; expand the live matrix (Gemini/Anthropic/Ollama, 40 scenarios).

## DONE — Task 9b (provider lifecycle + catalog/read-back, config-level, LIVE)
- Runtime path investigated: ConfigChanged executor = infallible effects only; provider swap =
  dedicated fallible `apply_provider_selection` (connection test + active-turn guard). `commit_provider`
  = the SAVE step (persist+vault+redact+activate), proven. Cloud connection-test/runtime-swap needs
  real creds → NOT on-box provable (honest limit).
- Added provider CATALOG + active read-back: `catalog::list_providers/active_provider` (schema-driven),
  `InfoQuery::Providers/ActiveProvider`, pipeline `detect_info` provider branch. Golden 73.
- LIVE (24 pass/1 skip): OpenAI multi-turn (key redacted), Ollama local switch active, provider
  catalog, active read-back, cancel. `config` 125, `loop_engine` 133, desktop clean.

## DONE — Wave 5 (evidence-based cross-domain injection gate)
- `agent/loop_engine/injection_gate.rs`: fused-evidence gate (semantic confidence + candidate
  competition + domain agreement via `tool_categories_for` + negative evidence (conversation_only) +
  floor + strong escape + cap). Generalized (category metadata only, no tool/prompt hardcoding).
  Explainable per-candidate trace (`target=tool_injection`). Wired at the injection site (top-5 →
  gate → accepted). 6 unit tests incl. "capital of India" drops marketplace/recall.
- No regression: loop 120, config 125, desktop clean, formatted. LIVE 26 e2e pass/1 skip incl. a
  tool_call-capture routing probe (knowledge/general prompt injects no marketplace/recall/skills).
- HONEST LIMIT (10b): positive live tool-selection needs a running local LLM (unavailable here;
  captured tools=[] proves it) → 60-prompt positive-routing campaign not on-box verifiable. Gate
  logic proven deterministically; no-interference proven live.

## DONE — Task 11 (locking + observability hardening)
- Approval lifecycle: `PendingChange.created_ms`; `gc_pending()` TTL(10m)+cap(128, evict-oldest)
  before each insert; `resolve()` releases pending on Deny AND Timeout (fixes leak). Tests added.
- Optimistic concurrency: `ConfigService::replace_all_checked(cfg, source, expected_version)` →
  `StaleVersion`; `commit_provider` reads version→builds→commits→retries once on stale.
  `replace_all` delegates None (no behaviour change).
- Durable diagnostics: `config/nl/diagnostics.rs` appends each decision trace to a bounded JSONL
  (`set_persist_path`, 2MB cap); wired at desktop startup → `logs_dir/settings_intent_routing.jsonl`.
- Verified: config 128, config::nl 71, loop+injection 120, kria-desktop 133; desktop clean; fmt clean.

## DONE — Task 10b (real routing campaign, live with the configured Qwen3-VL-4B)
- Harness: `scripts/run_routing_campaign.sh` (isolated HOME copies real config.toml + KRIA_MODELS_DIR
  → real models, NON-destructive; orchestrator boots the configured llama backend) +
  `tests/gui-cognition-e2e/specs/routing_campaign.e2e.ts` (31 prompts via send_message + config_prompt,
  captures agent:tool_call).
- RESULT (llmLive=true): interference=0/20; all knowledge/general/memory/math/translation chat prompts
  fired ZERO unrelated tools; 11 settings prompts routed correctly; provider read-back returned the
  REAL "llama.cpp … model qwen3-vl-4b"; catalog showed llama.cpp ACTIVE + Ollama configured.
- Honest scope: no-interference + settings/provider routing proven live. Positive tool-EXECUTION
  (file/vision/openclaw firing the right tool) not covered here → 10c (needs real fixtures+permissions).
- To re-run: set `withGlobalTauri:true`, `cargo tauri build --debug --no-bundle`, `bash
  scripts/run_routing_campaign.sh`, then revert the flag.

## (superseded) NEXT — Wave 5 root cause (now implemented)
- **Root cause (D1):** `loop_engine/mod.rs` "Cross-domain semantic tool injection" (~line 7800)
  injects top-3 FastEmbed tool matches at `confidence >= 0.35` REGARDLESS of domain. Broad-description
  tools (`search_marketplace`, `recall_fact`) match many queries at low confidence → injected →
  LLM calls them (e.g. "capital of India"). Existing guard `suppress_all_tool_routing =
  fast_path || is_satisfied` only zeroes tools for IntentGate conversational fast-path.
- **Plan (generalized, no hardcoding):** add a negative-evidence/confidence gate to the injection —
  raise/relativize the floor (margin over next candidate, or per-tool metadata "specificity"),
  and/or require the injected tool's domain to agree with the routed modality/evidence. Emit the
  injection decision to the diagnostics trace (explainable). Calibrate against a 40–60 prompt live
  matrix (settings/openclaw/marketplace/knowledge/memory/coding/files/providers/browser/search/
  images/general/ambiguous/Hinglish). HIGH blast radius — 133 loop tests + live matrix must stay green.
- **Also:** provider removal-by-conversation + "rotate/replace key" flows (config-lifecycle follow-ons). `config/nl/flow.rs` per-session ConfigFlowState;
  provider/model/key config ("connect OpenAI, key is…, use GPT-5, endpoint…, temp 0.2"); providers
  are `Vec<ProviderConfig>`; secrets→vault (never logged); confirm before commit; resumable/TTL.
- **Wave 5 — minimal routing / no-interference at PLANNER level (Task 10):** negative-evidence gate so
  `search_marketplace`/`recall_fact`/browser don't fire on general questions (the "capital of India"
  over-fire is a ROUTING issue outside the settings module). + locking (expected_version threading)
  + pending-approval TTL/GC + HITL timeout release (Task 11 remainder).
- **Coverage batch 2:** providers/voice extras/orchestrator/planner toggles (careful risk tiering).
- **Wave 6:** expand live + adversarial matrix (multilingual/Hinglish/typo/chained/pronoun/memory-ref)
  + restart-of-backend/LLM.

## e2e run
Set `withGlobalTauri:true` in `crates/kria-desktop/tauri.conf.json`, `cargo tauri build --debug
--no-bundle`, `bash scripts/run_settings_nl_e2e.sh`, then REVERT the flag (kept off in committed config).

## Hard rules
Intent not keywords (schema + semantic + evidence; marker lists are fallback only); no raw mutation;
fail toward conversation; secrets never logged/echoed; minimal explainable routing; real-frontend +
adversarial validation before declaring production-grade.
