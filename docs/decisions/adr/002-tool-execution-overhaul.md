# ADR: Tool Execution Layer Overhaul for kria-core

Status: Proposed  
Date: 2026-05-05  
Owner: Systems Architecture  
Scope: `crates/kria-core/src/tools/` and adjacent execution contracts (`tools::registry`, tool runtime boundaries, integration tests)

## 1) Decision Summary

We will redesign the `kria-core` tool execution layer around four non-negotiable pillars:

1. Strict I/O Contracts using typed `serde` structs and `schemars` schema export.
2. Defensive Execution Wrapper as the only allowed OS process execution path.
3. High-Fidelity Integration Testing using local sandboxes and `wiremock` for external boundaries.
4. Pre-Flight Idempotency for all mutating tools.

No Rust implementation code is included in this ADR. This document is the master architecture + execution plan that implementation will follow.

## 2) Context and Current State (Scanned)

### 2.1 Structural Snapshot

Current registry contract (`tools/registry.rs`) exposes:

- `ToolHandler::execute(&self, params: serde_json::Value) -> ToolResult`
- `ToolDef` schemas manually assembled from `ParamDef { name, type, required, ... }`

Observed quantitative indicators in `crates/kria-core/src/tools/`:

- `tokio::process::Command::new` call sites: 78
- `std::process::Command::new` call sites: 1
- Raw parameter indexing (`params["..."]`) occurrences: 201
- `serde_json::from_value` usage for typed tool inputs: 0
- `schemars` usage in tools: 0
- In-file `#[cfg(test)]` coverage in tools module files: sparse (notably `internet.rs`, `google_workspace.rs`, `mount_manager.rs`)

### 2.2 Brittle Patterns We Must Eliminate

1. Untyped input contracts and silent coercion
- Most tools parse inputs with `params["field"].as_*().unwrap_or(default)`.
- Missing or malformed inputs often degrade into defaults instead of hard validation errors.
- `ToolDef` schemas are manually maintained and can drift from runtime parsing.

2. Process execution sprawl and inconsistent failure handling
- Process launch logic is scattered across many modules, including:
  - `shell.rs`, `packages.rs`, `system_config.rs`, `desktop.rs`, `power.rs`, `scheduler.rs`, `process.rs`, `documents.rs`, `vision.rs`, `interaction.rs`, `communication.rs`, `developer.rs`, `internet.rs`, `app_lifecycle.rs`, `system_info.rs`.
- Many call sites either:
  - do not enforce strict per-command timeout semantics, or
  - return generic errors without structured exit metadata.

3. Timeout layering and diagnostic inconsistency
- `AgentLoop` already wraps tool execution with `run_isolated(... timeout ...)`.
- Some tools apply additional nested `tokio::time::timeout` internally.
- Result: inconsistent timeout ownership and non-uniform error payloads.

4. Mutating tools without enforced idempotency checks
- In several tools, pre-checks are advisory (in prompt descriptions/tests) rather than guaranteed in tool logic.
- Example risk classes:
  - package install/remove,
  - cron creation/deletion,
  - filesystem mutation,
  - service/process control,
  - system configuration changes.

5. Integration coverage skew
- Existing tests include many routing/schema/registration checks and some live/opt-in paths.
- Deterministic boundary testing at the tool execution layer is uneven:
  - limited use of process sandbox stubs,
  - minimal `wiremock` usage for tool HTTP boundaries (despite dependency being available).

Note:
- Tool outputs are expected to feed a synthesis layer that separates conversational summaries from raw payloads; tool contracts should preserve structured outputs for this flow.

## 3) Architectural Decision

## 3.1 Pillar 1: Strict I/O Contracts

### Rule Set

1. Every tool gets explicit `Input` and `Output` structs.
2. `Input` must derive at least:
   - `Deserialize`
   - `JsonSchema` (via `schemars`)
3. `Output` must derive at least:
   - `Serialize`
   - `JsonSchema`
4. All input structs use `#[serde(deny_unknown_fields)]`.
5. No direct `params["..."]` parsing inside tool business logic after migration.
6. LLM function schemas are generated from typed contracts, not hand-written `ParamDef` strings.

### Contract Integration Strategy

We will keep backward compatibility while migrating by introducing a typed bridge:

- Incoming `serde_json::Value` is deserialized once at the tool boundary.
- Validation errors are normalized to a consistent contract error envelope.
- Tool implementation receives typed input only.

### Schema Authority

`schemars` output becomes single source of truth for exported tool schemas.

Manual `ParamDef` remains only as a temporary compatibility layer during migration phases, then is retired for typed tools.

## 3.2 Pillar 2: Defensive Execution Wrapper (ExecWrapper)

### Rule Set

1. Any OS process execution must go through `ExecWrapper`.
2. Direct `Command::new` in tool modules is disallowed after migration.
3. Every process run must provide:
   - explicit timeout policy,
   - cancellation token propagation,
   - bounded stdout/stderr capture policy.
4. Non-zero exit must include:
   - command identity (program + args fingerprint),
   - exit code/signal,
   - captured stdout/stderr (truncated with clear marker).
5. `sh -c`/`bash -c` execution is prohibited by default and only allowed under explicit reviewed exceptions.

### ExecWrapper Contract (Architecture-Level)

Core request model:

- Program (binary path/name)
- Args (vectorized, no shell interpolation)
- Working directory (optional)
- Environment overrides (allowlist-based)
- Timeout policy (required)
- Output capture limits (required)
- Stdin mode (none/string/bytes)
- Cancellation token

Core result model:

- `status`: success/failure/timeout/cancelled/spawn_error
- `exit_code` and `signal` where available
- `stdout` + `stderr` + truncation flags
- `duration_ms`
- `timed_out` / `cancelled` booleans

### Timeout Ownership Model

We will define single ownership clearly:

- `AgentLoop.run_isolated`: turn/tool-level envelope timeout and panic isolation.
- `ExecWrapper`: process-level timeout and termination semantics.
- Tool-level ad-hoc timeouts are removed unless semantically required for sub-steps, and then implemented through wrapper subcalls.

## 3.3 Pillar 3: High-Fidelity Integration Testing

### Rule Set

1. Do not unit test physical OS/network boundaries directly.
2. Test boundary behavior with deterministic harnesses:
   - local command sandbox binaries,
   - temp filesystem sandboxes,
   - `wiremock` for HTTP services.
3. Keep live tests opt-in and non-blocking for CI.
4. Each mutating tool must have at least:
   - one success integration test,
   - one pre-flight no-op/idempotent test,
   - one failure diagnostics test (non-zero exit or timeout).

### Standard Harnesses

1. Command sandbox harness
- Create temp `bin/` with stub executables returning deterministic outputs/exit codes.
- Prepend to `PATH` within test scope.

2. Filesystem sandbox harness
- Use temp roots for all file/directory mutation tests.
- Assert before/after state transitions.

3. HTTP harness (`wiremock`)
- Mock all remote endpoints for internet/news/weather/exchange and similar tools.
- Validate retries, timeout handling, response limits, and error mapping.

## 3.4 Pillar 4: Pre-Flight Idempotency

### Rule Set

1. Mutating tools must execute a mandatory pre-flight state check before apply.
2. Mutating results must explicitly indicate one of:
   - `changed: true` (mutation applied)
   - `changed: false` + `already_in_desired_state: true` (idempotent no-op)
3. Apply step runs only when pre-flight indicates drift.
4. Pre-flight and apply should be logged with structured before/after snapshots (redacted where needed).

### Standardized Mutating Result Fields

For all mutating tools, add a shared response shape contract concept:

- `changed`
- `already_in_desired_state`
- `state_before`
- `state_after`
- `action_summary`

## 4) Tool-by-Tool Application Plan

This section maps the four pillars to existing tool modules.

## 4.1 Wave 1 (Highest Risk): Command-Heavy Tools

### `tools/shell.rs`

- Current: raw command/code strings, nested timeout, direct shell invocation.
- Apply:
  - Strict typed inputs (`ExecuteBashInput`, `ExecutePythonInput`, ...).
  - Route all process calls through `ExecWrapper`.
  - Enforce shell policy exceptions explicitly.
  - Add idempotency metadata as not-applicable (read/execute category), but still standard error diagnostics.
  - Integration tests: timeout/non-zero/stdout-stderr truncation using sandbox command stubs.

### `tools/packages.rs`

- Current: multiple ad-hoc runners (`run_cmd`, `run_priv_cmd`) and broad command matrix.
- Apply:
  - Replace internal runners with `ExecWrapper` adapters.
  - Mandatory pre-flight in `install_package` and `uninstall_package`:
    - install: verify not already installed.
    - uninstall: verify currently installed.
  - Return explicit no-op states when already converged.
  - Integration tests: sandbox PM commands (apt/snap/flatpak/etc simulations), privilege escalation paths, non-zero diagnostics.

### `tools/system_config.rs`

- Current: fallback chains (`wpctl/pactl/amixer`, `gdbus/brightnessctl/xrandr`, etc.) with mixed diagnostics.
- Apply:
  - Typed requests for each operation.
  - `ExecWrapper` for all command attempts with standardized per-backend error traces.
  - Pre-flight checks:
    - volume already at target,
    - brightness already at target,
    - wifi already on/off,
    - power plan already selected,
    - env var already same value.
  - Integration tests with command stubs for each fallback tier.

### `tools/desktop.rs`

- Current: many direct window command calls; failure detail often generic.
- Apply:
  - Strong input contracts for coordinates/layout/title matching.
  - Central process execution via `ExecWrapper`.
  - Idempotency examples:
    - maximize/minimize should no-op if already in state (where queryable).
    - tile operation should validate current layout and report drift.
  - Integration tests with wmctl/xdotool/xprop stubs.

### `tools/power.rs`

- Current: shell command strings through `sh -c`.
- Apply:
  - Remove shell-string dispatch and use direct program+args wrappers.
  - Strict input for delayed shutdown and policy-safe bounds.
  - Pre-flight checks for delayed shutdown scheduling where supported.
  - Integration tests for success/failure/timeout on each power action.

### `tools/scheduler.rs`

- Current: direct crontab mutation without duplicate prevention.
- Apply:
  - Typed cron entry model with validation.
  - `ExecWrapper` for `crontab` and `systemctl` operations.
  - Pre-flight checks:
    - create only when entry absent,
    - delete only when entry present.
  - Integration tests using fake crontab command sandbox.

### `tools/process.rs`

- Current: direct `renice/systemctl/ss` with coarse error handling.
- Apply:
  - Typed service/process requests.
  - Wrapper-based execution + structured errors.
  - Pre-flight:
    - priority already set,
    - service already in desired state.
  - Integration tests with command stubs.

### `tools/developer.rs`

- Current: custom `run_git` helper, direct `diff` invocation, read-only SQL guard by prefix.
- Apply:
  - Replace command runners with `ExecWrapper`.
  - Typed contracts for git commands.
  - Pre-flight:
    - `git_commit` no-op when clean workspace,
    - `git_checkout` no-op when already on branch.
  - Integration tests in temp git repositories + command sandbox for failure modes.

### `tools/documents.rs`

- Current: direct `pdftotext`/`pandoc` fallback calls.
- Apply:
  - Typed extraction requests (including operation flags).
  - Wrapper for fallback binaries.
  - Pre-flight: file existence, extension support, size caps.
  - Integration tests with sandbox binaries and temp documents.

### `tools/vision.rs`

- Current: direct `tesseract` and screenshot command execution.
- Apply:
  - Typed contracts for path resolution, ops, and hint fields.
  - Wrapper for local CLI fallbacks.
  - Pre-flight:
    - verify image path resolves deterministically,
    - verify required tooling/backend availability before apply.
  - Integration tests with fake `tesseract` and screenshot command stubs, temp images.

### `tools/interaction.rs`

- Current: screenshot and typing rely on direct command calls.
- Apply:
  - Typed input contracts.
  - `ExecWrapper` for `xdotool` and screenshot backends.
  - Pre-flight:
    - output path validation for screenshot,
    - non-empty text and target readiness checks for typing.
  - Integration tests with display-tool stubs.

### `tools/communication.rs`

- Current: notify/sound command execution with spawned tasks.
- Apply:
  - Typed contracts for notification/reminder inputs.
  - Wrapper-based process invocations for `notify-send`/`paplay`.
  - Pre-flight:
    - validate reminder delay bounds,
    - deduplicate identical pending reminders optionally.
  - Integration tests for delayed reminder execution with deterministic clock bounds.

### `tools/internet.rs`

- Current: mixed reqwest + direct `ping`/`dig`, with live-network leaning tests.
- Apply:
  - Typed request/response contracts for each internet tool.
  - `ExecWrapper` for `ping`/`dig`.
  - HTTP behavior tested through `wiremock` (status, retries, content limits, redirects, timeout).
  - Pre-flight:
    - destination/path checks for downloads,
    - optional no-op if destination already matches checksum/etag.

### `tools/app_lifecycle.rs`

- Current: modern dispatcher path plus legacy direct command/open fallbacks.
- Apply:
  - Typed contracts for all actions.
  - Route legacy process paths through `ExecWrapper` until legacy path is removed.
  - Pre-flight:
    - open app no-op guidance when already focused (where queryable),
    - close/kill should surface already-stopped state explicitly.
  - Integration tests for dispatcher-backed and fallback modes.

### `tools/system_info.rs`

- Current: mostly library-backed, one direct `nvidia-smi` call.
- Apply:
  - Typed output envelopes.
  - Move GPU process invocation through wrapper for consistency.
  - Integration tests with `nvidia-smi` stubs + library-only fallback behavior.

## 4.2 Wave 2: Filesystem and State Mutation Tools

### `tools/file_ops.rs`

- Current: rich file features but mutating flows often apply directly.
- Apply:
  - Typed input/output contracts for all operations.
  - Pre-flight idempotency hardening:
    - `create_directory`: no-op when exists,
    - `write_file`: include hash-before/hash-after and no-op if identical,
    - `rename/copy/move/delete`: explicit existence checks and deterministic no-op/error policy.
  - Integration tests in temp sandboxes with before/after assertions.

### `tools/disk.rs`

- Current: deletes temp entries by age without explicit dry-run mode.
- Apply:
  - Typed clean request including optional dry-run.
  - Mandatory pre-flight inventory with candidate list/count/size.
  - Apply only after pre-flight decision.
  - Integration tests in tempdirs with aging simulation.

### `tools/knowledge.rs` and `tools/rag.rs`

- Current: mostly internal storage operations; still raw `params[...]` parsing.
- Apply:
  - Typed contracts.
  - Pre-flight for deletions and ingestions (exists/non-empty/already indexed).
  - Integration tests using temp memory stores and deterministic seeds.

### `tools/proactive.rs`

- Current: operational state changes (`dismiss_alert`, `watch_directory`) with minimal typed validation.
- Apply:
  - Typed contracts and stronger validation for watch targets/labels.
  - Idempotent `watch_directory` semantics when already watched.
  - Integration tests with temp directories.

## 4.3 Wave 3: Sidecar and API Contract Tools

### `tools/news.rs`, `tools/precognitive.rs`, `tools/image_generation.rs`, `tools/google_workspace.rs`

- Current: sidecar/MCP-heavy, partial contract discipline, mixed payload parsing.
- Apply:
  - Strict typed tool-level contracts at Rust boundary.
  - Versioned sidecar/MCP payload contracts with explicit validation and conversion layers.
  - `wiremock` for HTTP-facing dependencies; mock sidecar transport for JSON-RPC determinism.
  - Pre-flight where mutations exist (create/edit/delete send actions).

### `tools/google_workspace_contract.rs`

- Role:
  - Keep as a shared envelope/error contract module.
- Action:
  - Align it with typed tool request/response structs and schema-versioned evolution.

## 4.4 Wave 4: Cross-Cutting Registry and Execution Core

### `tools/registry.rs`

- Replace manual schema assembly path with typed schema export pipeline.
- Keep compatibility during transition with dual registration mode:
  - legacy `ParamDef`-based,
  - typed `schemars`-based.
- Add migration gates and telemetry for remaining legacy tools.

### `agent/loop_engine.rs` + `infra/isolation.rs` alignment

- Preserve `run_isolated` for turn-level cancellation/panic safety.
- Ensure process-level execution semantics are delegated to `ExecWrapper`.
- Remove ad-hoc per-tool timeout logic from tool internals as migrations complete.

## 5) Implementation Roadmap (Step-by-Step)

## Phase 0: Baseline and Guardrails

1. Add ADR acceptance checklist and migration tracker.
2. Add lint-style CI guard to reject new direct `Command::new` in `tools/` (temporary allowlist during migration).
3. Add metrics logging for legacy input parsing paths.

Exit criteria:

- Migration tracker in place.
- CI warns/fails on new process-sprawl regressions.

## Phase 1: Introduce Core Primitives

1. Add `schemars` dependency and typed schema export plumbing.
2. Introduce typed tool execution adapter (legacy-compatible).
3. Introduce `ExecWrapper` abstraction and test harness.
4. Introduce shared mutating-tool pre-flight response conventions.

Exit criteria:

- At least one pilot tool uses typed input + wrapper + pre-flight + integration tests.

## Phase 2: Command-Heavy Migration (Wave 1)

1. Migrate `shell.rs`, `packages.rs`, `system_config.rs`, `desktop.rs` first.
2. Migrate `power.rs`, `scheduler.rs`, `process.rs`, `developer.rs`.
3. Migrate `documents.rs`, `vision.rs`, `interaction.rs`, `communication.rs`, `system_info.rs`, `app_lifecycle` legacy path.
4. Migrate `internet.rs` command portions and HTTP tests to deterministic wiremock strategy.

Exit criteria:

- No direct process execution remains in Wave 1 modules.
- Each migrated mutating tool has pre-flight behavior and deterministic integration tests.

## Phase 3: State Mutation and Sidecar/API Migration (Waves 2 and 3)

1. Migrate `file_ops.rs`, `disk.rs`, `knowledge.rs`, `rag.rs`, `proactive.rs` to typed contracts and strict idempotency.
2. Migrate sidecar/API tool surfaces (`news`, `precognitive`, `image_generation`, `google_workspace`) to typed boundaries and deterministic transport tests.

Exit criteria:

- All tools expose typed contracts.
- Mutating paths emit consistent pre-flight/apply metadata.

## Phase 4: Registry Finalization and Legacy Removal

1. Remove manual schema drift paths from `ToolDef` for migrated tools.
2. Remove legacy adapters once migration reaches 100%.
3. Enforce hard CI policy: no raw input parsing and no direct command execution in tools.

Exit criteria:

- 100% tool coverage on pillars 1-4.
- Legacy compatibility layer removed or explicitly constrained to documented exceptions.

## 6) Testing Strategy by Pillar

### Pillar 1 (Contracts)

- Contract decode tests: missing fields, wrong types, unknown fields.
- Schema snapshot tests (stable JSON schema export).

### Pillar 2 (ExecWrapper)

- Non-zero exit includes stdout/stderr and exit code.
- Timeout causes kill + timeout status.
- Cancellation token interrupts long-running process.
- Output truncation flags are accurate.

### Pillar 3 (Integration)

- Process tools: command sandbox integration suite.
- Network tools: `wiremock` suites for retries, redirects, status mapping.
- Filesystem tools: tempdir-based before/after assertions.

### Pillar 4 (Idempotency)

- Mutating tool no-op test when already converged.
- Mutating tool apply test when drift exists.
- Repeat apply test confirms stable no-op on second run.

## 7) Migration Risks and Mitigations

1. Risk: schema drift during partial migration
- Mitigation: dual schema export with parity tests until cutover.

2. Risk: timeout regressions from layered controls
- Mitigation: explicit timeout ownership matrix and wrapper-only process timeout policy.

3. Risk: behavior changes in fallback-heavy tools (`system_config`, `packages`, `vision`)
- Mitigation: fixture-driven command sandbox tests for each fallback tier.

4. Risk: increased implementation volume
- Mitigation: wave-based rollout with strict acceptance gates per phase.

## 8) Acceptance Criteria for This ADR Initiative

The overhaul is complete only when all are true:

1. Every tool has typed serde input/output contracts.
2. Tool schemas are generated from typed contracts (`schemars`), not manual `ParamDef` strings.
3. No direct process execution remains in tool modules (except explicitly documented temporary exceptions).
4. Every mutating tool performs mandatory pre-flight idempotency checks.
5. Deterministic integration tests exist for command and network boundaries (sandbox + wiremock).
6. Live environment tests remain opt-in and are not required for baseline CI correctness.

## 9) Non-Goals (for This ADR)

1. Rewriting business logic intent/routing in `AgentLoop`.
2. Redesigning safety policy semantics (approval rules remain in policy engine).
3. Replacing sidecar/MCP architecture itself.

## 10) Immediate Next Step (Execution Kickoff)

Start implementation with a pilot slice:

1. Introduce typed contract + `schemars` export + `ExecWrapper`.
2. Migrate `tools/shell.rs` end-to-end under all four pillars.
3. Use pilot learnings to template the remaining Wave 1 migrations.
