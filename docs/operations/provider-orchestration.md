# KRIA Provider Orchestration

## 1. Purpose

This subsystem manages model providers as execution backends under orchestrator control. It standardizes provider lifecycle, routing, streaming, and failover behavior.

Responsibilities:
- Initialize and register providers from config/runtime (`ProviderRegistry`).
- Route requests by capability and policy constraints.
- Manage backend health and execution-location notifications.
- Normalize streaming semantics for orchestrator consumers.

Non-goals:
- Providers do not own orchestration policy or tool authority.
- Backend-specific APIs must not leak into orchestration contracts.

## 2. Architecture Overview

Primary implementation:
- `crates/kria-core/src/llm/provider/registry.rs`
- `crates/kria-core/src/llm/provider/streaming.rs`
- `crates/kria-core/src/llm/provider/backends/*`
- `crates/kria-core/src/agent/turn_gate/mod.rs`

Architecture:
1. TurnGate derives `ResourcePlan`/`IntentEnvelope`.
2. Provider orchestration selects backend by capability and constraints.
3. `ProviderRegistry` executes request and exposes health/snapshots.
4. `UnifiedStream` normalizes event flow for downstream handling.

## 3. Runtime Execution Flow

1. Request enters orchestrator with turn context and policy envelope.
2. Registry resolves backend candidate(s) from mounted providers + policy.
3. Backend call starts; location callback can update orchestrator execution view.
4. Streamed or non-streamed output is normalized for loop consumption.
5. Errors trigger fallback selection (if eligible) or fail-fast return.

Authority boundaries:
- Orchestrator decides when/why provider calls occur.
- Registry executes backend calls, not policy overrides.
- Safety gates remain above provider layer.
- Tool output synthesis is handled in the tool execution path, not the provider layer.

## 4. Core Components

| Component | Location | Contract |
|---|---|---|
| `ProviderRegistry` | `llm/provider/registry.rs` | Provider lifecycle, routing, execution dispatch |
| Backend adapters | `llm/provider/backends/*` | Provider-specific API bridging to shared traits |
| `UnifiedStream` | `llm/provider/streaming.rs` | Consistent streaming event surface |
| Health/snapshots | `registry.rs` | Runtime provider availability and diagnostics |

Invariants:
- Provider selection must honor capability and policy constraints.
- Registry state transitions are explicit (init/register/select/execute).
- Streaming and non-streaming calls map to coherent output contracts.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | Provider calls are initiated by orchestrator turn flow |
| Tools | Tool-calling models surface tool intents; tool execution remains separate |
| Memory | Provider outputs can be persisted only through orchestrator memory flow |
| OpenClaw/n8n/MCP | Provider subsystem does not directly invoke substrates |
| Hardware | Local backends must respect device/VRAM constraints from orchestrator |
| Safety | Policy/HITL decisions constrain provider use for risky flows |
| GUI/Browser | Provider outputs may drive automation intent, but never direct automation authority |

## 6. Failure Handling & Recovery

- Backend timeout/error: classify transient vs hard failure.
- Fallback routing: use alternative compatible backend when policy allows.
- Degraded mode: continue with bounded capability downgrade (for example, reduced context/model class).
- Hard block: if no safe/compatible backend exists, fail with explicit reason.

Recovery rules:
- Preserve deterministic selection order for reproducibility.
- Avoid repeated oscillation across unhealthy backends.
- Keep fallback decisions visible in diagnostics.

## 7. Performance & Constraints

Key constraints:
- Token/context limits differ per provider.
- Stream latency and startup overhead vary by backend.
- Local models are constrained by VRAM and device contention.
- Remote providers add network variability and quota limits.

Operational tradeoff:
- Aggressive fallback improves availability but can reduce output quality consistency.

## 8. Security & Safety

Trust boundaries:
- Remote providers are external trust domains.
- Local providers still require policy governance for downstream actions.

Controls:
- Provider credentials are config-managed and not orchestration authority artifacts.
- Sensitive prompts/results should follow safety and audit pathways.
- Dangerous execution is never delegated to provider layer.

## 9. Observability

Capture:
- Per-provider latency, error rate, timeout rate.
- Fallback frequency and fallback target.
- Streaming interruption/partial completion signals.
- Health snapshot transitions and backend disable events.

Evaluation:
- Use `docs/evaluations/overview.md` for provider routing and regression coverage.

## 10. Future Evolution

1. Stronger provider capability typing (context/tool/vision/reasoning classes).
2. More explicit local-vs-remote cost/latency policy envelopes.
3. Deterministic fallback policy profiles by intent class.
4. Better per-provider SLO instrumentation.
