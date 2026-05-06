# RFC-004: VM Snapshot Orchestration

Status: Implemented
Author: TBD
Created: 2026-05-05
Depends On: RFC-002 Remote QEMU Execution, RFC-003 Target Inventory Pooling

## 1. Context

RFC-002 reset FSM currently performs hard recovery steps. This RFC introduces snapshot primitives to provide a fast restore path with strict integrity and drift checks.

## 2. Problem Statement

Hard reset is reliable but slower and can increase disruption under fault bursts. Recovery must prioritize fast restore while preserving fail-closed safety.

## 3. Design Goals

- Provide provider-level primitives: create_snapshot, verify_integrity, restore_snapshot.
- Bind every snapshot to a toolchain fingerprint and cryptographic digest.
- Integrate snapshot restore into reset_environment as primary fast-path.
- Detect post-restore drift beyond configured tolerance.
- Preserve RFC-001 and RFC-002 safety contracts.

## 4. Non-Goals

- Cross-hypervisor snapshot portability.
- Long-term archival or deduplicated object storage.
- Cross-host live migration.

## 5. Snapshot Data Model

### 5.1 SnapshotId

- UUID identifier for immutable snapshot records.

### 5.2 Metadata Fields

Each snapshot metadata document MUST include:

- snapshot_id
- target_instance_id
- created_unix_ms
- toolchain_fingerprint
- digest_sha256
- baseline_fingerprint

### 5.3 Payload Fields

Snapshot payload stores provider-state recovery anchors:

- generation
- epoch_uuid
- transport_generation_id
- toolchain_fingerprint
- baseline_fingerprint

## 6. Integrity And Fail-Closed Rules

### 6.1 Digest Verification

- verify_integrity MUST recompute SHA-256 over payload bytes.
- restore_snapshot MUST fail-closed on digest mismatch.

### 6.2 Toolchain Fingerprint Enforcement

- restore_snapshot MUST reject snapshots whose toolchain fingerprint differs from active runtime fingerprint.
- Fingerprint mismatch MUST taint environment immediately.

### 6.3 Fail-Closed Behavior

On integrity or fingerprint failure:

- tainted=true MUST be asserted.
- taint_reason MUST record failure details.
- restore_snapshot MUST return EnvironmentResetFailed.

## 7. Drift Detection

### 7.1 Runtime Fingerprint

Provider runtime fingerprint is computed from hashed normalized provider state, including:

- generation and epoch
- transport_generation_id
- taint/admission flags
- inflight and staging indices
- toolchain fingerprint

### 7.2 Drift Metric

Drift is normalized hash distance:

$$
drift = \frac{mismatched\_chars}{min(len(hash_a), len(hash_b))}
$$

### 7.3 Tolerance Rule

- restore_snapshot MUST fail if drift > max_normalized_hash_distance.
- Drift failure MUST taint and emit failure telemetry.

## 8. Reset FSM Integration

reset_environment flow is updated as follows:

1. Attempt restore from latest snapshot pointer.
2. If fast restore succeeds, return success.
3. If no snapshot exists, continue hard reset path.
4. If restore fails, keep taint asserted and continue hard reset fallback.

This enforces:

- fast-path first
- hard reset fallback only when restore is unavailable or fails

## 9. Baseline Snapshot Refresh Policy

- ensure_ready SHOULD create baseline snapshot when none exists.
- Successful hard reset SHOULD refresh baseline snapshot best-effort.
- Snapshot refresh errors are non-fatal but logged.

## 10. Telemetry Packet Specification

Snapshot subsystem MUST emit structured telemetry packets with:

- timestamp_unix_ms
- event
- snapshot_id
- target_instance_id
- digest_match
- restore_latency_ms
- drift_distance
- hard_reset_fallback
- details

Required events:

- snapshot_created
- snapshot_restore_skipped
- snapshot_restore_succeeded
- snapshot_restore_integrity_failed
- snapshot_restore_drift_failed

## 11. Contract Preservation

- CommandExecutor/FileSystemOps interfaces remain unchanged.
- Snapshot methods augment provider capabilities without altering RFC-001 call signatures.

## 12. Rollout Plan

- Phase A: snapshot metadata and integrity verification enabled.
- Phase B: reset fast-path restore enabled with hard reset fallback.
- Phase C: drift thresholds tuned with adversarial validation.

## 13. Open Questions

- Snapshot cadence under high churn and low storage conditions.
- Optional signature chain for snapshot provenance.
- Per-target tolerance profiles for drift thresholds.

## 14. Checklist

- [x] Finalize snapshot persistence format and backward compatibility.
- [x] Add integration tests for digest mismatch fail-closed taint.
- [x] Add reset FSM tests for restore-fast-path and fallback branch.
- [x] Add chaos scenarios for corrupted payloads and drift regressions.
- [x] Document operator runbook for snapshot restore failures.
