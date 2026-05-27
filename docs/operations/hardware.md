# KRIA Hardware Orchestration

## 1. Purpose

Hardware orchestration coordinates compute resources for deterministic model/runtime execution under load. It manages GPU leasing, degradation, and recovery so orchestrator behavior remains bounded.

Responsibilities:
- Track runtime hardware state and pressure.
- Allocate/lease GPU resources safely.
- Enforce degradation/recovery state transitions.
- Feed hardware constraints into provider/orchestrator decisions.

Non-goals:
- Hardware subsystem does not choose user-intent execution strategy.
- Hardware subsystem is not a policy authority for dangerous actions.

## 2. Architecture Overview

Primary implementation:
- `crates/kria-core/src/resource/gpu_lease.rs`
- `crates/kria-core/src/llm/orchestrator/mod.rs`
- `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs`

Architecture:
1. Runtime telemetry reports utilization/health.
2. Lease manager controls access and ownership state.
3. Watchdog applies hysteresis/EMA rules for stable transitions.
4. Orchestrator/provider routing adapts to hardware state.

## 3. Runtime Execution Flow

1. Turn/resource planning identifies compute requirements.
2. Lease acquisition attempts reserve required GPU capacity.
3. Execution proceeds if lease and policy constraints are satisfied.
4. Watchdog monitors pressure and can trigger recover/degrade transitions.
5. Lease release occurs on completion/cancel/failure.

Authority boundaries:
- Orchestrator decides workload routing using hardware signals.
- Hardware layer exposes state and lease controls, not user-task authority.

## 4. Core Components

| Component | Location | Contract |
|---|---|---|
| `GpuLeaseManager` | `resource/gpu_lease.rs` | Lease acquisition/release and state transitions |
| Lease states | `gpu_lease.rs` | `Idle`, `Held`, `Recovering`, `Degraded` semantics |
| GPU watchdog | `llm/orchestrator/gpu_watchdog.rs` | Telemetry-driven recover/degrade control with hysteresis |
| Orchestrator integration | `llm/orchestrator/mod.rs` | Uses hardware state for routing behavior |

Invariants:
- No lease, no GPU-bound execution.
- State transitions must be explicit and observable.
- Recovery behavior avoids thrashing through rate limits/hysteresis.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | Hardware state informs bounded routing decisions |
| Providers | Local backends require lease-compatible capacity |
| Tools | Hardware-heavy tools must honor runtime resource limits |
| Memory | Memory retrieval/persistence can be degraded under pressure |
| OpenClaw/n8n/MCP | External substrate use can reduce local hardware pressure |
| Safety | High-impact system operations remain safety-gated |
| GUI/Browser/Voice | UI/voice responsiveness may trigger conservative hardware routing |

## 6. Failure Handling & Recovery

- Lease denial: route to compatible non-GPU path when possible.
- GPU pressure spike: transition to degraded mode and reduce local workload.
- Device fault: isolate failing path and prefer remote/provider fallback.
- Recovery path: re-enable capacity only after stable telemetry window.

Recovery rule:
- Deterministic degrade/recover behavior is preferred over aggressive oscillation.

## 7. Performance & Constraints

Constraints:
- VRAM limits bound model class and concurrency.
- Thermal throttling and memory pressure affect latency.
- Lease contention can increase queueing delay.

Tradeoffs:
- Conservative leasing improves stability but may reduce throughput.
- Aggressive concurrency improves throughput but increases failure risk.

## 8. Security & Safety

Trust boundaries:
- Device/driver state is operationally sensitive but not policy authority.

Controls:
- Hardware-triggered fallback must still pass orchestration and safety rules.
- System-level operations remain gated for risky changes.
- Resource exhaustion handling prevents unsafe uncontrolled degradation.

## 9. Observability

Capture:
- Lease grant/deny rates and wait times.
- GPU utilization, memory pressure, and thermal indicators.
- Degraded/recovering dwell time and transition frequency.
- Turn latency impact by hardware state.

Evaluation:
- Hardware stress scenarios and regression checks are tracked through `docs/evaluations/overview.md`.

## 10. Future Evolution

1. Improve multi-device lease fairness and placement strategy.
2. Add richer SLO thresholds for degrade/recover triggers.
3. Enhance provider-routing policies with explicit hardware budgets.
4. Keep hardware as bounded signal/control plane, not orchestration authority.
