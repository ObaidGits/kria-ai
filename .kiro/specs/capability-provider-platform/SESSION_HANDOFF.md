# CPP Session Handoff

Continuation notes so a new session can resume immediately. Source of truth:
`requirements.md`, `design.md`, `tasks.md`, `PROGRESS.md`.

## Where we are

M1–M6 core implemented + real-validated (see PROGRESS.md evidence). The
provider-neutral boundary, OpenClaw ACL, federated discovery, effects-driven
permission + durable grants, the full `ExecutorKind`→`provider_id` de-enum, and
multi-provider federation (OpenClaw + MCP) are DONE and validated on real Docker
+ real node + real LLM. Workspace builds clean; clippy clean on CPP code.

## How to run the real validations

```bash
# LLM (if down): start it first
~/.kria/bin/llama-server -m ~/.kria/models/llm/Qwen3VL-4B-Instruct-Q4_K_M.gguf \
  --host 127.0.0.1 --port 8080 -ngl 27 -c 8192 &

# Unit suites
cargo test -p kria-core --lib capability::
cargo test -p kria-core --lib execution::

# Real Docker integration (needs Docker + kria/openclaw-substrate:latest + ~/.kria/skills.db + node)
KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_openclaw_provider_docker -- --nocapture
KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_platform_e2e_docker -- --nocapture
KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_mcp_federation_docker -- --nocapture

# Always verify 0 leaks after Docker runs:
docker ps -aq --filter "name=kria-openclaw" | wc -l   # must be 0
```

## DONE since last handoff

- **M6 federation:** `McpProvider` + minimal MCP stub; OpenClaw+MCP federated + both execute (real).
- **M7 SDK/conformance:** `capability::conformance::run_conformance` — passes for Fake + a brand-new
  from-scratch provider (Property 2) + both live providers; fails a broken provider. De-fragmentation
  assessed as a no-op (the other `capability_registry` modules are distinct domains, not duplicates).
- **M8 observability + recovery:** `capability::events` bus (validated on real path) + per-provider
  circuit breaker in `ProviderRegistry`. Caching = the in-memory index (invalidated via upsert/rebuild);
  ResourceBroker intentionally NOT built (runtime already owns HRA admission — no second scheduler).

## DONE since last handoff (batch 2)

- **M8 learning loop:** `InMemoryFederatedIndex` outcome stats + success signal in fusion; validated.
- **M9 Batch A+B:** desktop `cpp_*` commands + SolidJS `CapabilitiesView` + nav route; `cargo build
  -p kria-desktop`, `npm run build`, and full `cargo tauri build --debug --no-bundle` (embedded UI) all clean.
- **M10 diverse battery:** `tests/capability_prompt_battery_docker.rs` — 9/9 real skills across 2 providers
  on Docker, 0 leaks (calculator/text/json/regex/hash/csv/markdown + MCP reverse/word_count).

## DONE since last handoff (batch 4 — M4 UI, M9 Batch C, M10 engineering)

- **M4 approval UI + gated execute:** desktop `cpp_authorize`/`cpp_approve`/`cpp_execute`/`cpp_list_grants`/
  `cpp_revoke_grant` over a durable on-disk `GrantStore` + `DefaultPermissionEngine`; SolidJS Approval Center
  + approval modal + inline Run. Live approve→execute→revoke→re-prompt validated on real Docker
  (`tests/capability_approval_flow_docker.rs`, incl. GrantStore-reopen durability), 0 leaks.
- **M9 Batch C:** tabbed `CapabilitiesView` (Providers / Browser+Run / Marketplace / Approval Center /
  Timeline+Runtime+Recovery / Descriptor Viewer) + `cpp_recommend`/`cpp_descriptor`/`cpp_timeline`
  (event-bus ring). Build clean. `scripts/cpp_tauri_driver_drive.mjs` READY.
- **M10 engineering:** `scripts/cpp_production_gate.sh` = GO (3/3, 0 leaks, `PRODUCTION_GATE_REPORT.md`);
  flag-off rollback drill covered by config default test; `scripts/cpp_soak.sh` SOAK TEST READY.

## M12 — Single-pipeline migration (Option A, LOCKED) — resume here

Goal: ONE architecture (CPP). Retire legacy chat/permission/marketplace. No backward compat.

DONE this session:
- Stage 1: `crate::tools::capability_dispatch::CapabilityDispatchHandler` (CPP chat dispatcher; one permission
  engine + one durable grant store; NL→args via `openclaw::arg_gen`). 2 unit tests green.
- Stage 2: `kria-desktop/commands/runtime.rs` registers it for the `openclaw` tool (overwrites legacy handler
  by name; `list_installed_skills` kept). `cargo build -p kria-desktop` clean → chat now executes through CPP.

RESUME (in order, build-green each step):
1. Stage 3 — marketplace UI → remote catalog (`cpp_recommend`/`clawhub_fetch_remote_skills`); installed = separate view.
2. Stage 4 — inline approval modal round-trip on the chat path (Prompt → modal → durable grant → silent reuse).
3. Stage 5 — DELETE legacy once no caller remains: `SemanticOpenClawHandler` routing, `SemanticSkillRouter`,
   `ApprovalCache` (keep only genuinely-needed hashing), `openclaw::perm` (fold into `capability::grants`/
   `permission` — ONE grant store), legacy CIL discovery/recommend, `openclaw_icp_enabled` flag. Run the
   boundary + single-owner grep gates. Fix all references (docker.rs cap_hash uses ApprovalCache::compute_hash
   — move that hashing util or inline it).
4. Stage 6 — flip `[capability].enabled` default on; real desktop drive (chat/marketplace/permission);
   verify grant persistence across restart; 0 container leaks.

Key seam facts: chat tool name is `openclaw` (kept for the agent contract); `ToolRegistry::register` overwrites
by name; `SkillRegistry = ProductionSkillRegistry`; dispatcher is under `tools/` (not `capability/`) to bridge
without breaking the boundary gate.

### Stage 5 status + the deletion cascade (do surgically)
- **Batch A DONE:** `runtime.rs` no longer calls `register_into_tool_registry(_with_cil)`. Both init methods
  now have ZERO production callers (grep-confirmed). `openclaw` + `list_installed_skills` are served by
  `CapabilityDispatchHandler` + `CapabilityListHandler`. Desktop builds clean. Legacy is now dead/unwired.
- **CRITICAL for Batch B:** `openclaw::handler::build_runtime_registry` is NOT pure legacy — it's used by many
  still-valid real-Docker eval tests (`leak_freedom`, `a9_cloud_generation`, `execute_e2e` mount test,
  `openclaw_live_docker` runtime tests) that drive `DockerRuntime` directly (valid under CPP). **Preserve it**
  — relocate to `openclaw/runtime/mod.rs` (or keep a slim module), then delete the rest of `handler.rs`.
- **Batch B DONE:** `handler.rs` fully deleted; `build_runtime_registry` relocated to `openclaw::runtime`;
  3 legacy init registration methods deleted; legacy-handler tests removed. All three crates compile (lib +
  tests). `build_cil_facade` is now dead (delete with cil in Batch F).
- **Batch D DONE:** `openclaw::perm` deleted; desktop grant list/revoke commands migrated to
  `capability::grants` + `capability::permission` over `cpp_grants.db` (helper `cpp_grants_db_path()` in
  `commands/openclaw.rs`). ONE grant store, ONE permission engine now. Green (lib + tests + desktop).
### STATE FOR NEW SESSION (current)

**Migration + hardening COMPLETE. One CPP pipeline. All crates + tests green. Verdict: GO.**

Done (this + prior sessions):
- M1–M12 done: single execution path (chat → `CapabilityDispatchHandler` → `CapabilityPlatform` →
  ProviderRegistry → {OpenClawProvider, McpProvider}). Legacy fully deleted (SemanticOpenClawHandler,
  SemanticSkillRouter, openclaw::perm, openclaw::cil, ApprovalCache→`cap_hash` util, openclaw_icp_enabled).
  Grep-verified ZERO legacy production refs.
- ONE permission engine (`capability::permission`) + ONE grant store (`capability::grants`, `cpp_grants.db`).
- **Embedding model installed:** all-MiniLM-L6-v2 ONNX + tokenizer.json at `~/.kria/models/embeddings/`.
  Real WordPiece tokenizer wired (was a placeholder hash tokenizer). Hash fallback off. Semantic clustering
  verified (`embedding_semantic_validation.rs`).
- **Permission fix:** `Unknown` reversibility no longer forced to AlwaysAsk — thin-provider (MCP) grants persist.
- E2E: `capability_e2e_dispatch_docker.rs` 24/24 PASS (OpenClaw+MCP, real Docker), 0 leaks, avg 56ms.
- ClawHub acquire→describe→remove verified on real marketplace.
- Reports: `FINAL_VALIDATION_REPORT.md`, `E2E_VALIDATION_REPORT.md`, `PROGRESS.md`.

Remaining (NON-engineering / release validation only):
- Multi-hour soak (`scripts/cpp_soak.sh`), 100+ manual prompt campaign, live desktop UX drive
  (`scripts/cpp_tauri_driver_drive.mjs`), flip `[capability].enabled` default-on after soak green.
- OpenClaw substrate: bake execution handlers for the new marketplace skills (word_counter, base64_tool, …) —
  they install/discover/gate but decline at execution until handlers exist (substrate scope, not CPP).

Key real-validation commands:
```bash
KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_e2e_dispatch_docker -- --nocapture
cargo test -p kria-core --test embedding_semantic_validation -- --nocapture
KRIA_CPP_DOCKER=1 KRIA_CPP_NET=1 cargo test -p kria-core --test capability_acquire_marketplace -- --nocapture
docker ps -aq --filter "name=kria-openclaw" | wc -l   # must be 0
```

--- historical handoff below ---

- **Batch F (done):** delete `openclaw::cil` (whole module, ~15 files) + `init::build_cil_facade`
  (already dead) + the `openclaw_icp_enabled` flag + the `OpenClawConfig.cil` field + `openclaw::mod` re-exports
  (`CilConfig, DegradedState, RankWeights`). Migrate/delete the desktop cil commands
  (`openclaw_recommend_skills`, `openclaw_capability_manager`, `openclaw_capability_graph`) — recommendations
  should come from `cpp_recommend`/`CapabilityPlatform`. Delete the eval cil suites
  (`telemetry_completeness`, `honesty_sweep`). This frees `SemanticSkillRouter` (used only by `cil::learn`).
- **Batch C:** then delete `SemanticSkillRouter` + `semantic_router.rs`/`semantic_router_tests.rs`; fix/delete
  eval routing tests (`skill_management`, `performance_budgets`, `execute_e2e::r4_no_match`).
- **Batch E:** reduce `ApprovalCache` to just the `compute_hash` util `runtime/docker.rs` uses (inline it),
  delete the rest.
- **Batch F:** delete `openclaw_icp_enabled` flag + `openclaw::cil` module + `init::build_cil_facade`.
- Each batch: build kria-core + kria-desktop green before the next; expect legacy tests to delete.

## Exact next-batch items (release validation + M11)

1. **Soak (release validation):** `SOAK_HOURS=6 scripts/cpp_soak.sh` — the wall-clock gate before default-on.
2. **Default-on flip:** after soak green, set `[capability].enabled = true` default (still flag-reversible)
   and wire `CapabilityPlatform` into the agent chat path (the `cpp_execute` gate is the reference flow).
3. **Live desktop drive:** `DISPLAY=:1 node scripts/cpp_tauri_driver_drive.mjs` against the built binary.
4. **M11 legacy removal:** ONLY after default-on + soak — delete the flag-off direct-router path, legacy
   `openclaw::handler::register_skill`, legacy CIL/openclaw grant engine, `ExecutorKind` serde-compat, dead
   fields; consolidate one owner per concern.

## Remaining work (in order)

- **M6 finish (optional polish):** marketplace catalog federation into the index; feed
  per-`(provider_id,capability_id)` learning stats into the ranker fusion.
- **M9 Desktop Capabilities area:** tauri-driver + WebKitWebDriver + cargo-tauri ARE installed.
  Wire a `CapabilityPlatform` into `kria-desktop` boot (behind the flag), add provider-neutral Tauri
  commands (list providers/sessions/health, discover, catalog, grants list/revoke, event stream),
  elevate the 4 existing views (CapabilityManager/Graph/ExecutionLogs/PermissionManager) into a
  first-class Capabilities nav area, add Provider Manager / Approval Center / Timeline / Runtime Monitor
  / Recovery / Descriptor Viewer. Validate with `cargo tauri build --debug --no-bundle` + tauri-driver.
- **M10 Production DoD:** real prompt battery (diverse, not repetitive) + soak +
  budgets + flag-off rollback drill; then promote CPP default-on.
- **M11 Debt-removal:** ONLY after M10 default-on + soak — delete flag-off
  direct-router path, legacy `openclaw::handler::register_skill`, legacy CIL/
  openclaw grant engine, reserved dead_code fields; consolidate one owner per concern.

## Key invariants (do not break)

- No provider-native type outside `capability/acl/*` (grep gate:
  `grep -rn "crate::openclaw\|mcp::client" crates/kria-core/src/capability/ | grep -v /acl/` → empty).
- Provider identity is an open string everywhere (no enums).
- Each provider owns its catalog; federated index is derived/rebuildable.
- Everything gated behind `[capability] enabled` (default OFF); flag-OFF = current behavior.
- No fake success; honest degrade; 0 container leaks after every run.

## Gotchas

- No ONNX embedding model installed → hash-fallback embeddings (ranking still correct via
  lexical fusion). Install `~/.kria/models/embeddings/all-MiniLM-L6-v2.onnx` for dense quality.
- llama-server `/v1/embeddings` returns pooling error — irrelevant; CPP embeds via fastembed/hash,
  not llama-server.
- Pre-existing flaky test `duplicate_continuation_is_rejected` (passes in isolation) — ignore.
- Always `pool.shutdown()` in any test that builds a ContainerPool, or warm containers leak.


---

## Real-user debugging session (2026-07-07) — frontend↔CPP divergences fixed

Drove the REAL desktop chat pipeline over the live local API (`POST /api/chat`, same
n8n-pre-fallback → agent-loop → tools → CPP path the desktop `send_message` uses). Not
internal-component tests. Live system: kria-desktop (`cargo tauri dev`), real Docker
substrate pool (6 warm containers, 0 leaks), real orchestrator llama-server, real ClawHub
marketplace (`ObaidGits/kria-skills`, ~30 utility skills).

### Root causes found (forensic trace of the uploaded chat)
1. **n8n pre-fallback hijack.** Desktop `send_message` runs `desktop_n8n_pre_fallback_command_capture`
   BEFORE the agent. `WorkflowRankingEngine::route_chat` returned `SuggestWorkflow` on a weak,
   low-confidence, tag-only fuzzy match → "install web fetch tool" was stolen by "Manual Mail
   Fetch". Marketplace/tool/install prompts never reached the agent/CPP.
2. **No CPP marketplace tool on the agent path.** The agent only had OS `search_package`/
   `install_package` + `openclaw` (execute) + `list_installed_skills`. "Install a PDF extractor"
   had nowhere correct to go; CPP marketplace (`cpp_recommend`/acquire) was desktop-only.
3. **OS package-flow hijack + loop.** The agent loop's `PackageFlowState` triggered on any
   "install …" and force-injected `search_package`/`install_package`, competing with the semantic
   index → "max tool rounds (10) reached" (90s hang).
4. **Tool ambiguity.** Embedding router preferred OS `install_package` over a marketplace tool for
   "install … tool", because both said "install".

### Fixes (all build-green; unit + live verified)
- **FIX 1 `crates/kria-core/src/n8n/matching.rs`:** weak-match release — `route_chat` now returns
  `UseOtherTool` when the top candidate is low-confidence (<0.60), non-explicit, matched only on a
  broad field (not id/name/alias/example_prompt), not blocked/hard/ambiguous. Defers to normal
  routing. (27 matching tests green.)
- **FIX 2 marketplace tools:** `CapabilityPlatform::acquire_for_goal` (platform-neutral install:
  best-provider-first, refresh after) + `MarketplaceSearchHandler` (`search_marketplace`) +
  `MarketplaceInstallHandler` (`install_capability`) in `tools/capability_dispatch.rs`, registered
  in `kria-desktop/commands/runtime.rs` sharing the ONE platform. No hardcoded skill names.
- **FIX 3 `agent/loop_engine/helpers.rs`:** `detect_package_intent` now excludes capability/skill/
  tool/plugin/marketplace intents (`refers_to_marketplace_capability`) from the OS package flow;
  added `extract_capability_query`.
- **FIX 4 tool descriptions (`tools/packages.rs`):** narrowed `search_package`/`install_package` to
  "OPERATING-SYSTEM software … NOT KRIA skills/tools/capabilities (use search_marketplace/
  install_capability)" — disambiguates both the embedding router and the LLM.
- **FIX 5 deterministic capability-flow (`agent/loop_engine/mod.rs`):** `CapabilityFlowState`
  (Search/Install), a safety net that forces the correct marketplace tool when a small local model
  hedges ("local or VM?") on a clear "install/search a tool/skill" request. Single forced call
  (install internally searches+acquires), so no multi-round loops. (7 new unit tests green.)

### Live verification (POST /api/chat, real pipeline)
- "install web search tool" → NO n8n hijack (`n8n.action=none`), NO max-rounds loop (was 90s → 8s).
- **"Install the IP Info tool from the marketplace" → ✅ installed end-to-end** (the exact chat-export
  failure) via `install_capability`; then "use IP Info on 8.8.8.8" executed.
- "search the marketplace for a hash tool" → Hash Generator (`oc_hash_generator`). Marketplace search works.
- "Calculate 481*22+7" → 10589 (openclaw). "what tools installed" → backend returns all 8 (LLM sometimes
  under-summarizes — model quality, not backend).
- 0 container leaks (pool steady at 6).

### Remaining (documented; largely out-of-CPP-scope / model- or data-bound)
- **Daily tasks compress/PDF/zip/websearch fail** because those skills are GENUINELY absent from the
  marketplace (`ObaidGits/kria-skills` = format/encode/convert/network utilities only). Honest
  "no matching capability / not in marketplace" is now the correct behavior. **Authoring these skills
  is OpenClaw substrate scope, not CPP.**
- Small local model (Qwen3VL-4B) still occasionally hedges/asks-to-clarify or under-summarizes; the
  capability-flow + description tuning mitigate the install/search case specifically.
- Approval persistence (Issue 4) not reproduced this session (calculator ran promptlessly; grants
  persisted in prior validation). Left as-is; revisit if a concrete re-prompt repro surfaces.

Files touched: `n8n/matching.rs`, `capability/platform.rs`, `tools/capability_dispatch.rs`,
`tools/packages.rs`, `agent/loop_engine/helpers.rs`, `agent/loop_engine/mod.rs`,
`agent/loop_engine/tests.rs`, `kria-desktop/commands/runtime.rs`, `scripts/cpp_live_probe.sh`.
