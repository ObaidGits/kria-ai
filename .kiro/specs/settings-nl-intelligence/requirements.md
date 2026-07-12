# Requirements — Settings Intelligence (production-grade NL settings OS)

Builds on `settings-nl-control` (shipped: single pipeline + handler + gate + backend + real-frontend
validation). This spec makes the UNDERSTANDING layer production-grade: true intent (not keywords),
evidence-based Configuration-vs-Conversation separation, universal schema-driven value extraction,
multi-turn conversational configuration (providers/models/keys), catalog/help/explain/read-back,
minimal explainable tool routing, and durable observability — with ABSOLUTELY NO prompt/keyword
hardcoding and natural scaling to thousands of settings/prompts/providers.

## Guiding principles (binding)
- **Intent, not keywords.** Every field/value/subject/intent decision is driven by schema-declared
  metadata + semantic similarity + conversation/memory evidence + confidence. No `contains("theme")`
  and no curated marker word-lists as the decision mechanism.
- **Evidence → confidence → destination.** Decisions combine Conversation Context + Recent Memory +
  Conversation State + Semantic Intent + Entity Resolution + (optional) Planner/LLM evidence into a
  calibrated confidence. Insufficient confidence ⇒ ONE clarifying question, never a guess.
- **Fail toward conversation.** A wrong answer is recoverable; a wrong mutation is not.
- **One safety gate.** Every mutation flows through the existing PolicyEngine + HITL + audit.
- **Zero interference.** Non-settings turns behave byte-identically; the settings subsystem never
  triggers unrelated tools and never lets unrelated tools trigger for a settings turn.
- **No fabricated results.** Completion requires real-frontend + adversarial validation.

## Requirements

### R1 — Evidence-based intent separation (the core)
1. Configuration-vs-Conversation SHALL be decided from combined evidence (semantic similarity to the
   schema corpus + conversation topic + recalled memory + subject/entity resolution + confidence),
   not from static keyword/marker lists.
2. WHEN a settings-like phrase ("current theme", "voice", "model") could plausibly refer to a
   conversation topic, a user artifact, a presentation/website/research subject, or KRIA config,
   the system SHALL use available evidence to pick the intended target; below the act threshold it
   SHALL answer from conversation or ask ONE clarifying question — never mutate/read KRIA config by
   assumption.
3. A calibrated, documented confidence function with tunable weights + thresholds SHALL be the
   single decision authority, and every decision SHALL emit a durable, inspectable trace.
4. Non-settings turns SHALL incur negligible added latency (cheap negative gate before any expensive
   stage) and SHALL be byte-identical to legacy when NL settings is disabled.

### R2 — Universal, schema-driven value extraction
1. The system SHALL extract and validate values for: booleans, integers, floats, enums (with
   spacing/underscore/case normalization), strings, URLs, file paths, durations, language codes,
   model IDs, endpoints, provider names, and lists — with NO per-field/per-prompt code.
2. Each field's type, bounds, allowed values, and value-aliases SHALL be declared in schema metadata
   (`FieldMeta`), and extraction/validation SHALL be driven entirely by that metadata + semantics.
3. Invalid/out-of-range/malformed values SHALL be rejected with a grounded message stating the
   type/range/allowed values (reask), on both local and cloud backends.
4. WHEN tier-A (lexical/type) cannot resolve a value, an optional LLM-constrained extraction
   (grammar local / structured cloud) SHALL fill it, schema-validated; absence of the LLM SHALL
   degrade gracefully (clarify), never fabricate.

### R3 — Full coverage
1. Every user-facing config field SHALL be annotated (risk, value-kind, bounds, allowed values,
   aliases, synonyms, label, description, prompt_changeable, temp_overridable). Unannotated/system
   fields stay fail-closed.
2. Adding a new field/provider/model SHALL enable NL control + read-back + help with ZERO
   routing-code changes (proven by a synthetic-field test).

### R4 — Multi-turn conversational configuration (slot-filling)
1. The system SHALL support configuring complex/multi-field targets (e.g. a cloud provider:
   name + api_key + model + endpoint + temperature + context window + routing + streaming) across
   MULTIPLE turns, accumulating known fields and asking ONLY for the missing ones.
2. It SHALL resolve which target/provider/model the user means from context, track what is known vs
   missing, never lose context across turns, and confirm before committing a multi-field change.
3. Secret fields (api keys) SHALL be captured and routed to the secure vault flow, never logged or
   echoed, on any turn.
4. The user SHALL be able to supply values one-at-a-time across turns or all at once; both SHALL
   converge to the same committed configuration.
5. A partial/abandoned configuration SHALL not persist; it SHALL be resumable within the session and
   expire safely.

### R5 — Catalog, Help, Explain, Read-back (answer from the system, never hallucinate)
1. "what settings can I configure?", "list all <group> settings", "show all providers/models",
   "what options exist for X?" SHALL be answered from schema metadata.
2. "how do I change X?", "explain X", "what does X do?", "what are valid values for X?", "why is X
   locked?" SHALL be answered from schema/metadata/docs WITHOUT invoking the LLM unnecessarily.
3. "what is my current X?", "which values require restart?", "which are env-locked?", "what changed
   today?/show recent changes" SHALL be answered from `ConfigService`/schema/audit — never guessed.
4. Read-back SHALL use human-friendly labels + formatted values (never raw `section.field`/`true`),
   report secrets as set/unset only, and note restart-required/env-locked status.

### R6 — Minimal, explainable tool routing (no interference)
1. For a settings turn, no unrelated tool (marketplace, recall, search, browser, installed-skills,
   GUI) SHALL be invoked.
2. For a general/knowledge/coding/OpenClaw/memory/research turn, the settings subsystem SHALL stay
   out of the way (no mutation, no read) and SHALL NOT add latency beyond the cheap gate.
3. The planner SHALL choose the minimum correct execution path; unnecessary tool invocations
   (e.g. `search_marketplace`/`recall_fact` on a general question) SHALL be eliminated via
   negative-evidence/confidence gating, and each routing decision SHALL be explainable via trace.

### R7 — Locking, concurrency, lifecycle, recovery
1. Concurrent settings changes (chat vs UI vs command) SHALL not lose updates or corrupt state;
   optimistic-concurrency or an explicit, documented policy SHALL be applied.
2. Pending approvals SHALL be bounded (GC/TTL); a never-answered HITL approval SHALL time out and
   release the turn with a clear message.
3. Every decision + mutation SHALL be durably traced/audited; misroutes SHALL be diagnosable from
   persisted traces.
4. All guarantees SHALL survive app/backend/desktop/LLM restart (persistence, undo-after-restart,
   read-back correctness), verified live.

### R8 — Validation (mandatory for completion)
1. Backend: golden set + property tests P1–P10 (carried from prior spec) + new coverage/value/
   slot-filling/catalog/help tests, all green (`cargo test --workspace`).
2. Real frontend: WebDriver/tauri-driver driving the actual chat UI + IPC through the human path,
   covering every category (theme/voice/search/image/autonomy/providers/models/keys/help/catalog/
   read-back/undo/temp/ambiguity both directions), with persistence + restart checks.
3. Adversarial: ambiguous, multilingual, Hinglish, typo'd, incomplete, chained, pronoun/reference,
   and memory-reference prompts SHALL be tested; the system SHALL never mutate on low confidence and
   never hallucinate a setting.
4. Anything unverifiable on-box (OS keychain) SHALL be marked honestly, never fabricated.
