# Wave 9 — Implementation Handoff (Capability Synthesis / Engineering)

> Memory handoff for continuing Wave 9. Trust ONLY code + runtime + this doc.
> Blueprint source: the Wave 9 Architecture Blueprint (IR-over-HTN + reuse-OpenClaw-sandbox).

## UPDATE — W9-R1…R12 landed (this session)
Blueprint fixes applied + verified (see `WAVE9_ARCHITECTURE_BLUEPRINT.md` + tasks.md M9 remediation note):
- **W9-R1** `intelligence/capability_graph.rs` (typed hashable IR + validate + effects_union + NodeExecutor).
- **W9-R2** `CapabilitySpecification.graph`/`normalized_graph()`/`ir_hash()`; provider executes the IR.
- **W9-R3** live pre-activation **smoke gate** in `synthesize_for_goal` + `acquire_for_goal_reasoned` (quarantine+rollback on fail) — the top R21 safety gap, now closed.
- **W9-R4** provenance (Decision Record `chosen=Some`+ir_hash; `SynthesizedRecord` manifest; descriptor `ir_hash`/`ir_graph`).
- **W9-R5** `Stage::Synthesize` granular events. **W9-R6** effect-union descriptor. **W9-R7** in-flight lock.
- **W9-R8** `platform.execute_synthesized_graph` + `PlatformNodeExecutor` (composed capability-node graphs; single executor).
- **W9-R11** `IrProposer`/`DeterministicIrProposer`/`propose_validated` (model-optional, validator-mandatory).
- **W9-R12** Generate tab + `cpp_synthesis_preview`/`cpp_synthesize`.
Tests: `capability::` lib 170 pass; `capability_wave9_synthesis` 9 pass; neutrality green; both crates check clean; ui build clean.

### Remaining for the NEXT session (honest blockers)
- **W9-R13** Tier-3 raw code-gen node + seccomp-Docker — blocked on a reliable code model (scaffold/flag only).
- **W9-R9** true multi-input / non-text modalities — IR/descriptor IO infra ready; needs multi-input primitive/capability nodes.
- **W9-R10** auto-repair-by-regenerate — version+provenance done; wire "evolution repair proposal → re-synthesize from stored IR".
- **W9-R11 LLM proposer** — `LlmIrProposer` behind `synthesis_llm` flag; blocked on model reliability.
- **Live GUI-click campaign** — Tauri IPC, needs a webview harness (Wave 12).

## Where Wave 9 stands (verified) — pre-remediation baseline below
- **Done (foundation):** `capability/intelligence/synthesis.rs` (`CapabilitySpecification`, `CapabilityGapAnalyzer`, `GapResolution`), `capability/intelligence/primitives.rs` (11 audited pure ops + `apply_primitive`/`apply_pipeline`/`infer_primitive_from_goal`/`infer_pipeline_from_goal`), `capability/acl/synthesis.rs::SynthesisProvider` (acquire=generate, execute=run pipeline, lowest trust, elevated effects → permission-gated, golden-case smoke, atomic temp+rename writes), `platform.rs::synthesize_for_goal` (fall-through from `acquire_for_goal_reasoned` when no catalog candidate + synthesizable; Decision Record path=Generate + trust gate + CKB install/outcome + events), `with_synthesis(provider_id)` (neutral, data-injected), registered in BOTH runtime.rs (chat) + capability.rs (UI) behind the `synthesis` flag. Composition = **linear pipeline** of primitives.
- **Tests:** `tests/capability_wave9_synthesis.rs` (6), primitives units (6), synthesis units (4). `capability::` lib 162 pass. Neutrality gate green. Flag default OFF.

## The 12 issues (A–L) + roadmap this handoff drives
- **A** real generation = deterministic keyword→primitive lookup (no model). → Blueprint: **Capability-Graph IR** (nodes=primitive|installed-cap|provider; typed edges), LLM proposes IR (Tier-2), raw code = guarded Tier-3.
- **B** no sandbox for generated code (R11.4/§38) → reuse OpenClaw Docker+seccomp substrate; Tier-3 only.
- **C** composition linear-only → generalize to a validated DAG IR that emits into the EXISTING HTN runtime (no new engine).
- **D** mono-input `{text}` → typed nodes + descriptor `input_schema`/`io_modality`; reuse Wave-3 `planner::io_links`.
- **E** events incomplete (no `capability:synthesis`) → granular neutral Stage emission from `synthesize_for_goal`.
- **F** no Generate UI → `cpp_synthesize*` commands + Generate tab in `CapabilitiesView.tsx`.
- **G** no versioning/repair/optimize/migrate → wire synthesized caps into the Wave-8 EvolutionEngine (repair=re-synth from IR).
- **H** determinism/provenance → record model id + policy version + IR hash in the Decision Record.
- **I** trust laundering via composition → union effects at max-risk (`planner::union_effects` + `plan_permission`).
- **J** SynthesisProvider.catalog() empty → gap analyzer is the explicit synthesize driver (not empty-ranking coincidence).
- **K** concurrent synthesis of same goal → in-flight lock keyed by capability_id.
- **L** cost/latency → gap-gated + budgeted LLM calls; IR cached by capability_id.

## Roadmap (blueprint order)
W9-R1 Capability-Graph IR (pure, no LLM, testable) ← **implement first**
W9-R2 LLM-assisted IR generation (generate→validate→repair; honest-decline; provenance) — model-dependent
W9-R3 emit IR into existing HTN runtime
W9-R4 granular events + cpp_synthesize* + Generate FE tab
W9-R5 evolution wiring (repair/optimize/version)
W9-R6 Tier-3 code sandbox (OpenClaw+seccomp) — **needs a reliable code model (external blocker)**
W9-R7 real desktop campaign

## Genuine external blockers (do NOT fabricate)
- **Reliable code-generation model**: local Qwen3-VL-4B mis-routes; cloud opencode gateway returns 500/403 on tool payloads (proven). Tier-3 raw code-gen (W9-R6) cannot be validated → keep behind its own flag, ship Tiers 0–2 without it.
- **Live GUI validation**: `cpp_*` are Tauri IPC (no HTTP); headless webview automation unavailable → Wave 12 real-UI scope.

## Guardrails (unchanged)
Brain decides / Hands execute (R23); neutrality gate green (no `crate::openclaw`/`crate::mcp::`/provider-name branching in `capability/` outside `acl/`); flags default OFF ⇒ byte-identical legacy; no fake/stub on hot paths; honest-decline over fabrication; reuse existing HTN/sandbox/CKB/events — add no rival engine.

## Verification commands
```
cargo test -p kria-core --lib "capability::"
cargo test -p kria-core --test capability_wave9_synthesis
cargo test -p kria-core --lib "capability::intelligence::neutrality"
cargo check -p kria-core && cargo check -p kria-desktop
```
