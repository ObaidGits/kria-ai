# KRIA Resource Lease Contract

**Document status:** Implementation-bound lease contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/resource_lease.rs`, `execution_gate.rs`, `resume_executor.rs`, `gui_wiring.rs`

---

## 1. Purpose

Resource leases prevent concurrent side-effecting workflows from acting on the same shared capability at the same time.

This is a small arbitration primitive. It is not a scheduler.

---

## 2. Resource Vocabulary

Current `ResourceKind` values:

```text
GuiForeground
KeyboardMouse
BrowserProfile
FilesystemPath
VmTarget
GpuModel
VerifierSlot
DelegatedWorkflow
```

The HITL MVP live gate currently declares requirements for:

- GUI foreground,
- keyboard/mouse,
- filesystem path writes,
- browser default profile,
- VM target operations.

`GpuModel`, `VerifierSlot`, and `DelegatedWorkflow` are shared vocabulary only unless a caller explicitly declares them.

---

## 3. Access Modes

```text
Read
Write
Exclusive
```

Read leases may coexist only with other reads. Write and exclusive leases conflict with active leases from other workflows for the same resource key.

---

## 4. Lease Contract

Current `ResourceLease` shape:

```rust
struct ResourceLease {
    lease_id: String,
    workflow_id: String,
    stage_id: Option<String>,
    action_hash: String,
    kind: ResourceKind,
    scope: String,
    access_mode: AccessMode,
    owner: String,
    state: OwnershipState,
    acquired_at: Instant,
    expires_at: Instant,
    preemptible: bool,
}
```

Leases are in-memory runtime claims with TTL cleanup. Persistent decision records may store lease references, but the lease manager itself does not provide durable recovery.

---

## 5. Requirement Declaration

`ExecutionGate::declare_resource_requirements` currently maps:

| Tool/action | Requirement |
|---|---|
| `type_text`, `click_mouse`, `click_element`, `press_shortcut`, `focus_window`, `drag_mouse` | `GuiForeground` exclusive `desktop:foreground` and `KeyboardMouse` exclusive `desktop:input`. |
| `release_all` | `KeyboardMouse` exclusive `desktop:input`. |
| `write_file`, `append_file`, `delete_file`, `move_file` | `FilesystemPath` write on `path`, `target`, `destination`, or `filesystem:unknown`. |
| `browser_search`, `open_url` | `BrowserProfile` write on `browser:default-profile`. |
| `execute_fleet_command`, `vm_reset`, `vm_snapshot`, `qemu_reset` | `VmTarget` exclusive on target/host/vm/default. |

---

## 6. Acquisition And Release

Callers must:

- acquire all declared requirements before side effects,
- release acquired guards on success, failure, cancellation, or early return,
- treat lease conflict as execution blocked,
- not execute after `RESOURCE_LEASE_DENIED`,
- bind lease ownership to the proposal `workflow_id`, `stage_id`, and `action_hash`.

`ResourceLeaseGuard` releases explicitly or on drop.

---

## 7. Boundaries

- The lease manager does not verify GUI focus content; it only arbitrates ownership.
- The lease manager does not canonicalize filesystem paths; callers/preflight must ensure path safety.
- The lease manager does not persist active leases across process restarts.
- Existing specialized foreground/GPU/VM managers remain authoritative for deeper domain-specific safety.

---

## 8. Required Tests

- two workflows cannot hold conflicting GUI input leases,
- same workflow can reacquire compatible leases,
- read/read can coexist,
- write/read and write/write conflict,
- filesystem write actions declare path leases,
- browser actions declare browser profile leases,
- VM operations declare exclusive VM target leases,
- lease conflict blocks resume execution.
