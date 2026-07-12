# CPP Implementation Progress

Live implementation log for the Capability Provider Platform. Evidence, decisions,
deviations, bugs, fixes. Keep in sync with `tasks.md`.

## Status summary

| Milestone | Status | Evidence |
|---|---|---|
| M1 Boundary foundation | DONE | 20 unit tests; clippy clean; flag-OFF no-op |
| M2 OpenClaw provider (ACL) + de-enum | DONE (de-enum → M5) | Real Docker: `oc_calculator→6` via adapter, 0 leaks; boundary grep clean |
| M3 Federated discovery + platform | DONE (CIL-retype → w/ M5) | Real Docker E2E: skills.db→3 descriptors→discovery ranks calculator→exec, 0 leaks |
| M4 Permission + grants + approval UI | DONE | 75 tests incl. Property 7; real Docker effects→gate; live approve→execute→revoke→re-prompt (`capability_approval_flow_docker.rs`), 0 leaks |
| M5 Execution de-enum | CORE DONE (acquisition → M6/M7) | Workspace builds; 3113 tests; real Docker provider-addressed exec via ExecutionEngine, 0 leaks |
| M6 MCP provider + federation | FEDERATION DONE (marketplace/learning pending) | Real Docker+node: 2 providers federated, both execute, 0 leaks |
| M7 Provider SDK + conformance | DONE | Conformance harness; passes for Fake + brand-new provider + both live providers; fails broken provider |
| M8 Observability + recovery + learning | DONE | Event bus (4 real events); circuit breaker; learning-success ranking shift |
| M9 Desktop Capabilities area | DONE (live drive harnessed) | Tabbed Capabilities area (Providers/Browser+Run/Marketplace/Approval Center/Timeline/Descriptor Viewer); `cargo build -p kria-desktop` + `npm run build` clean; `scripts/cpp_tauri_driver_drive.mjs` READY |
| M10 Production validation | ENGINEERING DONE; soak deferred | Diverse battery + live approval test + flag-off drill + `cpp_production_gate.sh` = **GO 3/3, 0 leaks**; soak SOAK TEST READY |
| M11 Debt-removal | TODO (gated on default-on + soak) | — |

## M10 diverse prompt battery evidence

`tests/capability_prompt_battery_docker.rs` (KRIA_CPP_DOCKER=1) — through the CPP platform on real Docker +
real node, 9/9 pass, 0 leaks:
- openclaw/oc_calculator `7*6` → 42
- openclaw/oc_text_tool `upper(Hello)` → HELLO
- openclaw/oc_json_tool `minify` → `{"b":2,"a":1}`
- openclaw/oc_regex_tool `match [0-9]` → ["1","2","3"]
- openclaw/oc_hash_tool `sha256(kria)` → 9b8f38…
- openclaw/oc_csv_tool `to_json` → [{"a":"1","b":"2"}]
- openclaw/oc_markdown_tool `# Title` → `<h1>Title</h1>`
- mcp:stub/reverse_text `kria` → airk
- mcp:stub/word_count `one two three` → 3

## SOAK TEST READY (scope)

The CPP **backend + provider layer is SOAK TEST READY**: boundary, OpenClaw + MCP providers, federation,
discovery, permission + durable grants, de-enumed execution seam, observability, circuit-breaker recovery,
learning loop, and a diverse cross-provider real-execution battery are all implemented + real-validated,
build/clippy/fmt clean, 0 container leaks. The desktop is build-validated (embedded Capabilities UI).

NOT yet soak-ready (remaining implementation, next session): acquisition-via-LIFECYCLE (OpenClaw
`acquire`/`remove` wired to BundleInstaller/A9 — marketplace reachable), marketplace catalog offline
federation into the index, live approval-flow modal + tauri-driver desktop drive (M9 C / M4-UI), then M11
legacy removal after default-on. These are real remaining features, not soak.

## M9 Batch A evidence

`kria-desktop/src/commands/capability.rs` adds `cpp_status`, `cpp_list_providers`, `cpp_discover`,
`cpp_catalog` — backed by a cached `CapabilityPlatform` built from live app state (OpenClaw provider from
`skill_registry`+`container_pool`) + config-declared providers (`[capability].providers`). Registered in
`main.rs`. `cargo build -p kria-desktop` → clean.

Batch B: `ui/src/views/CapabilitiesView.tsx` — first-class Capabilities nav area (Provider Manager +
Capability Browser + discovery search + elevated markers), wired to the `cpp_*` commands; `App.tsx` route +
nav button added. `npm run build` (tsc + vite) → clean. Workspace `cargo build` + `cargo fmt` clean.

## Batch 3 evidence (this session)

- **M5 acquisition (LIFECYCLE):** `OpenClawProvider::with_lifecycle` + `acquire`/`remove` via frozen
  `ClawHubClient`+`BundleInstaller`. Real: installed `oc_code_sandbox` from live `ObaidGits/kria-skills`
  → refreshed descriptor → registry-present → removed (`tests/capability_acquire_marketplace.rs`). Wired
  into desktop `commands/capability.rs` platform builder.
- **Task 11.2 real-LLM A9 generation:** with the live cloud LLM (`opencode`/`deepseek-v4-flash-free`) the
  real `GenerationPipeline`+`BundleInstaller` generated + installed 3 skills (`oc_word_count`,
  `oc_reverse_string`, `oc_celsius_to_fahrenheit_converter`, quality ~0.99); the generated `oc_word_count`
  then **executed in a real container** → `{"wordCount":5}`, 0 leaks
  (`kria-eval a9_cloud_generation::task26_*`).
- **Release artifacts:** `OPENCLAW_FEATURE_MATRIX.md` + `OPENCLAW_RELEASE_CHECKLIST.md` generated from real
  evidence.

## Genuine remaining constraint

M10's Production DoD mandates a **4–8 hour soak** + a 100+ diverse real-prompt battery. The soak is
wall-clock-bound and cannot be produced in a single working session. M11 (legacy deletion) is gated on M10
being green + default-on + soaked, so it cannot proceed earlier without removing the rollback safety net.
Everything up to and including M9 Batch A is implemented + build/real-validated. The remaining work is the
SolidJS Capabilities UX (M9 B/C, large but doable), then the time-bound soak (M10), then cleanup (M11).

## Architecture facts (implemented)

- Neutral boundary module: `crates/kria-core/src/capability/` — `provider` (trait),
  `descriptor` (v1.1 + Guidance/Expectations), `protocol` (negotiation + FeatureSet),
  `state` (capability + provider machines), `error` (CapError), `config`
  (`[capability]`, flag default OFF), `index` (Embedder/FederatedIndex/fusion),
  `registry` (ProviderRegistry), `platform` (CapabilityPlatform), `permission`
  (effects-driven engine), `grants` (durable SQLite GrantStore).
- ACL adapters (only place provider-native types live): `capability/acl/openclaw.rs`
  (`OpenClawProvider`), `capability/acl/mcp.rs` (`McpProvider`).
- Execution seam de-enum: `execution::ExecutorKind`/`ExecutorKindTag`/
  `ExecutorKindTagRecovery` REMOVED; open `provider_id: String` everywhere
  (`Executor::provider_id()`, String-keyed `ExecutorRegistry`,
  `NodeKind::Skill{provider_id,..}`, `PlanStep.provider_id`,
  `RecoveryAction::AlternateExecutor{provider_id}`).

## Validation evidence (real)

- M2: `cargo test -p kria-core --test capability_openclaw_provider_docker` (KRIA_CPP_DOCKER=1)
  → `oc_calculator 3+3 → {"result":6}`, 0 leaked containers.
- M3: `cargo test -p kria-core --test capability_platform_e2e_docker` → real skills.db
  (3 skills) federated, discovery ranks `oc_calculator` top (score 0.199 vs negatives),
  executed → 6; permission gate over real descriptors (calculator NeverAsk, web_* Prompt); 0 leaks.
- M5: `cargo test -p kria-eval openclaw_executor_real_docker_end_to_end` → provider_id="openclaw"
  Skill node dispatched through frozen ExecutionEngine → executed, 0 leaks. Unit: execution 19,
  capability 68, cil 199, handler 8, all green.
- M6: `cargo test -p kria-core --test capability_mcp_federation_docker` → OpenClaw(Docker) +
  MCP(node stub) federated (5 descriptors / 2 healthy); discovery routes by goal; MCP
  `reverse_text("capability")→"ytilibapac"`, OpenClaw `12*12→144`; 0 leaks.

## Environment

- LLM: `~/.kria/bin/llama-server` (Qwen3VL-4B :8080, ngl=27) recovered + chat inference verified.
- Docker: up; `kria/openclaw-substrate:latest` present. node v24.
- Embeddings: no ONNX model in `~/.kria/models/embeddings/` → `MemoryEmbedder` hash fallback
  (identical code path; lexical fusion carries ranking).

## Design decisions / justified deviations

1. **De-enum sequenced into M5, not M2** — the open `provider_id` seam has no consumer until
   provider-addressed execution; doing it in M2 would be churn + double-rewrite the serialized
   graph. Executed atomically in M5. End state identical.
2. **Executor registry keyed by open `provider_id` string** rather than a monolithic
   ProviderExecutor — each provider registers its executor under its open id; removes the closed
   enum (R1.3) with minimal churn. OpenClawExecutor retained (wraps SkillRuntime).
3. **Legacy coupling removal + flag-off path deletion deferred to M11** — those feed the still-live
   OpenClaw path; deletable only after CPP is default-on + soaked (matches migration philosophy).
4. **Federated index in-memory (rebuildable)** — durable descriptor/session caching is M8's concern;
   in-memory rebuild-from-describe() is correct (idempotent), not a second source of truth.

## Bugs found + fixed

- Test-hygiene container leak (M2): validation test dropped the pool without `shutdown()` → 6 warm
  containers leaked. Fixed: tests call `pool.shutdown()`; leak baseline re-verified 0.

## Known pre-existing issues (not CPP)

- `agent::continuation_reentry::tests::duplicate_continuation_is_rejected` — flaky under full-parallel
  test run (timing); passes in isolation. Documented in original SESSION_HANDOFF; untouched by CPP.
- Repo has ~233 pre-existing clippy warnings + a non-semver `#[deprecated(since="batch-1")]` in
  `automation/workflows.rs` (an M11 cleanup target).

## Batch 4 evidence (this session) — M4 approval UI, M9 Batch C, M10 engineering

- **M4 approval UI + gated execution (default-on-ready path):** `kria-desktop/src/commands/capability.rs`
  gained the durable-grant + permission surface — `cpp_authorize` (descriptor→decision, no execute),
  `cpp_approve` (persist scoped Allow/Deny), permission-gated `cpp_execute` (authorize → on Allow run through
  the platform; on Prompt return `needs_approval` with the decision for the modal; on Deny return `denied`),
  `cpp_list_grants`, `cpp_revoke_grant`. Backed by a process-cached `CppState` holding the
  `CapabilityPlatform`, a durable on-disk `GrantStore` (`~/.kria/cpp_grants.db`), a `DefaultPermissionEngine`,
  and a bounded event ring. Added `CapabilityPlatform::descriptor(provider_id, capability_id)` for the
  permission/viewer lookup.
- **M9 Batch C surfaces:** `ui/src/views/CapabilitiesView.tsx` rewritten as a tabbed area — Providers,
  Browser (inline Run with a JSON args editor + Descriptor Viewer modal), Marketplace (installable
  recommendations via `cpp_recommend`), Approval Center (grant list + revoke + the live approval modal with a
  scope picker and standing-deny), and Timeline (the `cpp_timeline` event feed, doubling as Runtime Monitor +
  Recovery via `recover`/`failure` stages). New desktop commands `cpp_recommend`, `cpp_descriptor`,
  `cpp_timeline` (the ring subscribes to a `CapabilityEventBus` attached to the platform). `cargo build
  -p kria-desktop` + `npm run build` clean.
- **M4 real validation:** `tests/capability_approval_flow_docker.rs` (KRIA_CPP_DOCKER=1) — (A) real Docker
  calculator gate(NeverAsk)→execute→6; (B) elevated (network) descriptor: first-use Prompt(AskPerSession) →
  approve(session) → silent reuse (same grant id) → **survives GrantStore reopen** (desktop-restart
  durability) → revoke → re-prompt. 0 leaked containers.
- **M10 DoD automation:** `scripts/cpp_production_gate.sh` runs the real gated validations (M4 approval,
  M6 federation, M10 battery), enforces the 0-leak discipline, and writes
  `PRODUCTION_GATE_REPORT.md`. **Latest run: GO — 3/3 pass, 0 leaks.** `scripts/cpp_soak.sh` (wall-clock soak)
  and `scripts/cpp_tauri_driver_drive.mjs` (live desktop drive) are prepared and READY FOR EXECUTION.

## Milestone status after Batch 4

M1–M9 fully done (9/11) + task 11.2. M10 engineering complete (only the wall-clock soak remains, which gates
the default-on flip). M11 gated on default-on + soak. The CPP is **SOAK TEST READY end-to-end** (backend +
providers + acquisition + generation + permission/approval + full desktop surfaces). The only remaining work
is release validation: the multi-hour soak, a 100+ manual prompt campaign, and live UX truthfulness — all
harnessed and READY FOR EXECUTION.

## M12 — Single-pipeline migration (Option A, LOCKED)

Decision LOCKED: KRIA collapses onto ONE execution architecture = the Capability Provider Platform. The
legacy chat pipeline (`SemanticOpenClawHandler` → `SemanticSkillRouter` → `ExecutionEngine` →
`ApprovalCache`/`openclaw::perm`) is being retired, not preserved. No backward-compat.

Investigation findings (evidence): chat executed through `SemanticOpenClawHandler::execute_semantic`
(registered as the `openclaw` tool in `kria-desktop/commands/runtime.rs`), which carried its OWN permission
engine (`openclaw::perm`) + grant store + CIL gated by `openclaw.cil.openclaw_icp_enabled` (default OFF) and
a default IN-MEMORY grant store — so grants never persisted and re-prompted, and CPP (`capability::*`) was a
parallel path used only by the `cpp_*` desktop commands. That divergence is exactly the "two systems" bug.

- **Stage 1 DONE (build+tests):** `crate::tools::capability_dispatch::CapabilityDispatchHandler` — the single
  CPP-backed chat/agent dispatcher (discover → one permission engine + one durable grant store → provider
  execute; NL→typed args via `openclaw::arg_gen`). Lives under `tools/` so it may bridge tools↔capability
  without breaking the `capability/` boundary gate. 2 unit tests green.
- **Stage 2 DONE (build):** `runtime.rs` now registers the dispatcher for the `openclaw` tool, overwriting the
  legacy handler by name (keeps `list_installed_skills`). Reuses the shared embedding model + OpenClaw
  provider (marketplace lifecycle) + durable `cpp_grants.db`. `cargo build -p kria-desktop` clean. Chat
  capability execution now flows through `CapabilityPlatform`.
- **Remaining (Stages 3–6):** marketplace UI → remote provider catalog; inline approval-modal round-trip on
  the chat path; delete legacy (`SemanticOpenClawHandler` routing, `SemanticSkillRouter`, `ApprovalCache`,
  `openclaw::perm`, legacy CIL discovery, `openclaw_icp_enabled`) once callers are gone; default-on + real
  desktop drive + restart persistence. Each stage build-green before the next.

### M12 Stage 5 progress (legacy deletion)

- **Batch A DONE:** legacy handler unwired from production (chat + list on CPP).
- **Batch B DONE:** `crates/kria-core/src/openclaw/handler.rs` **deleted** in full. `build_runtime_registry`
  relocated to `openclaw::runtime` (neutral runtime infra, still used by CPP + real-Docker eval suites); all
  imports updated. The 3 legacy `OpenClawSubsystem::register_into_tool_registry*` methods deleted. Legacy
  handler tests removed (`execute_e2e::r4_4_fixed_*`, `trust_revocation`'s community-network + source-tripwire
  tests, `openclaw_live_docker`'s NL→calculator e2e). `build_cil_facade` is now dead (deleted with `cil` in
  Batch F). Evidence: `cargo test -p kria-core -p kria-eval --no-run` compiles all binaries; `cargo build -p
  kria-desktop` clean; `tools::capability_dispatch` 2/2 tests green.
- **Batch D DONE:** `openclaw::perm` (grant_store + engine) **deleted** — the duplicate grant store + permission
  engine are gone. Desktop `openclaw_list_grants`/`openclaw_revoke_grant` migrated to `capability::grants` +
  `capability::permission` over the ONE `cpp_grants.db` (shared with the chat dispatcher + Capabilities panel).
  Evidence: `cargo build -p kria-core -p kria-desktop` clean; `cargo test -p kria-core -p kria-eval --no-run`
  compiles all binaries. (cil doc-links to `perm::grant_store` remain — they vanish with cil in Batch F.)
- **Remaining:** Batch F delete `openclaw::cil` + `build_cil_facade` + `openclaw_icp_enabled` + the desktop cil
  commands (recommendations/capability_manager/capability_graph) — this frees `SemanticSkillRouter` (used only
  by `cil::learn`); Batch C then deletes `SemanticSkillRouter` (+ eval routing tests); Batch E reduces
  `ApprovalCache` to the `compute_hash` util `runtime/docker.rs` needs (inline), deletes the rest.

### M12 Stage 5 COMPLETE — legacy execution architecture removed

Batches A–F all done, tree green (kria-core + kria-desktop + kria-eval, lib + tests):
- **A/B:** `SemanticOpenClawHandler` + `handler.rs` deleted; chat served by `CapabilityDispatchHandler`;
  `build_runtime_registry` relocated to `openclaw::runtime`.
- **D:** `openclaw::perm` deleted; ONE grant store + ONE permission engine (`capability::grants`/`permission`);
  desktop grant commands migrated to `cpp_grants.db`.
- **F:** `openclaw::cil` (whole module) + `build_cil_facade` + `OpenClawConfig.cil` + `openclaw_icp_enabled`
  flag + cil re-exports deleted. Desktop cil commands rewritten cil-free (delegate to CPP / registry-only).
  Eval cil suites removed/trimmed (kept `honesty_ledger`).
- **C:** `SemanticSkillRouter` + `semantic_router.rs`/tests deleted; eval routing tests migrated to
  registry-based checks or removed.
- **E:** `ApprovalCache` deleted; reusable `compute_hash` extracted to `openclaw::cap_hash`; `docker.rs`
  updated. Only `capability::permission` + `capability::grants` remain.

**Evidence:** grep of `crates/*/src` for `SemanticOpenClawHandler|SemanticSkillRouter|openclaw::perm|
openclaw::cil|ApprovalCache|openclaw_icp_enabled` → ZERO production references. `cargo test -p kria-core
-p kria-eval --no-run` compiles all binaries; `cargo build -p kria-desktop` clean; 110 capability lib tests
pass; `cargo fmt` clean.

**Final architecture:** User → CapabilityDispatchHandler → CapabilityPlatform → ProviderRegistry →
{OpenClawProvider, McpProvider} → execution. One permission engine, one grant store, one provider registry,
one discovery index, one dispatcher. No duplicate systems, no legacy execution path, no feature flags
protecting dead code.

### E2E validation session (post-migration)

Drove the REAL chat entry (`CapabilityDispatchHandler`) with 18 diverse prompts on real Docker
(`tests/capability_e2e_dispatch_docker.rs`): **PASS 18 / FAIL 0**, 0 container leaks, avg ~52ms.
Categories: arithmetic, hashing (sha256/md5), JSON (minify/pretty), regex, CSV, markdown, string
(upper/lower), gzip, + negatives (unknown/malformed/empty) + permission-gate + grant-reuse.

Two real dispatcher bugs found + fixed: (1) no relevance floor → mis-executed the top hit for
irrelevant queries; (2) mis-routing among similar utility skills under the hash-fallback embedder.
Both fixed by an overlap-based re-rank + relevance floor in `tools::capability_dispatch` (prefix-token
matching; irrelevant/unknown → honest no-match). 110 capability lib tests still green; desktop builds;
fmt clean. Full report: `E2E_VALIDATION_REPORT.md`. Known prerequisite: install the ONNX embedding model
for full semantic chat routing (hash fallback routes by lexical keyword only).

### Final engineering validation (embedding model + E2E campaign + ClawHub lifecycle)

- **Embedding model installed:** all-MiniLM-L6-v2 ONNX (90MB) + tokenizer.json at
  `~/.kria/models/embeddings/`. Fixed a real bug: `embeddings.rs` used a placeholder hash tokenizer →
  wired the real `tokenizers` WordPiece tokenizer; ONNX path requires model+tokenizer. Hash fallback off.
  Semantic clustering verified (`embedding_semantic_validation.rs`): intra 0.49–0.73 vs cross 0.004.
- **E2E campaign:** `capability_e2e_dispatch_docker.rs` — 24 diverse prompts through the real
  `CapabilityDispatchHandler` (OpenClaw + MCP), **24/24 PASS, 0 leaks, avg 56ms**. Fixed a real permission
  bug: `Unknown` reversibility was over-classified as AlwaysAsk → thin-provider (MCP) tools couldn't remember
  a grant; refined so only explicit-irreversible/host-subprocess is AlwaysAsk. 110 capability lib tests green.
- **ClawHub lifecycle:** acquire→describe→remove on real marketplace (`capability_acquire_marketplace.rs`),
  0 leaks. Install-battery (30 skills) reused from `capability_prompt_report_docker.rs`.
- Full report: `FINAL_VALIDATION_REPORT.md`. Verdict: **GO** (remaining = soak / manual UX / substrate handlers).


## Real-user debugging session (2026-07-07): frontend↔CPP divergences

Verified on the REAL live path (`POST /api/chat` = same pipeline as desktop `send_message`),
real Docker + orchestrator LLM + real ClawHub marketplace.

Fixed (build-green; unit + live-verified):
- n8n pre-fallback weak-match hijack → `route_chat` releases low-confidence/non-explicit matches
  to normal routing (`n8n/matching.rs`).
- Exposed CPP marketplace to the agent: `search_marketplace` + `install_capability` tools +
  `CapabilityPlatform::acquire_for_goal` (no hardcoded skill names).
- OS package-flow no longer hijacks skill/tool/capability installs (`loop_engine/helpers.rs`);
  narrowed `search_package`/`install_package` descriptions to OS-only (`tools/packages.rs`).
- Deterministic `CapabilityFlowState` safety net forces the correct marketplace tool when the
  small model hedges (`loop_engine/mod.rs`).

Live proof: "Install the IP Info tool from the marketplace" installs end-to-end (was the exact
chat-export failure); no n8n hijack; no max-rounds loop (90s→8s); 0 container leaks.

Known remaining (out of CPP scope): compress/PDF/zip/websearch skills are genuinely absent from
the marketplace repo (substrate content gap — author skills to close); small-model summary/hedge
quality.
