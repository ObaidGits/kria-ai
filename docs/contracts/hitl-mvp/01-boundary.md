# KRIA HITL MVP Boundary

**Document status:** Implementation-bound scope contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/collaborative_decision.rs`, `execution_gate.rs`, `resume_executor.rs`, `resource_lease.rs`, `continuation_reentry.rs`, `gui_wiring.rs`, `crates/kria-core/src/safety/audit.rs`

---

## 1. Goal

KRIA HITL MVP exists to do one bounded job:

```text
pause unsafe or underdetermined side effects,
persist the exact action and target being paused,
accept only a fresh scoped decision,
revalidate before execution,
lease shared resources before side effects,
and leave enough audit/decision evidence to explain what happened.
```

This layer is not a scheduler, replay engine, trust model, semantic OS, or adaptive autonomy system.

---

## 2. In Scope

| Capability | Current implementation requirement |
|---|---|
| Durable decision envelope | `InteractionDecision` is persisted through `DecisionStore`; persistent default is `.kria/decisions/decision_events.jsonl`. |
| Immutable action intent | `ActionProposal` binds `workflow_id`, `attempt_id`, `stage_id`, `tool_name`, normalized JSON parameters, target, tool schema version, registry version, `action_hash`, and `target_hash`. |
| Target binding | `TargetBinding` records kind, id, optional workspace/session/execution boundary, and metadata. |
| Deterministic execution gate | Live tool execution uses `ExecutionGateOutcome::{Proceed, Block, PauseForDecision, RequiresApproval}`. |
| Authority ambiguity pause | Execution-authority ambiguity creates a durable target-selection decision and returns `PauseForDecision`. |
| Policy approval | Policy-required approval creates a durable approval decision and returns `RequiresApproval`. |
| Resume validation | `ResumeExecutor` validates decision version, action hash, target hash, tool versions, gate result, grounding, and leases before one-step execution. |
| Minimal leases | `ResourceLeaseManager` handles GUI foreground/input, filesystem path, browser profile, and VM target requirements declared by `ExecutionGate`. |
| Bounded continuation | `ContinuationReentryService` verifies one executed decision-bound action and stops; it does not replay a whole workflow. |
| Audit | `AuditLogger` records policy/HITL execution decisions in SQLite with a hash chain; `DecisionStore` records decision lifecycle/execution/continuation events in JSONL. |
| Tests | Unit and integration tests must cover stale version/hash rejection, policy blocks, lease conflicts, resume gate blocking, unsupported resume tools, and decision replay from JSONL. |

---

## 3. Explicitly Out Of MVP

Do not add these to HITL MVP scope:

- semantic scheduler,
- cognitive pressure scoring,
- human trust model,
- adaptive urgency AI,
- autonomy drift scoring,
- substrate trust decay,
- planner outcome scoring,
- global causality graph,
- deterministic GUI replay,
- external delegated HITL bridging,
- mobile/remote/voice approval expansion,
- LLM-generated recovery sessions,
- metrics-driven safety tuning,
- preference learning from HITL answers,
- multi-user collaboration model,
- full replay UI.

Generic enum values such as `GpuModel`, `VerifierSlot`, or `DelegatedWorkflow` may exist in shared lease vocabulary, but HITL MVP must not build new scheduling systems around them.

---

## 4. Runtime Authority Order

Authority order is represented by `AuthorityLevel` and must remain monotonic:

```text
PolicyBlock
PolicyRisk
VerifierTruth
ExecutionAuthority
RecoveryFeasibility
WorkflowSemantics
UserInstruction
Preference
PlannerRecommendation
ModelSuggestion
```

Lower authority may provide context or wording. It must not reduce risk, bypass policy, override verifier truth, or mutate persisted action/target hashes.

---

## 5. Completion Criteria

The HITL MVP boundary is satisfied only when:

- side-effecting live tools pass through `ExecutionGate`,
- policy `Black` decisions block even if a user approves,
- policy `Red` or approval-required actions cannot execute without a fresh approved decision,
- stale decision versions are rejected,
- action hash mutation rejects resolution or resume,
- target hash mutation rejects resolution or resume,
- expired decisions invalidate instead of executing,
- resume executes exactly one persisted `ActionProposal`,
- unsupported tools cannot be resumed from the action center,
- GUI/input/filesystem/browser/VM resource conflicts block execution,
- continuation re-entry verifies one previous action and stops,
- audit and decision events explain pause, resolution, execution, invalidation, denial, timeout, or lease conflict.
