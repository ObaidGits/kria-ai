# KRIA Deployment Architecture

## 1. Purpose

This document defines production deployment architecture and operating boundaries for KRIA as an Execution Intelligence Platform.

Responsibilities:
- Define deployment topology and runtime packaging.
- Preserve orchestration authority and safety behavior across environments.
- Standardize configuration, rollout, and recovery practices.

Non-goals:
- This is not a step-by-step beginner install guide.
- Deployment does not redefine subsystem logic contracts.

## 2. Architecture Overview

Primary deployment surfaces:
- Core runtime binaries (Rust crates/workspace outputs).
- Containerized deployment artifacts (`Dockerfile*`, compose files).
- Configuration files (`kria_config.toml`, env/config overlays).

Deployment model:
1. Build immutable runtime artifact.
2. Inject environment-specific configuration/credentials.
3. Start runtime with controlled integration endpoints.
4. Monitor health, safety events, and orchestration behavior.

## 3. Runtime Execution Flow

1. Runtime boots and initializes core subsystems (providers, memory, safety, integrations).
2. Integration adapters are mounted according to config.
3. Requests/turns execute through orchestration loop with policy gates.
4. Runtime exports logs/telemetry for operations and incident response.
5. Graceful shutdown preserves durable state and in-flight safety semantics.

Authority boundaries:
- Deployment platform hosts KRIA; it does not replace runtime orchestration authority.
- External substrates remain controlled through KRIA execution gates.

## 4. Core Components

| Component | Contract |
|---|---|
| Build/release scripts (`scripts/*`) | Produce consistent deployable artifacts |
| Container definitions (`Dockerfile*`, compose) | Standardized runtime environment provisioning |
| Runtime config (`kria_config.toml` + env) | Controlled feature/provider/integration wiring |
| Persistent stores (memory/audit DB) | Durable state and governance data retention |

Invariants:
- Same safety and authority gates apply in all environments.
- Deployments must avoid undocumented config drift.
- Artifact provenance and config changes are traceable.

## 5. Integration Contracts

| Integration | Deployment contract |
|---|---|
| Orchestration | Runtime authority remains inside KRIA core |
| Providers | Endpoint/credentials supplied by env config, not hardcoded |
| Tools | Dangerous tool capabilities remain policy-gated |
| Memory | Persistent storage availability and migration integrity required |
| OpenClaw/n8n/MCP | External substrate endpoints configured explicitly and monitored |
| Hardware | Device/runtime limits reflected in deployment resource settings |
| Safety | HITL/audit/policy stores must be reachable and durable |
| GUI/Browser/Voice | Optional surfaces enabled per environment capability |

## 6. Failure Handling & Recovery

- Startup failure: fail fast with explicit subsystem readiness errors.
- Integration outage: degrade to available substrates/providers under policy.
- Resource exhaustion: trigger hardware/provider fallback pathways.
- Data store issue: preserve safe degraded runtime where possible; block unsafe operations.

Recovery:
- Use bounded restart/retry strategies and explicit health checks.
- Prefer controlled rollback to unknown partial state.

## 7. Performance & Constraints

Constraints:
- Provider latency and network stability affect turn SLOs.
- Hardware capacity (CPU/GPU/VRAM) shapes local model viability.
- Persistent storage performance affects memory/safety audit throughput.

Tradeoff:
- Higher resilience and observability often increase infrastructure cost/complexity.

## 8. Security & Safety

Controls:
- Principle of least privilege for runtime credentials and system access.
- Protect integration secrets and rotate per operational policy.
- Preserve policy/HITL/audit controls in production; no bypass modes.

Trust boundaries:
- External providers and substrates are untrusted networks/services.
- Runtime control plane remains authoritative and isolated.

## 9. Observability

Required operations telemetry:
- Service health/readiness/liveness.
- Turn latency, tool failure, provider fallback, and safety decision metrics.
- Audit-log integrity and write health.
- Integration endpoint error rates and saturation signals.

Evaluation:
- Deployment validation should include subsystem smoke checks and eval regressions from `docs/evaluations/overview.md`.

## 10. Future Evolution

1. Strengthen environment profiles for local/dev/staging/production parity.
2. Improve rollout safety with stricter automated health gates.
3. Expand integration-specific SLO dashboards.
4. Keep deployment architecture anchored to deterministic runtime governance.
