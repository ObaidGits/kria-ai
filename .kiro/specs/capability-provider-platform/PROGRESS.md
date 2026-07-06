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
