# OpenClaw Production Hardening — Session Handoff

> Context transfer for the next session. Architecture A0–A9 is FROZEN: repair,
> integrate, harden, and prove only. No redesign, no parallel implementations.
> The REAL KRIA Desktop (tauri-driver + WebKitWebDriver) is the source of
> truth — backend unit tests are NOT proof.

---

## TOP PRIORITY — FIXED (code + unit-level), real-GUI proof still pending

**Status: FIXED in code.** `tool_matches_lab_app_lock`'s `"openclaw" | "claw"` arm
(`crates/kria-core/src/agent/loop_engine/mod.rs`) now allows `"openclaw"`,
`"list_installed_skills"`, AND `oc_*` (backward compat) instead of `oc_*`-only.
Confirmed root cause first: A6 registers exactly two tools — `"openclaw"` and
`"list_installed_skills"` (both category `openclaw`, `openclaw/handler.rs`
`register_semantic_openclaw`); per-skill `oc_*` tools no longer exist, so the old
`starts_with("oc_")`-only gate blocked the only tools that satisfy an OpenClaw request.
Permanent regression test added: `openclaw_app_lock_allows_real_a6_semantic_tools`
(`loop_engine/tests.rs`) — asserts the 2 real tools pass, `oc_*` still passes, no
over-broadening (unrelated tools stay blocked, semantic tools don't leak into other
modes). `cargo test -p kria-core --lib app_lock` → 3/3 pass; kria-core lib+tests build
clean.

**STILL PENDING (per mission rule "REAL GUI is source of truth"):** prove via real
desktop — select OpenClaw Tool Mode → "Use OpenClaw to calculate 3+3" → real
`oc_calculator` executes in a real container. Requires GUI binary rebuilt with
`cargo tauri build --debug --no-bundle` + tauri-driver + Docker + LLM up.

**RELATED — ALSO FIXED (same session):** the `"docker"` app_lock arm had the same
`oc_*`-only defect and would have blocked the semantic `"openclaw"` tool. OpenClaw
skills run in Docker containers, so "docker" mode should reach them — the arm now also
allows `"openclaw"` + `"list_installed_skills"`. Covered by the same regression test.

---

### Original diagnosis (kept for reference)

**Selecting "OpenClaw" from the UI Tool Mode dropdown did NOT work.** This was the
single most important open item and was confirmed with real code tracing:

- UI dropdown "OpenClaw" → `appLock: "openclaw"` (`ui/src/stores/app.ts`, `MANUAL_TOOL_MODES`).
- Backend gate: `TurnExecutionProfile.allows_tool_name()` →
  `tool_matches_lab_app_lock(tool_name, "openclaw")` in
  `crates/kria-core/src/agent/loop_engine/mod.rs` (~line 4193).
- The rule is: `"openclaw" | "claw" => tool_name_lower.starts_with("oc_")`.
- BUT the real executable tool is named **`"openclaw"`** (single semantic tool, A6),
  and the introspection tool is **`"list_installed_skills"`** — NEITHER starts with
  `oc_`. So locking to OpenClaw mode blocks the only tools that can satisfy the request.
- Root cause: leftover from the pre-A6 architecture where each skill was a separate
  `oc_*` tool. The gate was never updated when A6 replaced that with one `"openclaw"` tool.
- Fix (one line, low risk, additive): change the `"openclaw" | "claw"` match arm to allow
  `"openclaw"`, `"list_installed_skills"`, AND `oc_*` (keep `oc_` for backward compat).
- MUST be proven by: select OpenClaw mode in real GUI → "Use OpenClaw to calculate 3+3" →
  real `oc_calculator` executes in a real container. Add a permanent regression test on
  `tool_matches_lab_app_lock` / `allows_tool_name`.

---

## Confirmed environment ground truth (verified this session)

- `~/.kria/config.toml`: `[openclaw] enabled = true`, image `kria/openclaw-substrate:latest`,
  registry `index_url = https://raw.githubusercontent.com/ObaidGits/kria-skills/refs/heads/main/index.json`.
- `~/.kria/skills.db` (SQLite, WAL) currently holds 3 enabled curated skills:
  `oc_calculator`, `oc_web_search`, `oc_web_fetch` — all `state='enabled'`, `trust_tier='verified'`, `risk='GREEN'`.
- Real GPU: RTX 4050 Laptop (6141 MiB). LLM boots via internal orchestrator; `ngl=27` is the
  known-good GPU offload (ngl=36 hangs on this box — see ngl-ladder fix below).
- Cloud provider configured for A9 generation: `opencode`, `https://opencode.ai/zen/v1`,
  `deepseek-v4-flash-free`, key in `~/.kria/config.toml`.
- Docker: unrelated containers `kria-guacd`, `n8n`, `portainer`, `python-services-redis-1`
  must NEVER be touched. After every Docker-touching run verify
  `docker ps -aq --filter "name=kria-openclaw" | wc -l` == 0 (leak discipline).

## CRITICAL operational hazard (recurring)

- The entire A6 OpenClaw architecture is **uncommitted** in the working tree (git HEAD has
  the old A5 code). ~20 modified + ~15 new untracked files under `crates/kria-core/src/openclaw/`,
  plus loop_engine/router/tools/ui edits. There is NO stable git baseline.
- **Frontend-embedding regression**: plain `cargo check`/`cargo build`/`cargo test` on
  `kria-core` silently recompiles `kria-desktop` WITHOUT Tauri asset embedding, reverting the
  binary to a broken `devUrl: http://localhost:1420` fallback (webview shows
  "Could not connect to localhost"). ALWAYS rebuild the GUI binary with
  `cargo tauri build --debug --no-bundle` before any GUI test. A guard was added to
  `tests/gui-cognition-e2e/wdio.conf.ts` that refuses to run if the binary lacks embedded
  asset markers (checks `strings <binary> | grep -c "assets/index-"` via bash, large maxBuffer).

---

## Verified-WORKING this session (real GUI, tauri-driver, real Docker+LLM)

Spec `tests/gui-cognition-e2e/specs/openclaw_pipeline_regression.e2e.ts` — 6/6 mocha-pass
after readiness-wait fix (see below). Replays the ORIGINAL failing transcript prompts:
- "Use the openclaw calculator skill on 3+3" → real `oc_calculator` executes, correct answer,
  no "unknown error" (this was the exact original failure).
- "Use OpenClaw to evaluate the expression 8 * 8" → real execution, 64.
- "List installed OpenClaw skills." → reflects the real registry (via `list_installed_skills`).
- Nonexistent skill → honest, explained decline (not bare "unknown error").
- Marketplace/generated-skills questions → no filesystem-dotfile hallucination.

Backend real-Docker suite: `cargo test -p kria-eval --lib execute_e2e` → 5/5 pass, 0 leaks
(proves Router→Registry→Runtime→Docker→Container→Skill→Response works end to end).

NOTE: The above proves the **default (Auto) tool mode** works. The TOP PRIORITY bug is that
the **explicit manual "OpenClaw" mode** is what's broken.

---

## Fixes already applied this session (verify still present; all uncommitted)

1. **`tool_end_result_payload`** (`loop_engine/mod.rs` ~line 802) — Phase 10 error-system fix.
   Failed `ToolResult` sets `data=Null`, real message in `.error`; the `ToolEnd` event used to
   forward only `data`, so every failed tool showed "unknown error". Now folds `.error` into the
   payload. Regr tests: `regr_phase10_tool_end_payload_*` (3, passing).
2. **ngl backoff ladder** (`llm/orchestrator/mod.rs`, `build_ngl_backoff_ladder`) — cached
   known-good ngl == full collapsed the ladder to `[full, 0]`, causing 15+ min CPU-only boots.
   Now always builds full `full→3/4→1/2→1/4→0` ladder. 6 regr tests passing.
3. **n8n misrouting** (`n8n/matching.rs`) — hash/skill/search prompts no longer mis-route to the
   "Mail Schedule Test" workflow; whole-word matching + exclusion-list + `prompt_has_explicit_n8n_intent`
   fixes. Regr tests `regr_bug1_*`.
4. **`hash_text`** tool added (`tools/interaction.rs`) — real md5/sha1/sha256/sha512/blake3.
5. **`transform_text`** tool added (`tools/interaction.rs`) — literal-string transforms so
   `transform_clipboard` is no longer mis-selected and mutating the real clipboard.
6. **`list_installed_skills`** tool added (`openclaw/handler.rs`) — real registry introspection,
   wired to router hints in `agent/router.rs` for "which skills installed/enabled/disabled".
7. **`tool_name` placeholder** filtered in `response_parser.rs` (`is_placeholder_tool_name`).
8. **parse_csv inline text** (`tools/documents.rs`) — accepts `csv_text` param, not just file path.
9. **Dropped-turn fix** (`loop_engine/mod.rs`) — two terminal branches now emit "Turn cancelled."
   instead of silently returning. Regr `regr_bug7_*`.
10. **ChatView handleSubmit** (`ui/src/components/ChatView.tsx`) — removed premature
    `if (isThinking()) return;` that bypassed the real prompt queue. Regr in `app.tool-choice.test.ts`.
11. **Registry-empty bug — "No enabled skills found in registry" (SEVERE, root-caused + FIXED).**
    `crates/kria-core/src/openclaw/registry.rs` `row_to_metadata` read the `skills` table by POSITIONAL
    INDEX (`row.get(20)` = granted_capabilities). Schema migration 1 adds `granted_capabilities` via
    `ALTER TABLE ADD COLUMN`, which SQLite APPENDS at the END of the table. So a fresh DB has it at index
    20 (parser worked) but any EXISTING/upgraded user DB has it at the last index (26) — index 20 then
    resolves to `bundle_path` (NULL) → `get::<String>(20)` returns `InvalidColumnType` → the whole row
    fails to parse → silently dropped by `search_skills`' `if let Ok(..)`. Result: 0 enabled skills on
    every upgraded user, even though `skills.db` genuinely held 3 enabled curated skills. Fix: read every
    column BY NAME (order-independent) + log dropped rows instead of swallowing them.
    - Files: `crates/kria-core/src/openclaw/registry.rs` (`row_to_metadata` by-name; `search_skills` warn-on-drop).
    - Tests: `registry_tests.rs::enabled_skills_load_from_pre_granted_capabilities_migrated_db`
      (reproduces the exact migrated column order) + `tests/openclaw_real_db_smoke.rs`
      (gated on `KRIA_REAL_SKILLS_DB`; run against a COPY of the real `~/.kria/skills.db` → 0→3 enabled).
    - Proven: real-DB copy returned `oc_calculator`, `oc_web_search`, `oc_web_fetch` all `Enabled`.
    - **RUNTIME-PROVEN in the REAL desktop** (from `~/.kria/logs/kria.log.2026-07-05`, session
      `f54e43c9-…`, prompt "Use OpenClaw to calculate 3+3" in manual OpenClaw Tool Mode):
      `tool_calls_parsed → [{"name":"openclaw","arguments":{"query":"calculate 3+3"}}]` (GATE fix works —
      the openclaw tool is selected & dispatched under the OpenClaw lock, impossible before) →
      `Tool execution started tool_name=openclaw` → `policy_evaluated risk_level=RED requires_approval=true`
      → `approval_requested` (REGISTRY fix works — the tool progressed PAST `get_enabled_skills()`; the
      "No enabled skills" error is gone). Both fixes verified end-to-end at runtime.

12. **RuntimeManagerSpawn::create_container implemented (warm-pool prewarm gap FIXED).**
    `crates/kria-core/src/openclaw/runtime_manager.rs`. The continuous prewarm loop, checkin
    recycle-replacement, and `trigger_recovery` replacement all spawn via `RuntimeManagerSpawn`, whose
    `create_container` was a stub returning an honest error — so after boot the warm pool could never
    replenish (real desktop logged "Prewarming failed … not implemented against real Docker" every 15s).
    Now performs the same real create+start+register as `RuntimeManager::create_container`, keeping the
    `kria-openclaw` name prefix for leak detection. Safe because the earlier task-2 hardening already made
    `shutdown()` join the prewarm task before the destroy sweep. Gated real-Docker regression test
    `runtime_manager::spawn_prewarm_tests::spawn_create_container_makes_a_real_container` (creates,
    asserts registration, destroys — 0 leaks). Workspace builds clean.

## tasks.md restructured (spec-format clean)

- Progress log (per-task evidence) moved OUT of `tasks.md` into `PROGRESS.md` — the `### Task N — DONE`
  headings were what triggered the 31 spec-format diagnostics. `tasks.md` is now a clean checkbox plan
  (`# Implementation Plan` + `## Overview` + conventions + tasks + dep graph + notes) → **0 diagnostics**.
- Checkboxes synced to PROGRESS.md's explicit DONE markers: DONE = 1–23, 25, 26, 29, 33, 34, 35 (+11.1);
  PENDING = 11 (11.2 externally blocked on real-LLM), 24 (GUI wave), 27 (4–8h soak), 28 (UX truthfulness),
  30 (continuous regression capture), 31 (release checklist), 32 (feature matrix).

## REMAINING to render the literal "6" in the GUI (separate from the two fixed bugs)

1. **RED-tier HITL approval hangs the headless test.** The `openclaw` tool is classified RED by the
   pipeline safety policy → `approval_requested` → the turn waits for user approval that the automated
   spec never answers (120s timeout). To fully render "6", the GUI spec
   (`tests/gui-cognition-e2e/specs/openclaw_manual_mode.e2e.ts`) must drive the `assistant:approval_required`
   UI (click Approve) — OR a test-mode auto-approve must be enabled. This is correct safety behavior, not
   a bug. Harness now already starts a fresh chat per test (history-reload issue resolved).
2. **`RuntimeManagerSpawn::create_container is not implemented against real Docker`** — the warm pool's
   prewarm path logs this repeatedly (Light/Medium/Heavy). This is the deliberate honest-error from
   task 2 (no fabricated container ids). Real execution uses `SemanticOpenClawHandler → DockerRuntime →
   ContainerPool` (proven working in `kria-eval execute_e2e`), but the RuntimeManager-driven prewarm is a
   real gap to close/confirm for the full GUI execution path. Filed as a distinct known issue.

## GUI test harness notes (learned the hard way)

- `before()` MUST wait for real backend readiness (`.status-dot` class == exactly `"status-dot"`,
  no `warming`/`degraded`/`disconnected` suffix), NOT a fixed pause. Backend attaches the model
  router ~21s AFTER the textarea renders; sending before that = silent prompt loss. This is a REAL
  finding a real user could hit (Phase 1 boot pipeline).
- Send/textarea clicks get transiently intercepted during streaming DOM reflow — wrap clicks in
  retry + `browser.execute` DOM-click fallback (already done in the pipeline-regression spec).
- `wdio`'s mocha reporter (`✓`/`✗`) is authoritative. The custom `after()` summary array in specs
  can show stale/empty entries — do NOT trust it over mocha's own pass/fail.
- Terminal stdout capture buffers unreliably; read the real report at
  `tests/gui-cognition-e2e/reports/<spec>-0-0.log` and grep for `deleteSession` (run end) / `RESULT`.

---

## MISSION — remaining phases (in priority order)

**Immediate (do first):**
- Fix the TOP PRIORITY manual-OpenClaw-mode gate bug above; prove via real GUI in OpenClaw mode;
  add regression test; then also verify n8n/gmail/etc. modes still gate correctly (no over-broadening).

**Then the full audit (each phase = REAL GUI proof, not unit tests):**
- **P1 Boot pipeline**: ordering, no race/double/missing init, no stale caches, no start-before-deps.
  (Known real issue: chat accepts input before model router attached → silent loss.)
- **P2 Registry**: skills.db ⇄ registry always consistent; state/enable/disable/remove/update/discovery;
  no stale/duplicate/hidden registry. **"registry empty for 13h of boots" MYSTERY SOLVED + FIXED this
  session** (see fix #11 below) — root cause was NOT stale seeding; it was `row_to_metadata` reading
  columns by positional index while the `granted_capabilities` migration appended its column at the end
  of the table, so every enabled skill silently failed to parse on any upgraded DB.
- **P3 Semantic Router**: every OpenClaw request reaches the router; no silent native fallback unless
  OpenClaw genuinely can't satisfy; record every routing decision.
- **P4 Execution**: engine/executor/runtime-mgr/scheduler/container select+start+reuse+cleanup,
  cancel/recover/checkpoint/retry/rollback/metrics; verify every state transition.
- **P5 Docker**: image, skill mounting (`.bridge` dir), MCP bridge, load, exec, stdout/stderr,
  timeouts, cleanup; every installed skill actually present in the container.
- **P6 Skills**: calculator/web_search/web_fetch/json/regex/markdown/csv/text/gzip/hash + generated +
  marketplace; install/execute/update/remove/rollback/enable/disable each.
- **P7 Marketplace**: `ObaidGits/kria-skills` authoritative; sync/search/install/upgrade/remove/
  rollback/offline; install multiple real skills, execute, remove, reinstall. (Live repo has 1 skill:
  `oc_code_sandbox`.)
- **P8 A9 Generation**: generate→validate→repair→package→sign→install→registry→execute→remove→
  regenerate (use configured cloud provider; local models proven to not converge in budget).
- **P9 Desktop**: Settings / OpenClaw panel / Marketplace panel / Skill panel / Developer Mode / Logs /
  Metrics / container+runtime+pool+generation controls — every setting must control REAL backend
  behavior (no fake settings). Known gaps: no generated-skills view, no Developer Mode, no logs page.
- **P10 Error system**: hunt every "unknown error"/generic fallback; user always gets the real reason.
- **P11 True GUI testing**: hundreds of real prompts through tauri-driver.
- **P12 Stress**: 100 prompts/installs/removals/updates, parallel, container/docker/llm/network faults,
  app+docker restart, registry corruption → automatic recovery.
- **P13 Pipeline trace**: for every failed prompt, identify EXACT failing stage.
- **P14 Regression**: permanent test for every bug; replay ALL historical failures (n8n misroute,
  openclaw fallback, unknown error, marketplace failure, registry empty, calculator, permission loops,
  container reuse, generation, router mistakes, skill discovery).
- **P15 Production hardening / self-audit**: remove dead code, duplicate logic, legacy paths, hidden
  registries, stale caches; exactly ONE owner for registry/runtime/execution/marketplace/installer/
  generation/router/skill-lifecycle/desktop-integration.

## Success criteria

A real user in the real desktop can: "Use OpenClaw to calculate", "Install this skill",
"Search marketplace", "Generate a skill", "Update the skill", "Remove it", "List installed skills"
— all work, no native fallback, no hidden/unknown errors, no permission confusion, no manual
intervention.

## Working rules

- Every fix proven by replaying the ORIGINAL failing prompt through the REAL GUI.
- Update `.kiro/specs/openclaw-production-validation/tasks.md` continuously (findings, fixes,
  regressions, validations).
- Don't stop after one bug. Continue until the whole pipeline is production-ready or a remaining
  failure genuinely requires a new product feature outside OpenClaw.
- After every Docker run: verify 0 leaked `kria-openclaw` containers; never touch unrelated services.
- Rebuild GUI binary with `cargo tauri build --debug --no-bundle` (never plain cargo) before GUI tests.
