# NATURAL VM EVAL REPORT

**Date:** 2026-05-08  
**Scope:** Natural-language VM intent routing + fleet target resolution + Zone 0 stability

## 1) Outcome Snapshot

- **VM prompt matrix routing:** **54/54 passed (100%)**
- **Zone 0 UI pre-flight (`npm run check`):** **PASS**
- **Full runner (`cargo kria-test --mode FULL`):** **PASS** with VM-destructive suites skipped due VM probe unreachable in this environment
- **Natural intent additions:** `my VM`, `the server`, `VM1`, and generic health/status language now resolve deterministically

## 2) Implemented Changes

### A. Natural Language -> VM Intent Bridge

- Updated `crates/kria-core/src/agent/router.rs`:
  - Added direct routing for general health/status prompts to **`check_device_health`**.
  - Added routing for `remote command: ...` to **`execute_fleet_command`**.
  - Added tests:
    - `is my VM up?` -> `check_device_health`
    - `check status of the server` -> `check_device_health`

### B. Primary Target + Fuzzy Name Resolution

- Updated `crates/kria-desktop/src/device_control.rs`:
  - Added fuzzy token normalization (`vm1`, `vm 1`, alias fragments, compact forms).
  - If no explicit target hint is supplied, runtime now resolves to **primary enrolled target** (first deterministic projection entry).
  - If hint is generic (`my vm`, `the server`, `vm1`) and exact match is missing, fallback resolves to primary target from registry-backed projections.

### C. Explicit Error Propagation

- Updated:
  - `crates/kria-desktop/src/commands/device_tools.rs`
  - `crates/kria-desktop/src/device_control.rs`
- Behavior:
  - Dispatch/connectivity failures are classified to explicit user-facing forms when detected:
    - `Permission Denied (Publickey)`
    - `SSH Timeout`
    - `Connection Refused`
    - `Host Unreachable`
  - Generic “dispatch issue” messages are replaced with classified errors + contextual details.

### D. Health Tool for Non-Technical Prompts

- Added **`check_device_health`** tool in `crates/kria-desktop/src/commands/device_tools.rs`.
- Runs lightweight remote health command:
  - `hostname && whoami && uptime`
- Supports optional target hint; defaults to primary target resolution.

### E. UI Degraded/Unreachable Signal Mapping

- Updated `ui/src/hooks/useDeviceStatus.ts`:
  - Extended `DeviceTargetView.state` with `degraded` and `unreachable`.
  - Added normalization logic to infer unreachable/degraded from fleet/SSE reason payloads (timeout/publickey/connection refused/etc.).
  - Ensures `DeviceMatrix` can reflect target-level health degradation quickly via stream updates.

## 3) Validation Evidence

### VM prompt matrix (54 prompts)

Command:

`cargo test -p kria-core --test cognitive_e2e_tests cognitive_vmtestprompts_matrix -- --nocapture`

Result:

- Loaded 54 prompt cases from `VMTestPrompts.txt`
- **VMTestPrompts score: 100.0% (54/54)**

### Zone 0 check

Command:

`npm run check` (from `ui/`)

Result:

- TypeScript pre-flight passed, no syntax/type regressions from this work.

### Full suite gate

Command:

`KRIA_REAL_LLM=1 KRIA_TEST_ALLOW_DESTRUCTIVE=1 cargo kria-test --mode FULL`

Result:

- Full suite passed; Zone 0 passed.
- VM destructive/chaos zones were skipped due VM reachability probe state in this environment.

## 4) Prompt-to-Reality Chain Status

Requested chain:

Natural Prompt -> Intent -> Target Resolve -> HMAC Signed -> Remote Execute -> Output

Current status:

- **Intent + routing:** verified (54/54)
- **Target resolve:** implemented for generic references and aliases/fuzzy forms
- **HMAC + dispatch pipeline:** exercised by test runner infrastructure zones (where reachable)
- **Live remote execution for all 54 prompts:** blocked in this run by VM probe reachability state reported by runner (`Reachable: false`)

## 5) Healing Loop Notes

- One VM prompt mismatch was found and fixed:
  - `VM-07 "Remote command: df -h"` previously routed to `complex_task`
  - now routed to `execute_fleet_command`
- SSH key/sudo remediation was not auto-applied in this pass because full live VM execution for all 54 prompts could not be completed in current connectivity state.

## 6) Success Rate Summary

- **Dedicated VM prompts (`VMTestPrompts.txt`):** **100.0% (54/54)**  
- **General natural VM phrases introduced in this refactor:** **validated via router unit tests + tool wiring**

