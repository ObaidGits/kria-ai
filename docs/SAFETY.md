# KRIA Safety Model

> **Last Updated:** 2026-05-11
> **Status:** Production

---

## Executive Summary

KRIA's safety system ensures all tool executions pass through a centralized policy engine before reaching the system. The model is **fail-closed**: if policy cannot be evaluated, execution is denied. Every state-changing action is logged to an append-only audit trail.

---

## Risk Classification

### Tier Overview

| Tier | Behavior | Examples |
|------|----------|----------|
| **Green** | Auto-execute, no approval | Read-only queries, status checks, search |
| **Yellow** | Execute + notify user (post-hoc) | Reversible writes, `set_volume`, `write_file` (safe paths) |
| **Red** | Block, require PIN approval | Destructive actions, `delete_file`, `install_package` |
| **Black** | Always denied | `rm -rf /`, writing to `/etc`, reading `~/.ssh/id_rsa` |

### Green Actions (Auto-Execute)

Read-only, trivially reversible, or purely informational:

- System info: `get_cpu_usage`, `get_memory_info`, `get_disk_space`
- File reading: `read_file`, `search_files`, `list_directory`
- Internet: `web_search`, `fetch_webpage`, `get_weather`
- Knowledge: `recall_fact`, `search_knowledge`, `get_snippet`
- Notifications: `send_notification`, `compose_email` (draft only)

### Yellow Actions (Execute + Notify)

Reversible writes that don't require pre-approval:

- Settings: `set_volume`, `set_brightness`
- File writes: `write_file` (in safe paths only)
- Communication: `send_email`, `gw_gmail_send`
- Process: `kill_process` (for user-owned apps)

### Red Actions (PIN Required)

Destructive or high-impact operations:

- File operations: `delete_file`, `move_file` (system paths)
- Package management: `install_package`, `uninstall_package`
- System: `shutdown`, `reboot`
- Network: `firewall_rule_add`

### Black Actions (Always Denied)

Operations that are never allowed:

- Writing to system directories: `/etc`, `/boot`, `/root`, `/usr`
- Reading sensitive files: `~/.ssh/id_rsa`, `~/.gnupg`
- Destructive shell patterns: `rm -rf /`, `dd if=/dev/zero`

---

## Policy Engine

### Evaluation Flow

```rust
impl PolicyEngine {
    pub fn evaluate(&self, tool: &str, params: &Value) -> RiskLevel {
        // 1. Check BLACK_ACTIONS (always deny)
        if BLACK_ACTIONS.contains(tool) {
            return RiskLevel::Black;
        }

        // 2. Check parameter-dependent rules
        if let Some(level) = self.evaluate_params(tool, params) {
            return level;
        }

        // 3. Check static tier sets
        if GREEN_ACTIONS.contains(tool) {
            return RiskLevel::Green;
        }
        if YELLOW_ACTIONS.contains(tool) {
            return RiskLevel::Yellow;
        }
        if RED_ACTIONS.contains(tool) {
            return RiskLevel::Red;
        }

        // 4. Default to Yellow (conservative)
        RiskLevel::Yellow
    }
}
```

### Parameter-Dependent Rules

Some tools have tier based on parameters:

```rust
fn evaluate_params(&self, tool: &str, params: &Value) -> Option<RiskLevel> {
    match tool {
        "write_file" => {
            let path = params.get("path")?.as_str()?;
            if is_protected_path(path) {
                return Some(RiskLevel::Black);
            }
            if is_system_path(path) {
                return Some(RiskLevel::Red);
            }
            Some(RiskLevel::Yellow)
        }
        "run_command" => {
            let cmd = params.get("command")?.as_str()?;
            if contains_destructive_pattern(cmd) {
                return Some(RiskLevel::Black);
            }
            Some(RiskLevel::Red)
        }
        _ => None,
    }
}
```

---

## Human-In-The-Loop (HITL)

### Approval Flow

```
1. Tool called with Red tier
2. AgentLoop pauses execution
3. ApprovalRequest event emitted to UI
4. User sees: tool name, params, risk summary
5. User responds: Approved (with PIN) | Denied | Timeout
6. If Approved: execution continues
7. If Denied/Timeout: ToolResult::err("denied by user")
```

### Approval Request Schema

```rust
pub struct ApprovalRequest {
    pub id: Uuid,
    pub tool_name: String,
    pub params: serde_json::Value,
    pub risk_level: RiskLevel,
    pub risk_summary: String,
    pub timeout_secs: u32,
    pub created_at: Instant,
}

pub enum ApprovalResponse {
    Approved { pin: String },
    Denied,
    Timeout,
}
```

### PIN Requirements

- 4-digit numeric PIN set by user
- Required for every Red-tier action (never cached)
- Stored hashed in `~/.kria/config.toml`

---

## Audit Logging

### Audit Entry Schema

```rust
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub tool_name: String,
    pub params_hash: String,      // SHA-256 (not full params)
    pub risk_level: RiskLevel,
    pub approved: bool,
    pub user_response: Option<ApprovalResponse>,
    pub result_summary: String,
    pub duration_ms: u64,
}
```

### Storage

- SQLite table: `audit_log`
- Append-only (no UPDATE/DELETE)
- Retention: 90 days (configurable)
- Location: `~/.kria/audit.db`

---

## Rollback Manager

### Supported Rollbacks

| Tool | Rollback Action |
|------|-----------------|
| `write_file` | Restore from backup |
| `delete_file` | Restore from trash |
| `install_package` | Uninstall package |
| `set_volume` | Restore previous level |

### Rollback Flow

```rust
impl RollbackManager {
    pub fn create_snapshot(&self, tool: &str, params: &Value) 
        -> Result<RollbackSnapshot, RollbackError>;

    pub fn rollback(&self, snapshot_id: Uuid) 
        -> Result<(), RollbackError>;
}
```

---

## Path Policy

### Protected Paths (Black)

```rust
static PROTECTED_PATHS: &[&str] = &[
    "/etc",
    "/boot",
    "/root",
    "/usr",
    "/var",
    "/proc",
    "/sys",
    "~/.ssh",
    "~/.gnupg",
    "~/.config/ssh",
];
```

### Safe Writable Paths (Yellow)

```rust
static SAFE_WRITABLE_PATHS: &[&str] = &[
    "~/Documents",
    "~/Downloads",
    "~/Desktop",
    "~/Pictures",
    "~/Videos",
    "~/Music",
    "/tmp",
];
```

---

## Safety Invariants

1. **Fail-closed**: If policy cannot be evaluated, deny execution
2. **No caching**: Red-tier approvals never cached
3. **Audit always**: All tool calls logged, regardless of tier
4. **Parameter validation**: All inputs validated at boundary
5. **Path restrictions**: System paths protected by default
6. **Timeout enforcement**: Red-tier approvals timeout after 30s

---

## Configuration

```toml
[safety]
# Require PIN for Red-tier actions
require_pin = true

# Approval timeout (seconds)
approval_timeout = 30

# Audit retention (days)
audit_retention_days = 90

# Enable rollback snapshots
enable_rollback = true

# Protected paths (additional)
protected_paths = ["/custom/protected"]

# Safe writable paths (additional)
safe_writable_paths = ["/custom/safe"]
```

---

## Related Documentation

- **TOOLS.md** — Tool system and handler implementation
- **OPENCLAW.md** — OpenClaw audit ledger
- **ARCHITECTURE.md** — System architecture
