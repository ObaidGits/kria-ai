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
