# Session Handoff — OpenClaw Intelligence Enhancements

> Purpose: let a fresh session resume without re-investigating. Everything below is
> verified against source + real execution (not assumptions). Last updated: 2026-07-09.

## TL;DR state
- **Waves 0–6 backend core + Wave 3.1 arbitration: implemented, wired, test-verified.** Cloud LLM works.
  OpenClaw execution substrate is **proven working** (24/24 real-Docker E2E). No open blocker to capability
  execution. Remaining: Wave 3.3 (HTN adapter runtime cycle), Waves 7–13 (2nd provider, evolution,
  synthesis, continuous discovery, hardening, ≥100-prompt real-UI campaign, gate) + the frontend track.

## Wave 6 — LIVE DESKTOP VALIDATION (real app + real LLM + real OpenClaw + real DB)
- Ran the **real built desktop** (`./target/debug/kria-desktop`, DISPLAY=:1) with local_api on :3001.
- **Config required to activate Wave 6 + a working LLM** (changed in `~/.kria/kria.db` config table):
  `capability.intelligence.marketplace_v2 = true`; `providers.active_provider = "llama_cpp"`,
  `llm.routing_mode = "local"` (cloud opencode free gateway returns 500/403 on realistic tool payloads —
  external-service unreliable; local Qwen3-VL-4B on GPU works). opencode provider `active_model` set to
  `mimo-v2.5-free` (only opencode model that reliably returns tool_calls) for if/when cloud is used.
- Log confirms: `[CPP] Marketplace intelligence v2 wired — neutral catalog ranking + TTL cache (spec R8)`.
- **Live campaign (real prompts via /api/chat → real agent loop → local LLM → tools):** 4 install prompts
  drove `install_capability` end-to-end: LLM tool-call → RED-tier permission (auto-approved owner) →
  `acquire_for_goal` rank → **Decision Record** → real download → **`[CPP] acquired-artifact provenance
  digest recorded`** (ArtifactVerifier/Digest live on real bytes) → trust gate (community→trusted) →
  activate → CKB. Result in the REAL `~/.kria/cpp_knowledge.db`: **4 cpp_knowledge rows** (oc_base64_tool
  [Data], oc_url_codec [Network], oc_hash_generator [Data], oc_regex_extractor [Coding], all Installed→
  enabled) + **4 cpp_decisions** (goal, class, ranked candidates+confidence — real explainability R16).
- **No container leak**: pool stable at 6 warm (2 light/2 medium/2 heavy); installs spawn none.
- **Known limitations (honest, NOT Wave 6 defects):** (a) the local 4B model's tool-selection is imperfect
  — "search the marketplace" sometimes routes to web_search, "what's installed" to fs list; the Wave 6
  tools ARE registered/reachable/working (proven by install_capability executing fully). (b) The real
  marketplace catalog lacks some requested skills (python-sandbox, OCR) so ranking honestly returns low
  scores (~0.37) and picks the closest — a catalog-content gap, not a pipeline bug. (c) opencode free
  cloud gateway is unreliable for tool payloads (external service).

## Wave 6 — PRODUCTION VALIDATION (real infra) + extra root-cause fixes
- **Real marketplace E2E** (`tests/capability_wave6_pipeline.rs`, gated `KRIA_CPP_NET=1`): drives the
  REAL OpenClaw provider against the REAL index (`ObaidGits/kria-skills`) through the real
  `CapabilityPlatform` (marketplace_v2 + real SQLite CKB + real event bus). Proven live:
  recommend ranks real catalog via ClawHub schema → Brain selects `oc_code_sandbox` → acquires the
  chosen capability → trust gate → **CKB records install + success outcome** → **Rank/Acquire events
  fire** → installed capability is **not re-recommended**. Second test: a strict `TrustPolicy`
  (min tier ≥ trusted) **quarantines the real community skill**, blocks acquisition + `execute`, and
  `release_quarantine` clears it. Both pass.
- **Extra root-cause fixes found during validation (not symptom patches):**
  1. Catalog cache not invalidated after install → `acquire_for_goal_reasoned` now calls
     `invalidate_catalog_cache(None)` post-activation.
  2. `catalog()` marked everything installed=false → now reflects real registry state; `recommend`
     filters out already-installed entries (native-first, no duplicate install).
  3. `openclaw/registry.rs::is_version_compatible` was an **always-true TODO stub** on the real
     `upgrade_skill` path → replaced with real `semver` (valid + differing versions), matching the
     single-source-of-truth semver rule.
- **Not verified here (honest, environment-bound):** live Desktop GUI + LLM `Prompt→Response` ≥100-prompt
  campaign — needs a running desktop session; that is Wave 12 scope. Backend + real acquisition/marketplace/
  events/CKB/trust/quarantine are validated against real infra.

## Wave 6 — GENUINELY integrated (post-audit remediation)
- **Ownership moved to Brain:** `AcquireRequest.capability_id` (provider.rs) — Brain selects the specific
  capability; `acl/openclaw.rs::acquire` honors it exactly (no provider re-resolution).
- **Brain-owned acquisition pipeline** in `platform.rs::acquire_for_goal_reasoned` (flag-on): rank →
  Decision Record (CKB) → dependency check (`DependencySpec`/`version_satisfies`) → acquire chosen →
  **trust gate** (`TrustPolicy`/`TrustVerdict`) → **Quarantine** on fail (blocks activation) → CKB
  install+outcome → activate; emits Rank/Acquire/Failure/Learn events. Flag-off ⇒ byte-identical legacy.
- **Quarantine enforced in `execute()`** (quarantined ⇒ refused). Surfaced via `cpp_quarantined`/
  `cpp_release_quarantine` + a Quarantine tab in `CapabilitiesView.tsx`.
- **ArtifactVerifier/Digest live:** provenance digest computed over real downloaded manifest bytes in
  `acl/openclaw.rs::acquire`, recorded to descriptor `extensions["artifact_sha256"]`.
- **Duplicates killed:** `openclaw/registry.rs::version_satisfies` TODO stub → neutral semver (single
  source); neutral `is_breaking_change` removed. **ClawHub single model:** `catalog()` maps the remote
  index through the neutral `ClawHubListing`/`PublishedVersion` (OpenClaw client = transport).
- Verified: `capability::` lib 134 pass (incl. 3 Brain-pipeline integration tests + neutrality + parity),
  `openclaw::` 113 pass, both crates check clean, `ui` build clean, real-Docker dispatch E2E passes.
- **Honest remainder (NOT backend gaps):** live ≥100-prompt Desktop campaign (needs running desktop);
  richer ClawHub backend fields (reviews/ratings/signatures) await a real ClawHub index that advertises
  them — the schema + wiring are ready.

## NEW earlier this session (Wave 6 initial + 3.1)
- **Wave 6 marketplace intelligence** — `intelligence/marketplace.rs` (all neutral):
  `CatalogRanker`+`CatalogRankingPolicy` (6.1: trust/quality/cost/adoption+relevance fusion),
  `ArtifactVerifier`+`Quarantine` (6.2: sha256/blake3 hash + ed25519 signature + quarantine-on-fail),
  `CatalogCache` (6.3: per-provider TTL + explicit invalidate), `CapabilityCoordinate`/`DependencySpec`/
  `version_satisfies`/`is_breaking_change` (6.4: namespacing + semver + deps), `ClawHubListing` schema
  (6.5). Wired into `platform.rs::recommend` behind `marketplace_v2` flag (+`with_marketplace_v2`,
  `gather_catalog`, `invalidate_catalog_cache`); runtime.rs wires it when the flag is on. 13 tests.
- **Wave 3.1 planning authority** — `intelligence/arbitration.rs::PlanningAuthority` (neutral
  `PlanningDomain` roles; sticky explicit intent; prior-owner continuity nudge; risk tie-break; abstain;
  JSON `ArbitrationDecision` trace). Exactly-one-winner. 8 tests. Agent-loop router wiring behind
  `routing_gate` is the remaining integration step (pairs with 3.3).
- Verified: `cargo check` both crates clean; `cargo test -p kria-core --lib "capability::"` = **128 pass**
  (was 107), neutrality gate green, flag-off parity preserved.

## Runtime / environment facts (verified)
- **LLM (fixed this session):** cloud provider `opencode` (`https://opencode.ai/zen/v1`). Valid key set in
  SQLite config store (`~/.kria/kria.db` → `config` table: `llm.cloud_api_key`, `llm.cloud_model_id`,
  and the `providers[]` opencode entry). Chosen model: **`deepseek-v4-flash-free`** (reliable
  `tool_calls`, clean `content` w/ no reasoning-channel complication, ~2.5s). Other working models:
  `nemotron-3-ultra-free`, `mimo-v2.5-free` (both use `reasoning` channel). `minimax-m2.5-free` = not supported.
- **Config authority:** `ConfigBackend::from_env()` defaults to **Sqlite** (`kria.db` is authoritative);
  `~/.kria/config.toml` edits are ignored unless `KRIA_CONFIG_BACKEND=toml`. Env override `KRIA_LLM_MODE=local`
  forces local routing. This split-brain is task **0.2 (still PENDING)**.
- **Launch:** `cargo tauri dev` (rebuilds kria-core+kria-desktop; ~3–5 min). Health: `curl :3001/api/health`.
  API token: `~/.kria/api_token`. Drive real turns: `POST :3001/api/chat` with bearer token
  (same pipeline as desktop UI). Enable intelligence flags: `capability.enabled=true` +
  `capability.intelligence` JSON in the SQLite `config` table (ckb/reasoner currently set true).
- **Chat turns are slow/queue-prone** in this box; curl often times out even though the backend runs.
  Verify via logs (`~/.kria/logs/kria.log.<date>`) + DB, not just curl return.

## What's DONE + verified (Waves 0–5)
All in `crates/kria-core/src/capability/intelligence/`:
- **W0:** `config.rs::CapabilityIntelligenceConfig` (all flags OFF default). `intelligence/mod.rs` (12 neutral
  traits + value types: ReasoningPolicy, GoalClass, CostVector, ScoredCandidate, Strategy, PlanStep,
  SolutionPlan, DecisionRecord, ExecutionPath, Selection). `kind.rs` (CapabilityKind/Family + infer_*).
  `neutrality.rs` (CI gate `#[test] brain_hands_neutrality_gate` — passes). Versioned policy/telemetry.
- **W1 (CKB):** `knowledge.rs::SqliteCapabilityKnowledge` (`cpp_knowledge.db`: `cpp_knowledge` + `cpp_decisions`
  tables; record_install/outcome/decision, list_installed, success_rate, purge, set_state, schema_version).
  Wired into `platform.rs::execute` via `with_knowledge`; injected in `runtime.rs` behind `intelligence.ckb`
  flag. Durable-across-restart proven.
- **W2 (selection):** `selector.rs::DefaultCapabilitySelector` (confidence fusion + native-sufficiency gate).
  Wired into `tools/capability_dispatch.rs` behind `.with_reasoner(flag)`. Decision Records recorded live.
- **W3 (planning):** `planner.rs::DefaultCapabilityPlanner` (typed-IO chaining, saga, effect union +
  `plan_reversibility`). `plan_executor.rs::CapabilityPlanExecutor` (sequential exec + honest saga partial).
  NOTE: `agent/htn_executor.rs` is **GUI-specific**, NOT a general capability engine — capability plans run
  via the thin `CapabilityPlanExecutor` over `platform.execute` (this is the correct layering; planning
  authority arbitration = task 3.1, still PENDING).
- **W4 (lifecycle):** `lifecycle.rs::DefaultLifecycleManager` (acquire→verify→smoke→activate w/ rollback,
  upgrade, reversible retire, cascade delete).
- **W5 (plan permission):** `plan_permission.rs::authorize_plan` (union at max-risk, one-approval-per-plan).

Tests: ~50 new unit/integration tests green; full `kria-core --lib "capability::"` = 107 pass. Both crates
`cargo check`/`fmt`/`clippy` clean. One PRE-EXISTING unrelated flake: `agent::continuation_reentry::
duplicate_continuation_is_rejected` (order-dependent DecisionStore shared-state; passes in isolation; NOT ours).

## Substrate investigation (this session's main result)
- **The "`RuntimeManagerSpawn::create_container is not implemented`" WARN is NOT a bug/blocker.** It is a
  **deliberate leak-safety no-op** used only by BACKGROUND prewarm/recycle/recovery. The hot path
  (`RuntimeManager::create_container`, `runtime_manager.rs:1755`) is a complete real-Docker impl;
  `checkout_container` (`:1507`) creates on-demand when no warm container exists → pool self-heals.
- **Proven:** `KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_e2e_dispatch_docker` → **24/24 PASS**
  (real containers, 9 capability families + MCP + permission gate + grant reuse). Substrate is production-functional.
- **Fix applied:** downgraded the per-15s WARN spam to a one-time INFO + at-most-once debug for the
  deliberate-disabled case; genuine Docker errors still WARN (`start_prewarming_system`). No regression.
- **Residual (not a blocker):** proactive background prewarm + recycle/recovery *replacement* are genuinely
  disabled (leak-safety). Enabling needs a "stop-loop-then-reap" guarantee at every pool owner (reverted once
  because it regressed the eval leak-baseline suite). Do NOT naively enable without running that suite.

## Other real findings (still open)
- **Capability hallucination / inconsistent marketplace routing / arg-abort** observed in early live tests are
  dominated by model quality; largely addressed by the selector + CKB grounding + (pending) constrained arg-gen.
- **Config split-brain (0.2)** still pending — surface effective backend + reconcile TOML↔SQLite.

## NEXT STEPS (recommended order)
1. Resume **Wave 6** (marketplace intelligence): neutral catalog ranking signals, hash+signature verify,
   catalog cache, namespacing/versioning/deps, ClawHub model. (Backend; testable.)
2. **Wave 3.1** planning-authority arbitration (one winner among ReAct/HTN/n8n/synthetic/CPP) — needed before
   composition ships live.
3. **Frontend track** (R27/§34 wiring map): `cpp_installed/families/health/decisions`, reasoning/plan panels,
   Jobs, oversight — pair each backend component with its command+event+view.
4. **Wave 12 real-UI campaign** once turns are drivable (LLM now works; watch turn latency).

## Verification commands
```
cargo test -p kria-core --lib "capability::"                     # 107 pass
KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_e2e_dispatch_docker -- --nocapture  # 24/24
cargo check -p kria-core && cargo check -p kria-desktop          # clean
curl -s :3001/api/health                                          # app up
```

## Guardrails (do not violate)
Provider-neutral (Brain=KRIA, Hands=OpenClaw); no `crate::openclaw`/`mcp::client` in `capability/` outside
`acl/` (CI-gated); flags default OFF ⇒ byte-identical legacy; no fake/stub on hot paths; no hardcoded
prompts/keywords/provider names; honest errors (no fabricated success). Composition feeds one execution
engine — do NOT add a rival planner. Memory/CKB stays behind its trait for the future Memory redesign.
