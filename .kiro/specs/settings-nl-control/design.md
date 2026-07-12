# Design — Natural-Language Settings Control (Production Architecture)

This design is presented in the required iterative form: **Wave 1** (current-state investigation +
Plan V1), **Wave 2** (self-critique), **Wave 3** (stress test), **Wave 4** (final architecture).
Waves 1–3 record the reasoning; Wave 4 is the implementation-ready design that `tasks.md` builds.

> Sources of truth: this repo's code (verified below with file:line), the prior spec
> `settings-config-revamp/{analysis,design}.md`, and `CONFIGURATION_ARCHITECTURE.md`.

## Overview

KRIA's storage/safety backbone for settings is built; the *understanding + routing* layer is not.
This design adds a single, schema-driven **Settings Intent pipeline** that runs as the first
classification stage of a chat turn, strictly separates **Configuration Intent** from
**Conversation Intent** (context + entity resolution + confidence, no keywords), and routes every
settings operation (change/read-back/undo/temp) through **one shared `SettingsHandler`** that both
chat and the desktop command surface call. All mutations pass the single PolicyEngine + HITL gate;
none run raw. The full canonical architecture is in **Wave 4** (the `## Architecture` and
`## Components and Interfaces` sections below); Waves 1–3 record the required iterative reasoning
(investigation → self-critique → stress test) that produced it.

---

## WAVE 1 — Current-state investigation (code-verified)

### 1.1 How a chat turn actually routes tools
Entry `AgentLoop::run` → `run_with_profile` (`agent/loop_engine/mod.rs:5127/5139`). Per turn:
- `last_user_text`/`previous_user_text` from the `messages` vec (5147–5158).
- **Pre-ReAct forcing layers:** `LiveFactClassifier` forces `#tool:searxng_search` (5393–5397);
  `GuiIntentClassifier` forces `#tool:browser_search` (5411–5420).
- **Deterministic pre-dispatch site #1** (5741) runs BEFORE GUI/ReAct;
  **site #2** (6796) on the non-GUI ReAct fast-path. Both call
  `try_deterministic_dispatch_with_profile` (326) → `try_deterministic_dispatch_with_context`
  (437). Inside: our config gate `try_config_prompt_dispatch` (367) runs FIRST (flag-gated), then
  a chain of regex branches incl. a **browser-search regex (~617)** and a **"what is my/current"
  system-info branch (~470)**.
- Both deterministic sites execute the chosen handler **RAW**: `handler.execute_with_context(...)`
  under a 30s timeout (5815/6873). **No PolicyEngine, no HITL.** Only guard =
  `execution_profile.allows_tool_name` (5762/6817).
- **ReAct path** (no deterministic match): semantic `Router::route_with_context`
  (`routing/mod.rs:105`, embedding centroids) + `TurnGate.direct_tool_hint` +
  `tool_index.top_k_by_text` (FastEmbed) build a narrowed schema set
  (`select_routed_tool_schemas`, ~7609); the **LLM makes the final tool call** (parsed 7943/7974).
- **The single real safety gate** is in the ReAct path only: `policy_engine
  .evaluate_with_modality_hint` (~8878) → BLACK block (8908) / RED `HitlGateway
  .request_approval_with_id` (8977–8999).
- `RoutingContext` (`routing/context.rs:216`) is handed to the router via `turn_gate.context()`
  (7518), but `TurnGate` is a **shared `Arc`** on `AgentLoop` (4632) with no reachable mutators →
  the routing context is **effectively default/empty**; real conversation state = the `messages`
  vec seen by the LLM.

### 1.2 Why the live prompts failed (mapped to code)
- `"set search engine to duckduckgo"` → config gate returned `None` (search.engine is **YELLOW**,
  and our GREEN-only gate at `mod.rs:384` falls through) → the **browser-search regex (~617)**
  matched, `query="engine to"`, `site="duckduckgo"` → dispatched raw → browser opened.
- `"what is my current theme?"` → system-info branch (470) matched "what is my/current" but had no
  hardware keyword → fell through → ReAct → semantic/FastEmbed surfaced knowledge tools
  (`searxng_search`, `search_marketplace`, `recall_fact`) → LLM called them → hallucinated.
- `"revert the last setting"` → not an `Act` with a field → `None` → ReAct → LLM (down) → error.
- Read-back/undo/YELLOW/temp have **no chat handling**; the rich flow lives only in the
  `config_prompt` desktop command, which chat never calls.

### 1.3 Confirmed defects (RC + NEW), verified
- **RC1/RC5:** deterministic path is raw (no HITL) → GREEN-only compensation → YELLOW/RED fall
  through and get **misrouted** (browser/search) instead of gated.
- **RC2/NEW-2/NEW-6:** `ConfigService::read_field` exists + tested but is **never called in
  production**; read-back is unhandled in chat AND the command (evaluate returns `NotAChange`).
- **RC3:** undo only in the command, only `contains("undo")` — "revert"/"change back" miss.
- **RC4:** two divergent entry points (chat deterministic vs `config_prompt` command).
- **RC6:** LLM prefers knowledge/browser tools over `config_patch`; deterministic routing is the
  only reliable path and it's incomplete.
- **RC7:** `recall_fact` FTS5 crash on `"?"`; `search_marketplace` fires on nearly every query.
- **NEW-1 (verified):** `update_settings` (`app_commands.rs`) preserves only `providers` +
  `llm.*`; `redact_secrets` clears 5 fields → whole-blob save **wipes** `planner.cloud_api_key`,
  `server.jwt_secret`, `telegram.bot_token`, `image_generation.hf_inference_token`.
- **NEW-3:** injection-wall `is_external_content_tool` uses invented names
  (`web_search`/`fetch_webpage`) not the real `searxng_search`/`search_marketplace`/browser → the
  wall never fires for the actual attack vector.
- **NEW-4:** `patch_config` command validates `field_exists` only, no secret guard.
- **NEW-5:** injection taint is a single process-global `AtomicBool` on the shared `ToolRegistry`
  → cross-turn/session bleed.
- **NEW-7:** `PromptAnalyzer::new()` rebuilds its synonym index every call, invoked 2×/turn.
- **NEW-9 (new):** pre-ReAct forcing (`LiveFactClassifier`/`GuiIntentClassifier`) can force
  searxng/browser on a settings prompt before any settings stage.
- **NEW-10 (new):** the browser-search regex is greedy enough to eat `"set search engine to …"`.
- **NEW-11 (new):** deterministic dispatch bypasses the single PolicyEngine/HITL gate entirely.
- **NEW-12 (new):** `RoutingContext` isn't per-session/populated → no usable conversation state at
  routing time; the self-reference gate must rely on `messages`, not RoutingContext.
- **NEW-13 (new):** `config_prompt` command and chat deterministic gate duplicate classification.

### 1.4 Plan V1 (first cut)
Insert a proper settings gate earlier; wire read-back/undo/YELLOW into chat; share one handler;
fix secret clobber; fix injection tool names; make taint per-turn. *(Critiqued in Wave 2.)*

---

## WAVE 2 — Self-critique of Plan V1

- **C-a (architecture):** "insert earlier + wire more branches" still leaves TWO deciders (a
  deterministic gate AND the LLM router). The real fix is a **single Settings Intent stage** that
  runs before all forcing/regex/semantic routing and, when confident, **owns the turn** via one
  shared handler. Otherwise misrouting recurs for any phrasing the gate misses.
- **C-b (hardcoding risk):** a keyword/regex settings gate cannot scale to 500+ settings and
  violates the no-hardcoding rule. Classification must be **semantic + schema-grounded** (synonym
  embeddings + field index), with the LLM used only for *constrained extraction*, not discovery.
- **C-c (HITL correctness):** routing settings changes through the raw deterministic path can never
  be right for YELLOW/RED. The handler must run through the **same PolicyEngine + HitlGateway**.
  Design a `SettingsHandler` that performs gating itself (like the command does) OR emits a gated
  tool call — but ONE mechanism, reused.
- **C-d (separation):** Configuration vs Conversation intent needs real evidence. RoutingContext is
  empty (NEW-12), so the classifier must derive conversation state from `messages` (recent topic,
  subject markers) — designed as a **ConversationContext adapter**, not a dependency on the broken
  RoutingContext.
- **C-e (secrets):** the secret-preserve fix must be **derived from `is_secret_field`** so it can't
  drift again; a static list is how NEW-1 happened.
- **C-f (injection):** hardcoding tool names (even correct ones) will drift again. Derive
  "external-content tool" from a **tool capability/category** (tools declare a `reads_external`
  trait/metadata) rather than a name list — future-proof.
- **C-g (observability):** without a decision trace, misroutes are undiagnosable in production. Keep
  a `SettingsIntentTrace` (scores, candidates, decision) — mandatory, not optional.

Plan V2 = Plan V1 + single-stage ownership + semantic/schema classifier + one gated handler +
ConversationContext adapter + `is_secret_field`-derived preservation + capability-based provenance
+ mandatory trace.

---

## WAVE 3 — Stress test (future scale & concurrency)

- **100 new settings / 500+ fields:** classifier must be an **indexed semantic matcher** over
  schema synonyms/fields (built once, cached — Req 12), not linear scans. Value mapping uses
  schema `valid_values`/synonyms + constrained LLM extraction. ✔ designed as `SchemaEntityIndex`.
- **50 new providers:** provider/model changes must keep routing through `apply_provider_selection`
  and the availability resolver (C4.1 of prior spec); the settings handler must not raw-write
  providers. ✔ handler delegates provider/model to the dedicated apply service.
- **Multiple chat sessions / concurrent users:** provenance/taint and any per-session state MUST be
  **per-turn/per-session isolated** (NEW-5). ✔ provenance carried on the turn/`ToolContext`,
  never a global flag; classifier caches are read-only shared, per-session state keyed by session.
- **Future memory system:** cross-session recall bolts on as a **ConfigService/audit reader**; the
  pipeline must expose a clean seam (`ReferentialResolver` trait) and degrade gracefully today. ✔
- **Future planner/agent:** the Settings stage is a **pluggable pre-planner classifier** with a
  typed decision; a future planner can call the same `SettingsHandler` API. ✔ handler is the stable
  contract, classifier is swappable (strategy pattern; trained model can replace heuristics).
- **Future tools/marketplace:** injection provenance via capability metadata means new external
  tools are covered automatically; settings gate claims only settings intent so new tools are
  unaffected (Req 14). ✔
- **Backend variance (local vs cloud LLM):** extraction uses grammar on local, structured-output +
  strict validate + reask on cloud (Req 8.4). ✔
- **LLM unavailable:** GREEN changes, read-back, and undo must work WITHOUT the LLM (deterministic
  schema resolution) so settings control survives model outages (the log showed model-down
  failures). ✔ classifier heuristic tier is LLM-free; LLM only refines extraction when available.

Plan V3 = Plan V2 hardened with the above seams (indexed matcher, referential seam, per-session
isolation, LLM-optional path).

---

## WAVE 4 — Final architecture

## Architecture
A single **Settings Intent pipeline** runs as the **first classification stage of a turn** (before
LiveFact/GUI forcing, deterministic regex, and semantic routing), gated by `KRIA_NL_SETTINGS`. When
it is confident the message is Configuration Intent, it **claims the turn** and hands a typed
`SettingsRequest` to one shared `SettingsHandler`; otherwise it returns `NotSettings` and the turn
proceeds through the existing pipeline untouched. Chat and the desktop command surface both call
the same pipeline + handler.

```
 user message ──▶ SettingsIntentPipeline (KRIA_NL_SETTINGS)
                    │  cheap domain gate → SchemaEntityIndex (semantic+synonym)
                    │  → ConversationContext (subject/topic from `messages`)
                    │  → intent-kind + scope + confidence → SettingsIntentTrace
                    ▼
        ┌───────── SettingsDecision ─────────┐
        │ Change | ReadBack | Undo |         │
        │ TempOverride | Clarify | NotSettings│
        └───────┬─────────────────────────────┘
                │ (NotSettings) ──▶ existing chat/tool pipeline (unchanged)
                ▼
          SettingsHandler  (ONE implementation; chat + command call this)
             validate(schema) → availability(C4.1) → provenance/injection wall
             → risk gate (PolicyEngine)
                  GREEN  → ConfigService.patch (auto)
                  YELLOW/RED/BLACK → HitlGateway approve → patch  (deny → no-op)
             → effect dispatch (existing services) → audit (redacted) → event
          ReadBack  → ConfigService.read_field (no mutation)
          Undo      → change-set forward patch (audit-preserving)
          TempOverride → RequestOverride threaded into the turn context
                ▼
        response rendered (applied/answer/needs-approval/clarify) + UI live-sync
```

## Components and Interfaces

### C1. `SettingsIntentPipeline` (`crates/kria-core/src/config/nl/pipeline.rs` — new)
Stateless-per-call classifier over cached indices. Stages, cheapest-first, LLM-optional:
1. **Domain gate** — cheap semantic score "is this about KRIA configuration at all?" (reuse
   `routing/semantic.rs` embeddings; centroid over schema synonym corpus). Below floor ⇒
   `NotSettings` immediately (Req 12.3, cheap for normal chat).
2. **Entity resolution** — `SchemaEntityIndex` (C2) maps the message to candidate `(section,field)`
   via synonym + embedding similarity (NOT keyword branches, Req 3.2).
3. **Conversation-vs-Configuration** — `ConversationContext` (C3) supplies subject markers
   ("your/the app/KRIA" vs "I/my/this code/project"), recent topic (from `messages`), and
   correction signals; combined with schema grounding to decide KRIA-directed vs user-artifact
   (Req 2). No RoutingContext dependency (NEW-12).
4. **Intent kind** — imperative(change) vs interrogative(read-back) vs undo-intent vs temp-scope,
   scored (extend the ONNX intent classifier where present; heuristic fallback offline).
5. **Extraction** — value via schema `valid_values`/synonyms; if unresolved and LLM available,
   constrained extraction (grammar local / structured cloud) + strict validate + reask (Req 8.4).
6. **Decision** — combine stage scores into a confidence; ≥act ⇒ typed decision; mid ⇒ `Clarify`;
   low ⇒ `NotSettings`. **Fail toward conversation** (Req 2.6). Emits `SettingsIntentTrace`.

Each stage is a swappable scorer behind a trait (strategy pattern) so a trained classifier can
replace stages 1–4 later without redesign.

### C2. `SchemaEntityIndex` (`crates/kria-core/src/config/nl/entity_index.rs` — new)
- Built ONCE from the `FieldMeta` registry + `KriaConfig` shape (synonyms, valid_values, section/
  field names) and cached (Req 12.1). Embedding vectors for each field's synonym set; nearest-field
  lookup by cosine. Scales to 500+ fields (indexed, not linear) (Req 3.3).
- Rebuild only on schema version change. Shared read-only across sessions/threads.

### C3. `ConversationContext` adapter (`crates/kria-core/src/config/nl/conversation.rs` — new)
- Derives lightweight state from the `messages` vec (the real per-turn state, NEW-12): recent
  topic embedding, last N user/assistant turns, subject-marker extraction, "were we discussing
  code/CSS/a project?" signal. Per-session (keyed by session_id), isolated (Req 12.2).
- No dependency on the broken shared `RoutingContext`.

### C4. `SettingsHandler` (`crates/kria-core/src/config/nl/handler.rs` — new; the ONE path)
- Input: `SettingsRequest { kind, section, field, value, scope, provenance, session_id }`.
- `Change`: schema validate → availability (C4.1 prior spec) → **injection wall (provenance!=User
  ⇒ refuse)** → PolicyEngine risk → GREEN `ConfigService.patch`; YELLOW/RED/BLACK ⇒ HITL via a
  `HitlSink` abstraction → on approve patch, on deny no-op → effect dispatch → audit (redacted).
- `ReadBack`: `ConfigService.read_field` → format (secrets → set/unset only) (Req 5).
- `Undo`: change-set forward patch (Req 6).
- `TempOverride`: build `RequestOverride`, attach to turn context (Req 7).
- Provider/model changes delegate to `apply_provider_selection` (never raw write).
- **Outcome-return pattern (per Wave 5 F1, supersedes `HitlSink`):** the handler does NOT embed
  streaming/HITL. It returns a typed `SettingsOutcome`; for non-GREEN it returns
  `NeedsApproval{change_set_id,...}`. The CALLER drives approval through its existing gate (chat →
  `HitlGateway` + `StreamEvent::ApprovalRequired`; command → `agent:approval_required`) and then
  calls `handler.apply_approved(change_set_id)`. Same handler + same gate for both surfaces, no raw
  path (Req 4.4), and no core→streaming dependency.

### C5. Turn integration (`crates/kria-core/src/agent/loop_engine/mod.rs` — modified)
- A new **first stage** in `run_with_profile`, inserted **after turn admission + `TurnAccepted`
  (~5303)** and **before IntentGate fast-path (5341), LiveFact/GUI forcing (5393/5411), and the
  deterministic pre-dispatch (5741)** (Wave 5 F4/F5). If `KRIA_NL_SETTINGS` and the pipeline
  returns a turn-claiming settings decision, run the `SettingsHandler`, render its `SettingsOutcome`
  as StreamEvents (+ drive HITL for `NeedsApproval` then `apply_approved`), emit `Done`, and return
  — bypassing browser/search/GUI (Req 4.1, 4.5, 14.2). `NotSettings` ⇒ untouched flow.
- **Multi-intent (Wave 5 F6):** when the decision is settings + a residual task, apply the settings
  clause, strip it, and let the normal pipeline continue for the task (do NOT finish the turn).
- Remove reliance on the old GREEN-only `try_config_prompt_dispatch` in the deterministic chain
  (superseded); keep it inert/removed under the new flag to avoid double-deciding (RC4/NEW-13).
- Provenance is carried on the turn/`ToolContext` per-turn (NEW-5), set from the trigger + tainted
  by capability-based external-content detection (C6).

### C6. Capability-based injection provenance (`tools/mod.rs`, `registry.rs` — modified)
- Tools declare `reads_external_content: bool` in their `ToolDef`/metadata (or a category). The
  loop taints the turn's provenance when any such tool runs — derived from the **live registry**,
  not a name list (Req 9.3, fixes NEW-3). Provenance stored per-turn, not a global `AtomicBool`
  (fixes NEW-5).

### C7. Secret-safe writes (`app_commands.rs`, `config/service.rs` — modified)
- `update_settings` preserves EVERY `is_secret_field` from the live config (iterate the single
  source), so a redacted blob can never clobber (fixes NEW-1, Req 10.1/10.4).
- `patch_config` command guards secret fields (route to vault flow or refuse) (fixes NEW-4).
- Whole-blob save audits consistently with field patches (NEW-8, Req 11.3).

### C8. Command-surface parity (`commands/config_prompt.rs`, `ui/` — modified)
- `config_prompt` command becomes a thin caller of the SAME pipeline + handler (delete its
  duplicate undo/keyword logic) (RC4/NEW-6/NEW-13). Read-back + undo synonyms now work there too.
- Frontend: keep field-level `patch_config`, config-changed re-fetch, history viewer, badges; add
  nothing divergent.

### C9. Misc correctness (RC7)
- Sanitize `recall_fact` FTS5 input (strip/escape `?` and reserved chars) — small, isolated.
- Ensure the settings stage claims settings-read queries so `search_marketplace` no longer fires
  for them (Req 4.5).

## WAVE 5 — Implementation-readiness hardening (self-review flaws + resolutions)

A critical self-review of Wave 4 found 17 gaps that would block implementation or cause bugs. Each
is resolved below; these resolutions are BINDING for `tasks.md`.

- **F1 — Response rendering / layering (BLOCKER).** Wave 4 said the handler "renders result", but
  streaming (`StreamEvent::Token/Done/ApprovalRequired`) is an agent-loop concern and the handler
  lives in `kria-core/config`. **Resolution:** the handler returns a typed **`SettingsOutcome`**
  ` { Applied{section,field,value,version,message} | Answer{text} | NeedsApproval{section,field,
  value,risk,change_set_id} | Clarify{question} | Refused{reason} | TempApplied{...} | Undone{...}
  | NothingToUndo }`. The **caller renders** it (loop → StreamEvents; command → JSON). HITL is NOT
  embedded in the handler: handler returns `NeedsApproval`; the caller drives approval via its own
  gate, then calls `handler.apply_approved(change_set)`. This mirrors the proven
  `evaluate → NeedsApproval → apply_approved` pattern and removes the core→streaming dependency.
  (Supersedes the `HitlSink` idea in C4; `HitlSink` is dropped.)

- **F2 — Pipeline dependencies (BLOCKER).** The classifier needs an embedder, optional LLM client,
  `ConfigService`, and `SchemaEntityIndex`. **Resolution:** `SettingsIntentPipeline::new(deps)` where
  `deps = { entity_index: Arc<SchemaEntityIndex>, config: Arc<ConfigService>, embedder:
  Option<Arc<dyn TextEmbedder>>, llm: Option<Arc<dyn ChatClient>> }`. All optional deps degrade
  gracefully (F3). Built once in `AppState`, injected into the loop + the command.

- **F3 — Embedder availability / cold start.** FastEmbed may be cold/absent (CI, boot).
  **Resolution:** domain gate + entity resolution use a **two-tier matcher**: (tier A) synonym /
  token-overlap / edit-distance over the schema corpus (always available, LLM- and embedder-free);
  (tier B) embedding cosine when the embedder is ready (higher recall). Tier A alone must pass the
  GREEN/read-back/undo golden cases so settings work at cold start and offline (P10).

- **F4 — Exact insertion point / ordering (BLOCKER).** The settings gate MUST run **after** turn
  admission + `TurnAccepted` (so the UI sees a normal turn lifecycle) but **before** IntentGate
  fast-path (`mod.rs:5341`), LiveFact/GUI forcing (5393/5411), and deterministic pre-dispatch
  (5741). Concretely: insert immediately after `TurnAccepted` (~5303) and before 5341. If the gate
  claims the turn it emits its outcome + `Done` and returns; IntentGate/forcing never run for it.

- **F5 — Turn lifecycle.** The settings turn goes through the SAME admission + `TurnAccepted` +
  `Done`/error events as any turn (F4), so the frontend turn-state machine (spinner, cancel,
  history) behaves normally. Provenance is set on the turn context at this point (User, unless
  tainted — but at turn start nothing is tainted).

- **F6 — Multi-intent prompts.** "switch to dark mode AND generate a cat". **Resolution:** the gate
  only **fully claims** the turn when the message is settings-DOMINANT (no residual actionable task
  after removing the settings clause). If the classifier detects a settings clause + a non-settings
  task, it: applies the settings clause (GREEN) or asks approval, THEN **does not finish** the turn
  — it rewrites the remaining message (settings clause stripped) and lets the normal pipeline
  continue for the task. YELLOW/RED settings in a mixed prompt ⇒ handle settings first (approval),
  then continue. This is a scored `claims_turn: bool` on the decision, not a hard rule.

- **F7 — Read-back effective value.** Read-back uses `ConfigService.get()` (effective: env +
  provider-sync applied), not raw rows, so provider-synced fields (`llm.active_model`) read
  correctly. Formatting via `FieldMeta` (enum labels), secrets → set/unset only.

- **F8 — Scoring contract (BLOCKER for determinism).** Each stage emits a score in `[0,1]`; the
  decision combines them into `confidence ∈ [0,1]` via a documented weighted function
  (domain 0.35, entity-match 0.30, intent-kind 0.20, conv-vs-config 0.15; weights are
  config-tunable). Bands: `confidence ≥ act_threshold(0.72)` ⇒ act; `clarify_threshold(0.45) ≤ c <
  act` ⇒ Clarify; `< clarify` ⇒ NotSettings. Thresholds live in a tunable struct with defaults,
  calibrated against the golden set (F9). `SettingsIntentTrace` records every stage score + the
  final band so misroutes are diagnosable.

- **F9 — Golden set is a first-class artifact.** A versioned `config/nl_golden_set.jsonl` (prompt,
  expected decision, expected field, context) is authored EARLY (before the classifier) from
  `settings-config-revamp/analysis.md` §5 + the live-failure prompts + the mandatory suite. The
  classifier is TDD'd + threshold-calibrated against it. It doubles as the Task 13 regression set.

- **F10 — FieldMeta coverage.** Only ~14 fields are annotated today; the rest are fail-closed. NL
  recall is only as good as annotations + tier-B semantics. **Resolution:** a task expands
  `FieldMeta` (synonyms, valid_values, prompt_changeable, risk) for all user-facing settings
  BEFORE relying on scale claims. Unannotated ⇒ fail-closed (correct, safe).

- **F11 — Injection scope clarified.** The wall covers TOOL-ingested external content (web/file/MCP
  output) triggering a change; a user PASTING text is still `User` provenance (they typed it) and
  is allowed — documented as intended, not a hole.

- **F12 — Durable undo (BLOCKER for restart).** The in-memory history ring is per-process
  (empties on restart). **Resolution:** Undo reads the last `config_change` from the **durable
  audit ledger** (`AuditLogger::config_change_history`) when the in-memory ring is empty, so
  "revert the last setting" works after a restart. Undo remains a forward patch (audit-preserving).

- **F13 — "How do I…" help intent.** Recognized as a distinct `HelpAbout` outcome that answers
  from schema metadata (where the setting lives / valid values) instead of guessing — optional but
  specified so it isn't silently misrouted.

- **F14 — Offline tests.** Classifier unit tests run with the embedder absent (tier-A only) so CI
  is deterministic; tier-B (embedding) tests are gated on embedder availability and marked skipped
  otherwise (no fabricated pass).

- **F15 — Retire duplicate deciders (BLOCKER for RC4).** `PromptAnalyzer`, `prompt/patch.rs`
  `evaluate`, the GREEN-only `try_config_prompt_dispatch`, and `build_turn_override` are SUPERSEDED
  by the pipeline. Task: migrate their still-useful logic into the pipeline and REMOVE them (or
  make them thin shims) so there are not three deciders. The `config_prompt` command becomes a thin
  caller (C8).

- **F16 — Flag migration.** `KRIA_NL_SETTINGS` folds in `KRIA_CONFIG_PROMPT_CONTROL`; a truthy old
  flag maps to the new one for one release with a deprecation log. Existing flag-off parity tests
  updated to the new flag.

- **F17 — Provider/model change mid-settings-turn (BLOCKER — deadlock risk).** `apply_provider
  _selection` rejects a swap while a local turn is active; the settings turn IS a turn. **Resolution
  (reuses N3 from `settings-config-revamp/design.md` C1.1):** provider/model/tier changes requested
  via the settings gate are NOT counted as an active-local-turn conflict against themselves; the
  handler applies them via the apply service which either succeeds or returns "will apply after the
  current generation" — never deadlocks or silently drops. Non-provider GREEN/YELLOW fields are
  unaffected.

## Data models
- `SettingsOutcome` (F1), `SettingsDecision` / `SettingsRequest` / `SettingsIntentTrace` (new types, core).
- `SchemaEntityIndex` cache keyed by schema version.
- Reuses `ConfigService`, `FieldMeta`, `RequestOverride`, `AuditLogger`, `HitlGateway`,
  `PolicyEngine` — no new storage.

## Correctness properties
- **P1 One path:** chat and command produce identical `SettingsDecision` + effect for the same
  prompt+context (Req 1.4).
- **P2 Separation:** a message resolved `NotSettings` never mutates config; a KRIA-directed
  imperative on a schema field is `Change` (Req 2).
- **P3 No raw mutation:** every config mutation passes PolicyEngine + (if non-GREEN) HITL (Req 4.4).
- **P4 Read truth:** read-back equals `ConfigService` effective value (Req 5.1).
- **P5 Secret safety:** no write path clears/leaks a secret; preserve-set = `is_secret_field`
  (Req 10).
- **P6 Injection wall:** non-User provenance never mutates; external detection tracks live registry
  capability (Req 9).
- **P7 Per-session isolation:** provenance/state never bleeds across turns/sessions (Req 12.2,
  9.4).
- **P8 No hardcoding:** adding a schema field enables NL control with zero routing-code changes
  (Req 3.1) — proven by a test that adds a synthetic field.
- **P9 Legacy equivalence:** flag off ⇒ byte-for-byte legacy (Req 1.3).
- **P10 LLM-optional:** GREEN change, read-back, undo succeed with the LLM unavailable (Wave 3).

## Error Handling
- Ambiguous intent ⇒ ONE clarifying question, never a guess or mutation (Req 2.5).
- Invalid value ⇒ grounded rejection listing allowed values (reask), local + cloud (Req 8.1, 8.4).
- Unavailable target (provider/sidecar absent, env-locked) ⇒ informative refusal, no silent apply
  (Req 8.2, 8.3).
- Injection (provenance != User) ⇒ refuse mutation (Req 9.1).
- Fallible effect failure/timeout ⇒ do not persist; surface error; prior value stands (reuse
  transaction model from `settings-config-revamp/design.md` C1.1).
- LLM unavailable ⇒ GREEN change / read-back / undo still succeed via the deterministic schema tier
  (Wave 3, P10); only LLM-refined extraction degrades to clarify.
- Corrupt/unopenable config store ⇒ fail closed to defaults (inherited from ConfigService).
- Secret write attempt via a non-vault path ⇒ refuse/route to vault (Req 10.2).

## Testing Strategy
### Backend (unit/integration)
- Classifier golden set (Configuration vs Conversation, false-positives, ambiguity, multilingual,
  negation, idempotency) — from `settings-config-revamp/analysis.md` §5 (reuse + expand).
- Entity-index scaling test (synthetic 500 fields), no-hardcoding test (add synthetic field).
- Handler tests: GREEN auto, YELLOW/RED HITL approve+deny, invalid-value grounded reask,
  availability refusal, env-lock refusal, injection refusal (capability provenance), undo synonyms,
  temp override apply+revert, secret preserve on whole-blob save, secret guard on patch_config.
- Property tests P1–P10.

### Real frontend (mandatory — Req 13)
Drive the actual desktop app via tauri-driver + WebKitWebDriver (isolated `HOME`), issuing prompts
through the real chat UI / same IPC the UI uses, asserting the rendered result AND persisted state
(python sqlite3) AND live UI sync. The full mandatory prompt suite (Req 13.1) must pass end-to-end,
including Conversation-vs-Configuration ambiguity in both directions and natural human phrasings.
Features unverifiable on-box (OS keychain) marked honestly (Req 13.2).

## Rollout & flags
- `KRIA_NL_SETTINGS` (default off) gates the whole pipeline + turn integration. Reuses
  `KRIA_CONFIG_BACKEND`/`KRIA_CONFIG_SERVICE`/`KRIA_CONFIG_PROMPT_CONTROL` (the last folds into the
  new flag). All off ⇒ legacy (P9).

## Out of scope (v1)
- Cross-session referential recall (memory subsystem) — seam only (`ReferentialResolver`).
- Prompt reconfiguration of `kria-server` (fleet stays file/env).
- Config-tunable risk tiers (compile-time for safety).
