# Implementation Plan — Settings Intelligence

Waves build the intelligence layer on the shipped backbone. Each task: `cargo check`/`clippy`/`fmt`
clean, tests written+green, flag-off parity, NO hardcoding (schema-driven only), real-frontend
validation before milestone completion. Read `investigation.md` + `requirements.md` + `design.md`.

```json
{
  "waves": [
    { "wave": 1, "tasks": [1, 2, 3], "description": "FieldMeta v2 (ValueKind/bounds/aliases/label/description) + universal ValueEngine + full coverage" },
    { "wave": 2, "tasks": [4, 5], "description": "Evidence-based intent (semantic conv/memory + weighted fusion + trace) replacing lexical marker authority" },
    { "wave": 3, "tasks": [6, 7], "description": "Catalog/Help/Explain/Read-all intents (answer-from-system, no LLM)" },
    { "wave": 4, "tasks": [8, 9], "description": "Multi-turn SlotFiller for provider/model/key config" },
    { "wave": 5, "tasks": [10, 11], "description": "Minimal routing / no-interference + locking/observability hardening" },
    { "wave": 6, "tasks": [12, 13], "description": "Backend suite P1–P15 + real-frontend + adversarial validation" }
  ]
}
```

### Milestones
- **M1 = Wave 1** — "set any variable" works (numbers/enums/urls/aliases), full coverage. Shippable.
- **M2 = Waves 2–3** — true intent separation + catalog/help/read-back. Shippable.
- **M3 = Waves 4–5** — conversational provider config + no-interference + locking.
- **M4 = Wave 6** — full backend + live + adversarial validation. Production.

## Tasks

- [~] 1. Field metadata v2 (DESIGN EVOLVED: companion functions, not struct-field bloat)
  <!-- PARTIAL/DECISION: rather than add 7 fields to FieldMeta and rewrite 22+ arms (churn +
       merge risk), auxiliary metadata is exposed as companion functions in the SAME schema
       module (single source of truth): value_kind is INFERRED (config/nl/value.rs, no annotation),
       numeric bounds via schema::field_bounds + schema::validate_range (+ SchemaError::OutOfRange),
       human labels + friendly values + restart/env-lock notes via config/nl/catalog.rs
       (label/render_value/status_note). description/help are generated from existing metadata.
       REMAINING: optional per-field description/aliases overrides if generic derivation proves
       insufficient. investigation/design updated to reflect this. -->
- [x] 3. Full field coverage annotation (conservative, correctly-tiered batch)
  <!-- DONE (batch 1): added prompt_changeable arms with curated risk + bounds for agent
       (min_confidence_to_act, clarify_threshold, require_plan_for_complex_tasks,
       require_evidence_for_completion), memory (max_context_turns, max_facts, retrieval_top_k,
       decay_threshold), ui (window_width/height GREEN), safety (hitl_timeout_secs), image_generation
       (enabled), llm (temperature, context_window, max_tokens). All YELLOW (gated) except UI window
       size (GREEN). Range-validated via field_bounds. Tests: expanded_coverage_fields_are_prompt_
       changeable_and_gated + numeric_range_is_validated. MORE sections (providers/voice extras/
       orchestrator/planner toggles) remain — deferred to a careful follow-up batch. -->

- [~] 2. Universal `ValueEngine` (`config/nl/value.rs`)  — DOWN PAYMENT LANDED
  <!-- PARTIAL (this session): config/nl/value.rs created. Type is INFERRED from the real
       KriaConfig serde shape (value_kind: Bool/Int/Float/Enum/Str/List) → generalizes with ZERO
       per-field code. extract() handles: enums (verbatim + spacing/underscore normalization +
       distinctive-token + transitional alias data), booleans (on/off/enable/disable/yes/no),
       integers, floats, and raw-word grounding for invalid enums. Wired into
       SchemaEntityIndex::resolve_value (removed the old hardcoded night/local/cloud/light hint
       chain). 6 unit tests green incl. numeric extraction (fixes B1: "set max tool rounds to 8"→8)
       and enum spacing (fixes B2: "push to talk"→push_to_talk). Also fixed a latent parallel env
       race in handler tests (ENV_LOCK guard on autonomy_profile tests).
       REMAINING for full Task 2: range bounds validation (needs FieldMeta v2 min/max — Task 1),
       URL/path/duration/langcode format validation + aliases, list parsing, LLM-constrained
       fallback trait, and moving VALUE_ALIASES into FieldMeta.aliases. -->
- [ ] 2b. Complete `ValueEngine` (bounds/url/path/duration/list/LLM-fallback + aliases→FieldMeta)
  - Type-aware extract+validate for bool/int/float/enum/str/url/path/duration/langcode/list, driven
    ENTIRELY by `FieldMeta` (no per-field code); alias normalization; coercion; grounded errors.
  - Wire into `SchemaEntityIndex::resolve_value` (replace the hinted hardcoding) + the handler.
  - Optional LLM-constrained fallback behind a trait (no-op when absent).
  - Tests: P12 matrix (numbers/floats/bools/enum spacing/underscore/aliases/url/path/lang/list);
    invalid→grounded reject; offline (no LLM) still passes tier-A.
  - _R2.1, R2.3, R2.4_

- [ ] 3. Full field coverage annotation
  - Annotate every user-facing field across all sections (risk/value_kind/bounds/allowed/aliases/
    label/description/prompt_changeable/temp_overridable). System/derived stay fail-closed.
  - Tests: coverage assertion (intended user-settings are annotated); synthetic-field no-hardcoding
    test (add field via metadata only → set/read works).
  - _R3.1, R3.2_

- [x] 4. Evidence collectors (`config/nl/evidence.rs`): semantic conversation + memory + subject
  <!-- DONE: config/nl/evidence.rs — TextEmbedder + MemoryEvidenceSource seams (Option, graceful
       degradation), cosine, EvidenceWeights (documented, golden-preserving defaults), EvidenceDeps.
       ConversationContext::topic_affinity(text, Option<embedder>) = semantic cosine to recent-turn
       centroid when an embedder is present, else the lexical code-topic signal (fallback, demoted
       from authority). Marker lists are now ONE fallback evidence source, not the decider.
       Wired a REAL FastEmbed embedder (RoutingTextEmbedder over routing::embed::embed_batch, which
       fails fast when the model is cold) into the chat settings stage. Tests: P11 paired evidence
       separation ("what is the current theme" → ReadBack with no topic, → NotSettings when the
       conversation is about a presentation theme via the embedder), P11 memory participation,
       P11 graceful degradation. -->
- [x] 5. Weighted confidence fusion + durable trace (`pipeline.rs` rework)
  <!-- DONE: the Configuration-vs-Conversation separation is now a weighted evidence fusion —
       neutral+value-less+weak guesses are SUPPRESSED proportionally by (conversation_penalty *
       conversation_topic + memory_penalty * memory_topic); UserArtifact subject is decisive
       negative evidence; KRIA-directed is positive (tunable). The expensive embedding/memory calls
       run ONLY inside the ambiguity guard (cheap domain gate — entity best=None exits before any
       embedding), preserving latency. SettingsIntentTrace enriched (conversation_topic/memory_topic/
       embeddings_used) and PERSISTED via config/nl/diagnostics.rs (bounded ring + per-decision
       tracing line). Golden 62 + P11/P15 green; defaults reproduce prior behaviour exactly
       (entity_conf ×0.6 == 1 − 0.4·topic). -->

- [x] 6. Catalog + Read-all intents (`config/nl/catalog.rs`)
  <!-- DONE: config/nl/catalog.rs (list_configurable/explain/help_change/recent_changes) +
       SettingsDecision::Info(InfoQuery{Catalog|Explain|Help|RecentChanges}) + SettingsHandler::info
       (answers from schema + live config + audit, NO LLM). Pipeline detect_info uses GENERIC intent
       verbs (no field keywords) + schema-driven field/group resolution; guarded against user-artifact
       subject + content requests. Wired into the loop + config_prompt command. Golden: 7 info cases. -->
- [x] 7. Help + Explain intents
  <!-- DONE: folded into Task 6's InfoQuery::Help/Explain — schema-generated help_change/explain
       (valid values, risk, restart, env-lock, why-locked) with NO unnecessary LLM. -->
- [ ] 6b. Provider/model catalog ("show all providers", "which model am I using") — needs Wave 4 provider modeling.

- [x] 8. `SlotFiller` core (`config/nl/flow.rs`) — multi-turn accumulation + missing-field asking
  <!-- DONE: config/nl/flow.rs — Slot, ProviderDraft, FlowStatus, ConfigFlowState, FlowStore
       (per-session, Mutex, 15-min TTL, isolated), FlowEngine (detects_start, step). Merges values
       each turn; asks ONLY missing (required derived from ProviderType metadata); confirm→commit;
       corrections (switch provider w/ cue, forget slot); defer key; cancel; validate (key len,
       URL, temp range, tokens). 17 backend scenario tests incl. P13 convergence (one-at-a-time ==
       all-at-once), cancel-no-persist, per-session isolation, generalization (OpenRouter unseen),
       key-never-echoed. -->
- [x] 9. Provider/model/key configuration via SlotFiller
  <!-- DONE: ProviderType::all()/synonyms()/resolve() = schema-driven provider catalog (metadata
       only — new provider = one arm). SettingsHandler::commit_provider builds ProviderConfig from
       the draft and persists via ConfigService::replace_all, which REDACTS the api_key from the
       config store AND vaults it via SecretStore (secret-safe, verified). Wired into the chat loop
       (run_settings_stage, SETTINGS_FLOW_STORE) and the command surface (config_prompt). Runtime
       apply (connection test / model swap) is the desktop ConfigChanged effect (design C5), not the
       core commit. LIVE e2e: multi-turn OpenAI converge+activate (api_key redacted in persisted
       config) + cancel-saves-nothing — both pass through the real app IPC. -->
- [x] 9b. Provider lifecycle completion + provider catalog/read-back (config-level, live-proven)
  <!-- DONE: investigated the full runtime path — ConfigChanged effect executor applies only
       INFALLIBLE effects; provider/model swap is the dedicated FALLIBLE apply path
       (apply_provider_selection) with connection test + active-turn guard. commit_provider
       correctly persists+vaults+redacts+activates config (the SAVE step); runtime swap + cloud
       connection validation are the dedicated path and need real services/creds (NOT on-box
       provable — honest limitation, like OS keychain). Added provider catalog + active read-back:
       catalog::list_providers / active_provider (schema-driven from ProviderType), InfoQuery::
       Providers/ActiveProvider, pipeline detect_info provider branch. LIVE (24 pass/1 skip):
       OpenAI multi-turn configure→activate (key redacted), Ollama local (no key) switch active,
       provider catalog, active-provider read-back, cancel-saves-nothing. Golden 73.
       RESIDUAL (honest, not on-box provable): cloud connection-test/runtime-swap with real creds;
       provider removal-by-conversation + "rotate key" are config-lifecycle follow-ons. -->

- [x] 10. Minimal routing / no-interference (planner) — Wave 5 evidence-based injection gate
  <!-- DONE: agent/loop_engine/injection_gate.rs — replaces the flat "top-K, confidence>=0.35,
       domain-blind" cross-domain tool injection with an EVIDENCE-FUSED gate: semantic confidence +
       candidate competition (distance-from-best) + domain agreement (candidate category vs the
       categories the semantic router chose, via tool_categories_for) + negative evidence
       (conversation_only suppresses) + absolute floor + strong-match escape hatch + max cap.
       Generalized (no tool names/prompt patterns; category metadata only). Explainable: per-candidate
       InjectionScore trace logged (target=tool_injection). Wired at the injection site (fetch top-5,
       gate, map accepted → SemanticInjection). 6 unit tests incl. the exact interference case
       ("capital of India" → only web; cross-domain noise dropped; strong cross-domain passes;
       conversation suppression; cap; floor). No regression: loop 120, config 125, desktop clean.
       LIVE (26 e2e pass/1 skip): no-regression + no-hang + a routing-interference probe
       (agent:tool_call capture) asserting a knowledge/general prompt injects NO
       marketplace/recall/skills tools.
       HONEST LIMIT: positive live tool-selection (searxng actually fires for a knowledge prompt)
       needs a RUNNING local LLM — unavailable in this e2e env (captured tools=[] confirms), so the
       full 60-prompt positive-routing campaign is not on-box verifiable; the gate logic is proven
       deterministically by unit tests + no-interference proven live. -->
- [x] 10b. Real routing campaign through the actual app + configured local LLM (Qwen3-VL-4B)
  <!-- DONE (live): scripts/run_routing_campaign.sh runs an isolated HOME that COPIES the real
       config.toml + points KRIA_MODELS_DIR at the real model dir (non-destructive), so the
       orchestrator boots the user's configured Qwen3-VL-4B llama backend. tests/gui-cognition-e2e/
       specs/routing_campaign.e2e.ts drives 31 real prompts via the same IPC the UI uses
       (send_message + config_prompt), capturing agent:tool_call.
       RESULT (llmLive=true): 31/31 prompts, interference=0/20 — every knowledge/general/memory/
       math/translation chat prompt fired ZERO unrelated tools (no marketplace/recall/browser);
       all 11 settings prompts routed correctly (apply/read-back/catalog/help/invalid-reject/undo/
       false-positive→not_a_change); provider read-back returned the real "llama.cpp … model
       qwen3-vl-4b" and the catalog showed llama.cpp ACTIVE + Ollama configured.
       HONEST SCOPE: this proves NO-INTERFERENCE + settings/provider routing live with the real
       model. It does NOT include positive tool-EXECUTION prompts (file read / vision / openclaw
       install actually firing their tool) — those need real fixtures + permission setup; knowledge
       prompts were answered directly by the model (correct minimal path, no tool needed). Minor
       observation: captured assistant text was a uniform length across chat prompts (token-capture
       artifact / brief model replies) — does not affect the routing (tool-firing) evidence. -->
- [ ] 10c. (optional) Positive tool-EXECUTION live cases (file/vision/openclaw) with real fixtures + permissions.

- [x] 11. Locking + observability hardening — DONE (backend-proven)
  <!-- DONE:
   • Approval lifecycle: PendingChange gains created_ms; gc_pending() TTL (10m) + MAX_PENDING (128,
     evict-oldest) called before each insert; resolve() now RELEASES the pending entry on Deny AND
     Timeout (fixes the E2/B7 leak). Tests: pending_is_released_on_deny_and_timeout,
     stale_pending_is_garbage_collected.
   • Optimistic concurrency: ConfigService::replace_all_checked(cfg, source, expected_version) →
     StaleVersion on conflict; commit_provider reads version → builds → commits with expectation →
     retries once on stale (lost-update prevention). replace_all delegates with None (no behaviour
     change). 
   • Durable diagnostics: config/nl/diagnostics.rs persists each decision trace to a bounded JSONL
     file (set_persist_path, 2MB cap w/ truncate); wired at desktop startup to
     logs_dir/settings_intent_routing.jsonl. Test: persists_traces_to_jsonl_when_path_set. In-memory
     ring retained for fast introspection.
   • HITL timeout release covered by the deny/timeout cleanup above.
   Verified: config 128, config::nl 71, loop+injection 120, kria-desktop 133 green; desktop compiles;
   formatted. -->
  - _R7.1, R7.2, R7.3_

- [ ] 12. Backend suite P1–P15
  - Golden + P1–P10 (carried) + P11–P15; coverage/value/slot-fill/catalog/help/no-interference;
    `cargo test --workspace` green.
  - _R8.1_

- [~] 13. Real-frontend + adversarial validation (MANDATORY)
  <!-- SUBSTANTIALLY DONE (production audit): tests/gui-cognition-e2e/specs/settings_audit.e2e.ts —
       55 negative + 30 positive + DB verification + real-HITL YELLOW approval, all via config_prompt
       (the classifier/handler chat uses) against an isolated HOME with the real config. First run
       found 9 real bugs (8 negative false-positives + 1 positive misroute); all root-caused + fixed
       GENERICALLY in config/nl/pipeline.rs (desire phrasing→imperative; help settings-directed vs
       definitional tiers; content-authoring guard; declarative-statement guard; word-boundary
       self-ref match fixing "my"⊂"autonomy"; provider definitional narrowed to "explain <provider>").
       Re-run GREEN: negative 0/55, positive 0/30, DB reflects applied changes, YELLOW applies after
       approval (4/4 passing). Permanent regression test audit_false_positives_are_fixed added.
       Backend: config::nl 72, config 129, loop_engine 120 green; desktop build clean; withGlobalTauri
       reverted OFF. REMAINING (honest): full adversarial matrix — multilingual/Hinglish/typo/chained/
       pronoun/memory-ref + app/backend/LLM RESTART persistence — not yet exhaustively driven live. -->
  - WebDriver/tauri-driver across every category incl. providers/keys/models/help/catalog/read-all/
    ambiguity both ways + persistence + restart (app/backend/desktop/LLM). Adversarial:
    ambiguous/multilingual/Hinglish/typo/incomplete/chained/pronoun/memory-ref. Mark keychain honest.
  - _R8.2, R8.3, R8.4_

## Notes
- Reuse everything shipped; do not rebuild backbone. One decider only (the pipeline).
- Hard rules: intent-not-keywords (schema+semantic+evidence); no raw mutation; fail toward
  conversation; secrets never logged/echoed; minimal explainable routing; real-frontend +
  adversarial validation before declaring production-grade.
- Begin on explicit "start", one wave at a time, stopping at M1…M4 for review.
