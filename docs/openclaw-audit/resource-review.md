# OpenClaw — Resource Management Review (HRA Integration)

> CPU/RAM/GPU ownership, container scheduling, parallelism, cancellation, priority, budget,
> recovery, cleanup — and integration with the Hardware & Resource Authority (HRA).

## 1. Current state

- **HRA today** governs **GPU** admission for STT, TTS, and Vision/OmniParser
  (`resource::authority` + `resource::gpu_lease`, `GpuOwner::{Voice,Vision,...}`). On denial
  those consumers gracefully fall back to CPU. This part is mature.
- **OpenClaw containers are NOT admitted through HRA.** They receive only static per-class
  Docker limits (256M/0.5cpu, 512M/1.0, 2G/2.0) and a local semaphore of 4. There is:
  - no CPU/RAM budget shared with voice/vision,
  - no GPU path at all (net=none, no device mapping),
  - no priority, no queue, no preemption,
  - no resource-cost accounting in the audit entry.

## 2. Findings

### RES-1 (High) — No HRA admission for CPU/RAM
A Heavy skill (2G, 2 CPU) can start concurrently with a voice turn and a vision parse, with no
central arbiter. On a low-tier host this contends with realtime voice — exactly what the HRA
exists to prevent for GPU.
**Fix:** register an **OpenClaw consumer** with the HRA. Container checkout must request an
admission lease (class → CPU/RAM cost). On denial: queue (preferred) or reject with a clear
"busy" result. Voice/vision keep priority; OpenClaw is interactive-background.

### RES-2 (High) — No GPU path for skills
GPU-accelerated skills (transcode, local model, CV) are impossible: `network=none` + no
`--gpus`/device mapping. Media/Heavy profiles imply GPU-ish work but cannot use a GPU.
**Fix:** add an optional GPU grant for Verified skills, admitted via HRA `GpuOwner::OpenClaw`
with the same lease/preemption model as vision; map the device only when granted; still no
network unless separately granted.

### RES-3 (Medium) — Semaphore rejects instead of queueing
`max_concurrent_invocations` uses `try_acquire_owned` → immediate `MaxConcurrent` error. No
backpressure, no fairness, no priority.
**Fix:** bounded async queue with priority (interactive > batch), integrated with HRA
admission so global load — not just OpenClaw's local count — drives admission.

### RES-4 (Medium) — Cancellation/global_halt does not reach containers
Handler implements `execute()` only; the loop's cancellation token and `global_halt` do not
tear down an in-flight container (pipeline Defect 4). Only outer 30s timeout + `kill_on_drop`.
**Fix:** implement `execute_with_context`, hold the container handle, and on cancellation
force-remove the container immediately; release the HRA lease.

### RES-5 (Medium) — No resource-cost telemetry
Audit records `duration_ms` + `resource_class` but not actual CPU/RAM (and GPU) consumed.
Budgeting and cost dashboards (roadmap success metrics) are impossible.
**Fix:** sample cgroup stats (bollard `stats`) at checkin; record peak mem, cpu-seconds,
(gpu-seconds if granted) into the audit entry.

### RES-6 (Low) — Cleanup on abnormal exit relies on lazy probing
See SBX-3 (events.rs unused). Leases and containers can linger after a crash between checkouts.
**Fix:** event-driven recycle also releases the HRA lease.

## 3. Target resource model

```
Checkout request
   → HRA.admit(OpenClaw, class→{cpu,ram,[gpu]}, priority=interactive-bg)
       ├─ granted: lease held for invocation; container created within budget
       └─ denied:  enqueue (bounded) or return busy result (never hard-fail UX)
Run (cancellable; lease + container bound together)
Checkin → sample cgroup stats → record cost in audit → release lease → destroy/recycle
```

Priority ladder (align with existing HRA classes):
`realtime-voice (STT/TTS) > interactive-foreground (chat) > vision > OpenClaw skills > batch`.

## 4. Summary

OpenClaw is the only major execution subsystem **outside** the HRA's authority. Bringing it
under HRA admission (CPU/RAM now, optional GPU later), adding a queue with priority, wiring
cancellation to lease+container teardown, and recording real cost turns it from an
uncoordinated resource consumer into a well-behaved citizen of the same authority that already
governs voice and vision. This is additive — the HRA API and consumer pattern already exist.
