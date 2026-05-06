# RFC-005: Adaptive Transport QoS

Status: Implemented
Author: TBD
Created: 2026-05-05
Depends On: RFC-002 Remote QEMU Execution, RFC-003 Target Inventory Pooling

## 1. Context

RFC-002 introduced priority classes and reserved reset slots. This RFC formalizes a shared adaptive QoS scheduler that tags infra operations and applies dynamic backpressure and starvation protection.

## 2. Problem Statement

Static queue limits alone do not sufficiently protect high-priority recovery latency during bursty mixed workloads.

## 3. Design Goals

- Add explicit QoS class tagging for all infra tasks.
- Implement threshold-based control loop from queue and latency telemetry.
- Automatically reject/defer LowMaintenance work when HighRecovery SLO is breached.
- Guarantee MediumReconnect opportunities during recovery storms.
- Emit structured telemetry for occupancy and drop/defer rates.

## 4. Non-Goals

- Kernel/network device QoS configuration.
- Global distributed scheduler across datacenters.
- Replacing RFC-002 reset barrier or replay protections.

## 5. QoS Class Model

- HighRecovery: reset and safety-critical control operations.
- MediumReconnect: reconnect and session stabilization operations.
- LowMaintenance: cleanup and maintenance operations.

Operation tagging is deterministic:

- reset_environment::* -> HighRecovery
- *reconnect* or reset_environment::medium_reconnect_slot -> MediumReconnect
- otherwise -> LowMaintenance

## 6. Adaptive Controller

### 6.1 Inputs

- per-class inflight counts
- rolling high-recovery latency samples
- p95 high-recovery latency
- defer/reject counters

### 6.2 Primary Threshold

- high_recovery_slo_ms defines breach threshold.
- if high_recovery_p95_ms > slo, LowMaintenance enters protection mode.

### 6.3 Protection Mode

In protection mode, LowMaintenance MUST be deferred or rejected.

Default policy:

- alternate defer and reject outcomes
- expose retry_after on defer
- keep hard reject path for sustained overload

## 7. Starvation Control For MediumReconnect

### 7.1 Credit Mechanism

- medium_reconnect_credits are minted on successful HighRecovery completion.
- MediumReconnect can consume a credit to proceed during recovery pressure.

### 7.2 Starvation Rule

- If high_recovery_inflight > 0 and no credits remain, MediumReconnect is deferred.
- Credits are bounded by max_medium_credits.

This guarantees reconnect traffic eventually receives service and avoids permanent starvation.

## 8. Scheduler Wrapper Contract

All infra operations MUST pass through scheduler admission:

1. classify operation into QoS class.
2. request admission (accepted/deferred/rejected).
3. on accepted completion, report latency and success.

Deferred tasks MAY retry once after retry_after.

## 9. Backpressure Semantics

- Deferred: temporary soft pressure with retry hint.
- Rejected: hard pressure signal to caller.
- Both outcomes MUST be surfaced as structured failure reasons.

## 10. Telemetry Packet Specification

QoS subsystem MUST emit structured telemetry packets containing:

- timestamp_unix_ms
- high_recovery_inflight
- medium_reconnect_inflight
- low_maintenance_inflight
- high_recovery_p95_ms
- high_recovery_slo_ms
- medium_reconnect_credits
- low_drop_rate
- low_defer_rate

## 11. Contract Preservation

QoS is an admission wrapper around existing infra execution.

- CommandExecutor/FileSystemOps signatures remain unchanged.
- RFC-001 upstream interfaces are preserved.

## 12. Integration With RFC-002 Reset Path

- reset_environment operations remain HighRecovery.
- reserved reset slot policy remains authoritative.
- adaptive QoS augments admission decisions without bypassing reset barriers.

## 13. Rollout Plan

- Phase A: telemetry-only QoS snapshots.
- Phase B: admission decisions enabled with conservative defaults.
- Phase C: tuned thresholds using adversarial replay and chaos validation.

## 14. Open Questions

- Should controller evolve from threshold to hybrid PID policy?
- How should thresholds scale with target pool occupancy?
- What operator-facing SLO presets should be supported?

## 15. Checklist

- [x] Finalize default controller thresholds and bounds.
- [x] Add integration tests for low-task backpressure during SLO breach.
- [x] Add starvation tests for medium reconnect credits.
- [x] Add telemetry dashboard fields and alert policies.
- [x] Define rollback strategy for aggressive QoS tuning.
