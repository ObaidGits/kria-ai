# Settings Intelligence — PROGRESS

Living status of the `settings-nl-intelligence` spec (builds on the shipped
`settings-nl-control` backbone). Newest first.

## Status by wave / task
- **Wave 1 (value engine + coverage)** — DONE. Type-inferred `ValueEngine`
  (bool/int/float/enum/alias, schema-driven), numeric bounds + range validation, 13-field
  coverage batch (curated risk).
- **Wave 3 (catalog/help/explain/read-back)** — DONE. `catalog.rs` + `InfoQuery` + humanized
  read-back (restart/env-lock notes). Answer-from-system, no LLM.
- **Wave 2 (evidence-based intent)** — DONE. `evidence.rs` (TextEmbedder + MemoryEvidenceSource
  seams, EvidenceWeights), semantic conversation topic-affinity (embedder w/ lexical fallback),
  weighted fusion, FastEmbed embedder wired into the chat stage, durable trace. Live-validated.
- **Wave 4 (multi-turn provider configuration)** — DONE. `flow.rs` slot-filling engine + FlowStore,
  schema-driven provider catalog (`ProviderType::all/synonyms/resolve`), secret-safe commit
  (`commit_provider` → `replace_all` redact+vault). Live: OpenAI multi-turn, Ollama local, cancel.
- **Task 9b (provider lifecycle + catalog/read-back)** — DONE (config-level, live). Runtime swap +
  cloud connection test = dedicated fallible desktop path (needs real creds → not on-box provable).
- **Wave 5 Task 10 (evidence-based injection gate)** — DONE. `injection_gate.rs` (competition +
  domain agreement + negative evidence + floor + strong + cap), replaces the flat 0.35 filter.
  6 unit tests + live no-interference probe.
- **Task 11 (locking + observability)** — DONE. Approval TTL/GC + deny/timeout release, optimistic
  concurrency (`replace_all_checked` + retry in commit_provider), durable JSONL diagnostics.
- **Task 10b (real routing campaign)** — DONE (live, real model). Ran 31 prompts through the actual
  app + real IPC against the user's configured Qwen3-VL-4B (llmLive=true): interference=0/20, all
  settings/provider routing correct, provider read-back returned the real active model. Harness:
  `scripts/run_routing_campaign.sh` + `specs/routing_campaign.e2e.ts` (isolated HOME, non-destructive,
  `KRIA_MODELS_DIR` → real models). Honest scope: proves no-interference + settings/provider routing;
  positive tool-EXECUTION (file/vision/openclaw firing) not covered (needs fixtures+permissions → 10c).

- **Task 4 / Task 13 (production audit — large real-app campaign)** — DONE (live, real app+IPC+DB).
  `specs/settings_audit.e2e.ts`: 55 negative + 30 positive + DB verification + real-HITL YELLOW
  approval, all via `config_prompt` (the same classifier/handler chat uses) against an isolated HOME
  copying the real config. First run surfaced **9 real bugs** (8 negative false-positives + 1 positive
  misroute); all root-caused and fixed GENERICALLY (no hardcoding), then re-run GREEN:
  negative **0/55** false-positives, positive **0/30** misroutes, DB reflects applied changes, YELLOW
  applies after approval. Four pipeline fixes (all in `config/nl/pipeline.rs`):
  1. **Desire phrasing** — `DESIRE_MARKERS` folded into `is_imperative` (acts only with a resolved
     field+value) → "I want dark mode" applies.
  2. **Help over-fire** — `detect_info` split into settings-directed vs definitional tiers;
     definitional ("explain X"/"what does X") requires a STRONG explicit setting reference (act
     threshold) → knowledge questions ("explain how voice recognition works") fall through.
  3. **Content-authoring guard** — a message opening with a content verb (write/generate/draw/…) is a
     generation request, never a settings mutation ("write code to change a theme") — exempts the
     turn-scoped "generate … using local/cloud" image-routing phrasing.
  4. **Declarative-statement guard** — a copular opinion ("dark mode is ugly") no longer triggers the
     bare-noun clarify path.
  5. **Word-boundary marker match** (`contains_word_marker`) — fixes the self-ref substring bug where
     "my" matched the tail of "autono**my**" ("what is autonomy in philosophy" → not a read-back).
  6. **Provider definitional narrowed** — only "explain <provider>" maps to the provider catalog;
     "what is OpenAI" (knowledge) falls through. Permanent regression test
     `audit_false_positives_are_fixed` added.

## Verification snapshot (latest)
- Backend: `config` 129, `config::nl` 72, `agent::loop_engine`(+injection_gate) 120,
  `kria-desktop` 133 — all green. Desktop compiles clean; `cargo fmt` clean.
- Live (tauri-driver, real app): production audit **4/4 passing** (negative 55/0, positive 30/0, DB,
  YELLOW). Earlier suite 26 pass / 1 skip. Persistence verified on disk; API keys redacted.
- Toolchain note: rustc 1.95.0-dev ICEs while ANSI-rendering certain unused-import warnings in the
  lib-TEST build — run config/loop tests with `--message-format=short` (source compiles fine; a plain
  `cargo tauri build` is unaffected).
- Known: `cargo clippy` on kria-core blocked by a PRE-EXISTING `since="batch-1"` lint in
  `automation/workflows.rs` (unrelated). Pre-existing flake `agent::continuation_reentry` under full
  parallel run (passes `--test-threads=1`).

## Honest remaining
- 10b positive live campaign (needs a capable local LLM + ModelRouter provider wiring).
- Provider runtime-apply + cloud connection test (real creds).
- Coverage batch 2 (more sections). Full 40–60 prompt live matrix once an LLM oracle is available.
