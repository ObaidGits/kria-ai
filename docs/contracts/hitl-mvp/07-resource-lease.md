# KRIA Resource Lease Spec

**Document status:** MVP lease semantics  
**Purpose:** Prevent concurrent side-effecting workflows from fighting over shared resources.

---

## 1. MVP Scope

Only implement leases needed to prevent real damage:

| Resource | MVP Mode |
|---|---|
| GUI foreground | Exclusive |
| Keyboard/mouse | Exclusive, requires GUI foreground |
| Filesystem write path | Exclusive writer |
| VM/container destructive operation | Exclusive |
| External workflow ownership | Token-bound |

Do not implement cognitive, GPU, verifier, or broad scheduler leases in HITL MVP.

---

## 2. Lease Contract

```rust
struct ResourceLease {
    id: LeaseId,
    workflow_id: WorkflowId,
    attempt_id: AttemptId,
    resource_kind: ResourceKind,
    resource_id: String,
    mode: LeaseMode,
    action_hash: ActionHash,
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    heartbeat_at: DateTime<Utc>,
}
```

Lease is bound to `action_hash`. A mutated action needs a new lease.

---

## 3. Acquisition Rules

- Acquire leases before side effects.
- Use deterministic resource ordering.
- Fail fast if lease cannot be acquired.
- Do not preempt destructive operations.
- GUI typing requires immediate foreground verification after lease acquisition.

---

## 4. Release Rules

Release on:

- action success,
- action failure,
- workflow pause if no active side effect,
- abort,
- timeout recovery scan,
- task cancellation.

Use RAII guards where possible. TTL is recovery fallback, not normal release.

---

## 5. Forbidden MVP Behavior

- No lease preemption for GUI typing.
- No silent lease stealing.
- No lease renewal without heartbeat.
- No filesystem lease without canonical path.
- No external owner lease without signed/unguessable token.

---

## 6. MVP Tests

- two GUI writers cannot run together,
- keyboard lease fails without GUI foreground lease,
- two writes to same path conflict,
- VM reset conflicts with active VM command,
- stale lease expires and emits recovery/audit record,
- action hash mismatch rejects lease use.

