# RFC-003: Target Inventory Pooling

Status: Implemented
Author: TBD
Created: 2026-05-05
Depends On: RFC-002 Remote QEMU Execution

## 1. Context

RFC-002 provides a hardened single-target remote execution runtime. This RFC extends the model to a multi-target execution fleet with explicit inventory, leasing, quarantine, and health-gated re-entry.

## 2. Problem Statement

Per-target operation limits throughput and resilience. A pooled model must guarantee deterministic routing and fail-closed safety while scaling command admission and reset recovery.

## 3. Design Goals

- Introduce a first-class TargetPool lifecycle manager for multiple QemuSshEnvironment instances.
- Define a bounded lease protocol with TargetId, LeaseId, TTL, and heartbeat requirements.
- Add strict quarantine semantics for degraded or lease-failed targets.
- Implement weighted target selection using health, latency, and recent failure rate.
- Preserve RFC-001 contracts for upstream tool execution paths.

## 4. Non-Goals

- Cross-region scheduler federation.
- Multi-tenant identity and authorization.
- Hypervisor abstraction beyond the RFC-002 QEMU provider family.

## 5. Core Types And State Model

### 5.1 Identity

- TargetId: stable UUID identity for a registered target.
- LeaseId: UUID identity for a lease grant.

### 5.2 Runtime States

- Ready: target is healthy and lease-eligible.
- Leased: target is actively assigned to one lease.
- Quarantine: target is blocked from selection until health gates pass.

### 5.3 Telemetry Inputs

Each target maintains rolling telemetry:

- health_score in [0, 1]
- latency_ewma_ms >= 0
- recent_failure_rate in [0, 1]

## 6. Lease Protocol

### 6.1 Acquire

- acquire_lease MUST return TargetId + LeaseId + expires_at + heartbeat_ttl.
- acquire_lease MUST only select from Ready targets.
- acquire_lease MUST fail with ProviderUnavailable when no Ready target exists.

### 6.2 Heartbeat

- heartbeat(LeaseId) MUST refresh expires_at to now + lease_ttl.
- heartbeat after ttl + grace MUST fail-closed:
  - lease removed
  - target tainted
  - target enters Quarantine

### 6.3 Release

- release_lease MUST clear lease ownership and return target to Ready if not quarantined.

### 6.4 Expired Lease Reaping

- Pool MUST periodically reap expired leases.
- Reaped leases MUST trigger the same fail-closed quarantine behavior as heartbeat failure.

## 7. Selection Policy

Ready targets are ranked by weighted score:

$$
score = w_h \cdot health + w_l \cdot \frac{1}{1 + latency/100} + w_f \cdot (1 - failure)
$$

Where default weights are:

- $w_h = 0.50$
- $w_l = 0.30$
- $w_f = 0.20$

Policy rules:

- Scores are computed only for Ready targets.
- Highest score wins.
- Inputs are clamped into legal ranges before scoring.

## 8. Quarantine And Health-Gate Re-Entry

### 8.1 Quarantine Triggers

- Lease heartbeat expiry.
- Explicit degradation marker from orchestration/runtime policy.
- Critical provider errors requiring operator or automated recovery.

### 8.2 Required Health Gates

Targets in Quarantine MUST pass all configured probes to re-enter Ready:

- ensure_ready probe: provider readiness checks complete.
- admission_barrier probe: no inflight admissions and empty inflight registry.

### 8.3 Re-Entry Rules

- Probes execute only after quarantine cooldown elapses.
- Any probe failure increments failed_probe_count and keeps target quarantined.
- Success clears taint and transitions target to Ready.

## 9. Contract Preservation

RFC-001 compatibility is preserved by design:

- CommandExecutor/FileSystemOps/EnvironmentLifecycle contracts remain unchanged.
- TargetPool is an orchestration layer around existing environment providers.
- Upstream tools continue invoking provider-compatible surfaces.

## 10. Telemetry Packet Specification

TargetPool MUST emit structured telemetry packets with at least:

- timestamp_unix_ms
- event
- total_targets
- ready_targets
- leased_targets
- quarantined_targets
- active_leases
- expired_lease_count

Required events:

- lease_acquired
- lease_released
- target_quarantined
- quarantine_exit_ready

## 11. Failure Handling Semantics

- Unknown lease heartbeat is treated as invalid session state and rejected.
- Heartbeat timeout is fail-closed and taints the target immediately.
- Probe failures never auto-bypass quarantine.

## 12. Rollout Plan

- Phase A: register targets + telemetry-only occupancy.
- Phase B: enable lease gating and fail-closed lease heartbeat enforcement.
- Phase C: enable quarantine probes and weighted selection as default.

## 13. Open Questions

- Sticky affinity policy for long-running conversations.
- Adaptive lease TTL by target reliability tier.
- Fleet-wide admission backpressure behavior when Ready capacity is low.

## 14. Checklist

- [x] Finalize TargetPool API and state transitions.
- [x] Integrate lease events into operator dashboard.
- [x] Add integration tests for lease expiry fail-closed quarantine.
- [x] Add chaos tests for flapping targets and mass lease expiry.
- [x] Define migration path for single-target deployments.
