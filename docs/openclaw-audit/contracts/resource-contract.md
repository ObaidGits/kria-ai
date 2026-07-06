# Resource Contract (FROZEN — Phase A0)

> INV-5: **all** OpenClaw execution is admitted and governed by the Hardware & Resource Authority
> (HRA). No subsystem self-allocates CPU/RAM/GPU. OpenClaw is a registered HRA consumer alongside
> voice (STT/TTS) and vision — the same authority the user's working GPU orchestrator already uses.

## 1. The one request object

```rust
struct ResourceRequest {
    cpu_millis:   u32,      // 500 = 0.5 core
    memory_mb:    u32,
    gpu:          GpuNeed,  // None | Shared{mb} | Exclusive{mb}
    storage_mb:   u32,      // workspace/tmpfs ceiling
    network:      bool,     // whether an egress path is needed (domains come from caps)
    priority:     Priority, // RealtimeVoice > InteractiveFg > Vision > OpenClawInteractive > Batch
    class:        ResourceClass,   // light|medium|heavy (existing hint)
    correlation_id: Uuid,
}
```

Derived from `manifest.resource` + granted capabilities. One object, from checkout to audit.

## 2. Admission lifecycle (frozen)

```text
execute()
  → HRA.admit(ResourceRequest)
      ├─ Granted(Lease)   → proceed to launch (execution-contract)
      ├─ Queued(position) → bounded queue with aging; emit SkillEvent::Waiting
      └─ Denied(reason)   → return typed "busy" result (never hard-crash the turn)
Run (Lease held == instance alive; the two are bound)
  → on cancel/timeout/complete/crash: release Lease  (ALWAYS, exactly once)
```

- **Lease ↔ instance are bound**: releasing a lease tears down the instance; destroying the
  instance releases the lease. Fixes today's leak + non-propagated cancellation.

## 3. Priority ladder (frozen, extends existing HRA classes)

```
RealtimeVoice (STT/TTS)  >  InteractiveForeground (chat)  >  Vision  >
OpenClawInteractive  >  OpenClawBatch/Scheduled
```

- OpenClaw never preempts realtime voice. Voice/vision may preempt OpenClaw batch jobs.
- OpenClaw has a **minimum reserved floor** + queue **aging** so it cannot be starved forever.

## 4. GPU (frozen rule)

- GPU is granted only via HRA `GpuOwner::OpenClaw` lease, mirroring the vision/voice model
  (shadow → enforce cutover pattern already in the codebase).
- GPU + network are independent grants; a GPU skill is not automatically networked.
- On GPU denial: skill either queues or fails cleanly with a typed reason — never runs ungoverned.

## 5. Cancellation, preemption, retry, recovery (frozen)

- **Cancellation:** cooperative signal → forced teardown after grace; lease released; `Cancelled`
  event. Wired to `global_halt` (phone kill-switch too).
- **Preemption:** higher-priority admission may reclaim an OpenClaw batch lease; the preempted
  instance receives cancel + `Preempted` recovery (retryable when capacity returns).
- **Retry:** bounded, backoff, only for transient failures (admission timeout, worker blip).
  Never for policy denials or skill logic errors.
- **Recovery:** typed `Failure` → `Retry | Rollback | Fail` (execution-contract §2).

## 6. Accounting (frozen: cost is recorded)

Every invocation records actual usage into the audit/event record: `cpu_ms`, `peak_mem_bytes`,
`gpu_ms` (if any), `storage_peak`, wall latency, queue-wait. Sampled from cgroup/runtime stats at
cleanup. Enables budgeting, dashboards, and the master-roadmap cost metrics.

## 7. Multi-host (forward-compat)

- Remote/GPU workers admit against a **remote HRA view** via signed leases
  (`kria-connection-control`). The `ResourceRequest`/`Lease` shape is identical; only the authority
  instance differs. Distributed scheduling is thus additive, not a redesign.

## 8. Self-review (challenge)

- *"HRA admission adds latency to every skill."* → Admission is in-process and O(1) for the common
  granted case; only contention hits the queue. Warm-pool + WASM keep the fast path fast.
- *"Binding lease↔instance is over-strict for pooled warm containers."* → Warm/idle containers hold
  **no** lease; a lease is acquired at checkout and released at checkin. Pooling and leasing are
  orthogonal. No double-accounting.
- *"Priority floor for OpenClaw could hurt voice on tiny hosts."* → Floor is tier-scaled and can be
  zero on low-tier hosts (voice absolute priority). Configurable, HRA-owned.
- *"Remote HRA view could diverge from local."* → Leases are authoritative and time-bounded;
  divergence self-heals on lease expiry. The worker cannot exceed its lease.
- *"GpuNeed granularity."* → Shared vs Exclusive covers current needs; finer partitioning (MIG) is
  ⚠ evolvable behind the same enum.

**Frozen:** single `ResourceRequest`/`Lease`, mandatory HRA admission for all execution, lease↔
instance binding, priority ladder with OpenClaw below realtime, cancellation/preemption releases
lease, cost accounting recorded, remote admission via signed leases.
**May evolve (⚠):** exact budgets/floors, queue aging policy, GPU partitioning granularity,
cgroup tunings.
