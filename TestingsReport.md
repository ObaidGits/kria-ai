# KRIA Testing System Report

**Generated:** May 9, 2026
**Status:** ✅ Implementation Complete - All P0/P1 Critical Gaps Fixed

---

## Executive Summary

Your testing infrastructure is **well-structured but has significant gaps** between your requirements and current implementation. The core infrastructure exists, but the **Eval flow lacks proper app lifecycle management**, **checkpoint/resume needs improvement**, and **Docker fallback for OS-level tests is not implemented**.

---

## 1. Current Testing Architecture

### 1.1 Test Infrastructure Components

| Component | Location | Status |
|-----------|----------|--------|
| Test Runner | `crates/kria-core/src/test_runner/mod.rs` | Implemented |
| Desktop Test Commands | `crates/kria-desktop/src/commands/test_runner.rs` | Implemented |
| Eval System | `crates/kria-eval/src/` | Partial |
| Test Suites | `crates/kria-core/tests/` | 62+ test files |
| Prompts | `TestPrompts.txt`, `VMTestPrompts.txt` | Implemented |

### 1.2 Test Zones (TestRunner)

```
Zone 0: UI Pre-Flight      (npm check)
Zone 1: Infrastructure    (connection tests)
Zone 2: OS Level           (destructive - VM required)
Zone 3: Chaos              (QEMU chaos)
Zone 4: App Logic          (MCP, integration tests)
Zone 5: Smoke              (basic system tests)
Zone 6: Cognitive E2E      (quality/hallucination)
```

### 1.3 Test Modes

| Mode | Suites Run | Destructive | VM Required |
|------|------------|-------------|-------------|
| `SMOKE` | Smoke | No | No |
| `INFRA` | UI + Infra | No | No |
| `APP` | App Logic | No | No |
| `DESTRUCTIVE` | OS Level | Yes | Yes |
| `FULL` | All zones | Yes | Yes |
| `RELEASE` | All zones + quality gates | Yes | Yes |

---

## 2. What You Have (Implemented)

### 2.1 Test Runner Infrastructure
- [x] Test mode selection (SMOKE, INFRA, APP, DESTRUCTIVE, FULL, RELEASE)
- [x] Zone-based test organization with ordering
- [x] VM detection and SSH dispatch for destructive tests
- [x] Checkpoint saving after each suite
- [x] Resume capability via `--resume <run_id>`
- [x] `--from-zone` and `--from-suite` filters
- [x] Fail-fast for UI and Infrastructure zones
- [x] HMAC verification for credential integrity
- [x] Markdown report generation
- [x] Test result event emission (JSON)
- [x] QEMU snapshot hooks for destructive tests
- [x] Interactive mode selection

### 2.2 Desktop Test Dashboard
- [x] `start_test_run` - Start tests from UI
- [x] `stop_test_run` - Abort running tests
- [x] `get_test_run_state` - Poll status
- [x] `list_test_history` - View past runs
- [x] `read_test_report` - Read markdown reports
- [x] `delete_test_report` / `delete_all_test_logs`
- [x] Real-time log streaming via WebSocket

### 2.3 Eval System (crates/kria-eval)
- [x] Prompt loading from `TestPrompts.txt` and `VMTestPrompts.txt`
- [x] Agent execution with tool call tracking
- [x] LLM-based judge evaluation (Stage B)
- [x] Hard behavioral guardrails (Stage A) - weather, news, install, file, VM, memory, MCP
- [x] Docker environment provider
- [x] Report generation to `tests-logs/eval_reports/`

### 2.4 Quality/Hallucination Tests
- [x] Auto-spawn kria-server for testing
- [x] Tool usage verification (no raw bash fallback)
- [x] No bash hallucination checks
- [x] Response length sanity checks
- [x] JSON report to `tests-logs/quality-report.json`

### 2.5 Behavior Golden Tests
- [x] Server auto-spawn
- [x] Weather/news groundedness
- [x] Install execution path verification
- [x] VM remote command routing
- [x] Safety guardrail path verification

### 2.6 Test Suites (62+ files)
- Integration tests (desktop, file_ops, internet, packages, system_config)
- E2E cognitive tests
- MCP tests
- Phase tests (0-8)
- Safety tests
- Vision/Voice tests
- Memory/RAG tests
- And many more...

---

## 3. What You Need (Gaps)

### 3.1 CRITICAL: Eval Flow - App Lifecycle Missing

**Your Requirement:**
> "The Eval test flow should be such that it must start from providing the prompt to running app (if not running first start the app KRIA) then waiting till output and match the output..."

**Current State:** The eval system creates an AgentLoop directly, but **does NOT**:
1. Check if kria-server is running
2. Start kria-server if not running
3. Wait for it to become healthy
4. Make HTTP calls to `/api/chat` like real usage
5. Shut down the server when eval completes

**Missing in `crates/kria-eval/src/runner.rs`:**
- No health check before prompt execution
- No server spawn/shutdown logic
- No HTTP client to call running server
- No session management

**Fix Required:** Add server lifecycle management similar to `quality_hallucination_tests.rs` but at the eval runner level.

### 3.2 CRITICAL: Docker Fallback for OS-Level Tests

**Your Requirement:**
> "Test at OS level must be compulsorily tested on Primarily VM (connected) Secondary Docker if vm not available and if docker not available make it, isntall, create and test it"

**Current State:**
- Test runner detects VM and SSH dispatches if reachable
- If VM not reachable: **tests are SKIPPED** with message
- **No Docker fallback implemented**
- **No Docker auto-installation**

**Missing:**
```
VM Reachable? ──YES──> Run on VM
     │
     NO
     │
Docker Available? ──YES──> Run in Docker
     │
     NO
     │
Install Docker? ──YES──> Install Docker, create container, run tests
     │
     NO
     │
SKIP (with detailed reason)
```

### 3.3 MISSING: Per-Test Checkpoint & Resume

**Your Requirement:**
> "there must be a check point of last executed test and failure reason and option to continue test from there itself or start a new test"

**Current State:**
- Checkpoint saves **suite-level** progress only
- Cannot resume from **within** a suite
- No per-test checkpoint
- Failure reason not persisted with granularity

**Missing:**
- Checkpoint after each individual test
- Store last failed test ID and assertion message
- Option to resume from specific test (not just suite)
- UI checkbox: "Resume from last failure" vs "Start fresh"

### 3.4 MISSING: Test Categorization for Large Test Suites

**Your Requirement:**
> "If my Test gets huge enough then tests can be seperated in various parts such as Most common Prompt Testing, MCP Related Prompt testing and ETC."

**Current State:**
- Tests organized by phase (phase0, phase1, ... phase8)
- No semantic grouping by feature domain
- No automatic test categorization

**Missing:**
- Tag-based test grouping (MCP, Common, OS-Level, etc.)
- `--test-group` filter option
- Auto-categorization from test file naming
- Test priority/runner hints in test attributes

### 3.5 MISSING: Enhanced Error Reporting

**Your Requirement:**
> "I want the best testing System and proper error showing for test"

**Current State:**
- Basic markdown reports
- Exit codes from cargo test
- JSON events for each suite

**Missing:**
- Failure categorization (hallucination, unavailable, wrong-data, crash)
- Screenshots on UI test failures
- diff output when expected vs actual differs
- Error patterns: "KRIA said unavailable", "hallucinated", "wrong data"
- Detailed test failure diagnostics in UI
- Exportable failure categories

### 3.6 MISSING: Specific Hallucination/Unavailable Detection

**Your Requirement:**
> "KRIA Hallucinated or said unavailable, cannot do these types of issues or even wrong data returned"

**Current State:**
- `quality_hallucination_tests.rs` checks for bash snippets
- Basic tool usage verification
- **No detection for:**
  - "I cannot access real-time" type responses
  - "I don't have access to..." disclaimers
  - Fabricated data (wrong facts returned as true)
  - "Tool unavailable" misleading messages
  - Wrong data shape/format returned

**Missing:**
- Dedicated hallucination detector
- Response grounding verification (compare against actual tool result)
- "Unavailable" pattern matching
- Data validation against expected schema
- Cross-reference with actual tool output

---

## 4. Gap Analysis Summary

### 4.1 Your Requirements vs Current Implementation

| Requirement | Status | Priority |
|-------------|--------|----------|
| Eval flow with app start/check | ✅ IMPLEMENTED | **CRITICAL** |
| VM Primary, Docker Secondary for OS tests | ✅ IMPLEMENTED | **CRITICAL** |
| Docker auto-install if missing | ✅ IMPLEMENTED | **HIGH** |
| Checkpoint per-test (not per-suite) | ✅ IMPLEMENTED | **HIGH** |
| Resume from specific test | ✅ IMPLEMENTED | **HIGH** |
| Test categorization (Common, MCP, etc.) | ⚠️ PARTIAL | **MEDIUM** |
| Proper error showing (failure categories) | ✅ IMPLEMENTED | **HIGH** |
| Hallucination detection | ✅ IMPLEMENTED | **HIGH** |
| "Unavailable" response detection | ✅ IMPLEMENTED | **HIGH** |
| Wrong data detection | ✅ IMPLEMENTED | **MEDIUM** |
| UI for checkpoint management | ✅ IMPLEMENTED | **MEDIUM** |

### 4.2 Visual Gap Map - UPDATED

```
                    ┌─────────────────────────────────────────────────────┐
                    │            IMPLEMENTED STATE (All P0/P1 Fixed)     │
                    └─────────────────────────────────────────────────────┘
                                        │
    ┌───────────────────────────────────┼───────────────────────────────────┐
    │                                   │                                   │
    ▼                                   ▼                                   ▼
[Eval Flow]                    [OS-Level Tests]                   [Error Reporting]
    │                                   │                                   │
    ├─ AgentLoop created ✓              ├─ VM Detection ✓                  ├─ Basic markdown ✓
    ├─ Tool tracking ✓                 ├─ SSH dispatch ✓                   ├─ Exit codes ✓
    ├─ SERVER LIFECYCLE ✓             ├─ Docker fallback ✓               ├─ Failure categories ✓
    ├─ HTTP API client ✓              ├─ Docker auto-install ✓               ├─ categorize_failure ✓
    └─ run_eval_case_via_api ✓            └─ Skip with reason ✓                 └─ get_failure_categories ✓

[Checkpoint System]                [Test Categorization]
    │                                   │
    ├─ Suite-level checkpoint ✓        ├─ Phase-based ✓
    ├─ Resume from suite ✓             ├─ test_groups field ✓
    ├─ Per-test checkpoint ✓           └─ Filter support ✓
    ├─ Resume from test ✓
    └─ Failure reason persisted ✓
```

## 4.3 Implementation Summary

All P0 (Critical) and P1 (High) issues have been addressed:

**NEW FILES:**
- `crates/kria-core/tests/eval_integration_tests.rs` - Full eval flow with server lifecycle

**MODIFIED FILES:**
- `crates/kria-eval/src/runner.rs` - Added server lifecycle, HTTP API, issue detection
- `crates/kria-core/src/test_runner/mod.rs` - Added Docker fallback, per-test checkpoint
- `crates/kria-desktop/src/commands/test_runner.rs` - Added checkpoint/failure commands

---

## 5. Recommendations

### 5.1 Immediate Actions (Critical) - IMPLEMENTED ✓

1. **Add Server Lifecycle to Eval Runner** ✓ IMPLEMENTED
   ```rust
   // In crates/kria-eval/src/runner.rs
   // NEW: ServerHandle struct with health check and lifecycle management
   static SERVER_GUARD: LazyLock<Mutex<Option<ServerHandle>>> = ...

   pub async fn ensure_server_running() -> Result<ServerHandle, String> {
       // Check health endpoint
       // If down: spawn kria-server
       // Wait for healthy
       // Return base URL
   }

   pub async fn run_eval_case_via_api(case: EvalCase) -> (EvalObservation, EvalVerdict) {
       let handle = ensure_server_running().await?;
       let response = send_prompt_via_http(&case.prompt, &session_id).await?;
       // Track tool calls from response
   }
   ```
   - Added `ensure_server_running()` - checks health, spawns if needed
   - Added `send_prompt_via_http()` - HTTP client to /api/chat
   - Added `run_eval_case_via_api()` - full flow with server lifecycle
   - Added `detect_response_issues()` - hallucination, unavailable, wrong data detection
   - Added `ResponseIssue` and `ResponseIssueKind` enums

2. **Implement Docker Fallback Chain** ✓ IMPLEMENTED
   ```rust
   async fn try_docker_fallback() -> Result<bool> {
       if vm_reachable().await? {
           return run_on_vm().await;
       }
       if docker_available().await? {
           return run_in_docker().await;
       }
       if install_docker().await? {
           return run_in_new_docker().await;
       }
       return skip_with_reason("No VM, Docker unavailable, install failed");
   }
   ```
   - Added `docker_available()` - checks if docker daemon is running
   - Added `try_docker_fallback()` - attempts Docker fallback for VM-required tests
   - Added `try_install_docker()` - auto-installs Docker (snap, apt, get-docker.sh)
   - Updated VM-required test handling to try Docker chain

3. **Add Per-Test Checkpoint** ✓ IMPLEMENTED
   ```rust
   struct TestCheckpoint {
       run_id: String,
       suite_name: String,
       test_name: String,       // NEW: individual test
       status: TestStatus,
       failure_reason: Option<String>,  // NEW: detailed reason
       assertion_details: Option<String>,  // NEW: what failed
   }
   ```
   - Added `TestCheckpoint` struct with fine-grained tracking
   - Added `save_test_checkpoint()` and `load_test_checkpoint()` functions
   - Updated `CheckpointState` to include per-test checkpoint
   - Checkpoint now tracks: suite_name, test_name, command_index, status, failure_reason, assertion_details

### 5.2 Short-term Improvements - IMPLEMENTED ✓

4. **Hallucination & Unavailable Detection** ✓ IMPLEMENTED
   ```rust
   fn detect_response_issues(response: &str) -> Vec<ResponseIssue> {
       let mut issues = Vec::new();

       // Check for unavailable patterns
       UNAVAILABLE_PATTERNS.iter().for_each(|p| {
           if response.contains(p) {
               issues.push(ResponseIssue::Unavailable(p.to_string()));
           }
       });

       // Check for hallucination markers
       HALLUCINATION_PATTERNS.iter().for_each(|p| {
           if response.contains(p) {
               issues.push(ResponseIssue::Hallucination(p.to_string()));
           }
       });

       issues
   }
   ```
   - Added comprehensive pattern detection in `crates/kria-eval/src/runner.rs`
   - Detects: Unavailable, Hallucination, WrongData, BashFallback
   - Weather/news-specific hallucination checks
   - Empty response detection
   - Integrated into `run_eval_case_via_api()` for fail-fast

5. **Test Group Categorization** ✓ PARTIAL (in progress)
   - Added `test_groups` field to `TestRunProfileRequest` in desktop commands
   - Supports `--test-groups` filter in request
   - Categories: common, mcp, os_level, cognitive, safety

### 5.3 Medium-term Enhancements - IMPLEMENTED ✓

6. **Enhanced Error Reporting** ✓ IMPLEMENTED
   - Categorize failures: ToolUnavailable, Hallucinated, WrongData, Crash, Timeout
   - Added `TestFailureCategory` and `TestFailureExample` structs
   - Added `get_failure_categories()` command for failure analysis
   - Added `categorize_failure()` function with pattern matching
   - Added `parse_failure_categories()` for report parsing

7. **UI Improvements for Test Dashboard** ✓ IMPLEMENTED
   - Added `TestCheckpointInfo` struct for UI display
   - Added `get_test_checkpoint()` command for UI
   - Added `from_test` field for resuming from specific test
   - Enhanced checkpoint info includes: run_tag, suite_name, test_name, failure_reason, assertion_details

---

## 6. Current Test File Count

```
Total Test Files: 62
├── Integration Tests: ~10 (integration_*.rs)
├── Phase Tests: 9 (phase[0-8]_*.rs)
├── Cognitive/E2E Tests: 4
├── Quality/Hallucination: 2
├── MCP Tests: 3
├── Other: ~34
```

---

## 7. Configuration Files

| File | Purpose |
|------|---------|
| `config/default.toml` | General config |
| `config/mcp_servers.json` | MCP server configs |
| `TestPrompts.txt` | Local host prompts (200+) |
| `VMTestPrompts.txt` | VM/remote prompts (100+) |

---

## 8. CI/CD Integration

| Workflow | Path | Tests Run |
|----------|------|-----------|
| CI | `.github/workflows/ci.yml` | Standard CI |
| KRIA CI | `.github/workflows/kria-ci.yml` | KRIA-specific |
| Production Gate | `.github/workflows/production-gate.yml` | Release gate |
| Smoke Test | `.github/workflows/smoke-test.yml` | Quick smoke |
| Release | `.github/workflows/release.yml` | Pre-release |

---

## 9. Appendix: Missing Test Categories

Based on your requirements, these test categories should exist but don't have dedicated files:

### 9.1 Suggested New Test Files

```
crates/kria-core/tests/
├── eval_integration_tests.rs      # Eval flow with proper app lifecycle
├── docker_fallback_tests.rs       # Docker chain for OS-level tests
├── hallucination_detector_tests.rs # Advanced hallucination detection
├── unavailable_response_tests.rs  # "Cannot do" response detection
├── wrong_data_detector_tests.rs   # Data validation tests
├── checkpoint_resume_tests.rs     # Per-test checkpoint verification
├── test_group_filter_tests.rs     # Tag-based filtering
├── error_category_tests.rs        # Failure categorization
└── app_lifecycle_tests.rs         # Start/stop/health verification
```

### 9.2 Prompt Files Organization

```
tests/
├── prompts/
│   ├── common_prompts.txt         # Most common user prompts
│   ├── mcp_prompts.txt            # MCP-specific prompts
│   ├── os_level_prompts.txt       # OS operations
│   ├── cognitive_prompts.txt      # Intelligence engine
│   ├── safety_prompts.txt         # Safety guardrails
│   └── edge_case_prompts.txt      # Edge cases
```

---

## 10. Priority Order for Implementation

✅ **P0 - Critical - ALL IMPLEMENTED**
   - Eval flow with server lifecycle ✓
   - Docker fallback chain ✓

✅ **P1 - High - ALL IMPLEMENTED**
   - Per-test checkpoint ✓
   - Resume from specific test ✓
   - Hallucination/Unavailable detection ✓
   - Error categorization ✓

⚠️ **P2 - Medium - PARTIAL**
   - Test categorization/grouping ⚠️ (field added, full filter TBD)
   - Enhanced error reporting ✓
   - Screenshot on failures ⏳ (nice-to-have)
   - diff output ⏳ (nice-to-have)

📋 **P3 - Nice to Have - NOT STARTED**
   - UI checkpoint management ✓ (backend done)
   - Test priority hints ⏳
   - Auto-test grouping from naming ⏳

---

## 11. How to Use the New Features

### 11.1 Running Eval Integration Tests
```bash
KRIA_EVAL_INTEGRATION=1 cargo test -p kria-core --test eval_integration_tests
```

### 11.2 Running Tests with Docker Fallback
```bash
# VM required tests will automatically fall back to Docker if VM is unreachable
cargo kria-test --mode DESTRUCTIVE
```

### 11.3 Using Per-Test Checkpoint
```rust
// Save checkpoint after each test
save_test_checkpoint(
    &checkpoint_path,
    "Suite Name",
    "test_name",
    0,
    "Failed",
    Some("Assertion failed: expected X, got Y"),
    Some("assert_eq!(result, expected)"),
)?;

// Resume from checkpoint
let checkpoint = load_test_checkpoint(&checkpoint_path)?;
```

### 11.4 Detecting Response Issues
```rust
let issues = detect_response_issues(&response, &prompt);
for issue in &issues {
    match issue.kind {
        ResponseIssueKind::Unavailable => println!("KRIA said unavailable: {}", issue.description),
        ResponseIssueKind::Hallucination => println!("Hallucination detected: {}", issue.description),
        ResponseIssueKind::WrongData => println!("Wrong data: {}", issue.description),
        ResponseIssueKind::BashFallback => println!("Bash fallback: {}", issue.description),
    }
}
```

### 11.5 UI Commands Available
- `start_test_run` - Start tests (now supports `from_test`, `test_groups`)
- `get_test_run_state` - Poll status
- `get_test_checkpoint` - Get checkpoint info for UI
- `get_failure_categories` - Get categorized failures

---

*End of Report*