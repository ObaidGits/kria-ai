# KRIA HITL Runtime Contracts

**Document status:** Implementation-bound contract specification
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/collaborative_decision.rs`, `execution_gate.rs`, `resume_executor.rs`, `resource_lease.rs`

---

## 1. Canonical IDs

```rust
type WorkflowId = String;
type AttemptId = String;
type StageId = String;
type DecisionId = String;
type OptionId = String;
type ActionHash = String;
type TargetHash = String;
type CheckpointId = String;
type ActionId = String;
```

IDs are opaque. Runtime behavior must come from typed fields and policy/verifier outputs, not encoded ID meaning.

---

## 2. ActionProposal

Current shape:

```rust
struct ActionProposal {
    workflow_id: WorkflowId,
    attempt_id: AttemptId,
    stage_id: StageId,
    tool_name: String,
    parameters: serde_json::Value,
    target: TargetBinding,
    tool_schema_version: String,
    tool_registry_version: String,
    action_hash: ActionHash,
    target_hash: TargetHash,
    requested_by: Actor,
    created_at: String,
}
```

`action_hash` and `target_hash` are immutable. Any changed parameter, target binding, stage, tool schema version, or registry version creates a different proposal.

---

## 3. TargetBinding

Current shape:

```rust
struct TargetBinding {
    kind: String,
    id: String,
    workspace_id: Option<String>,
    session_id: Option<String>,
    execution_boundary: Option<String>,
    metadata: serde_json::Value,
}
```

The execution gate builds this from `execution_authority::ValidationResult`. Ambiguous or blocked authority results still produce a target binding so the decision can explain what was unsafe or unresolved.

---

## 4. InteractionDecision

Current shape:

```rust
struct InteractionDecision {
    id: String,
    workflow_id: String,
    attempt_id: String,
    stage_id: Option<String>,
    action_hash: String,
    target_hash: String,
    action_proposal: Option<ActionProposal>,
    decision_type: DecisionType,
    status: DecisionStatus,
    version: u64,
    reason: String,
    risk_level: RiskLevel,
    options: Vec<DecisionOption>,
    recommended_option: Option<String>,
    rollbackability: Rollbackability,
    confidence: ConfidenceBand,
    affected_resources: Vec<String>,
    rule_id: Option<String>,
    evidence: Vec<EvidenceSummary>,
    invalidation_rules: Vec<String>,
    created_at: String,
    updated_at: String,
    expires_at: Option<String>,
    resolution: Option<String>,
    execution: Option<DecisionExecutionRecord>,
    stage_binding: Option<DecisionStageBinding>,
    checkpoint_summary: Option<CheckpointSummary>,
    continuation: Option<ContinuationClaim>,
    verification: Option<PostDecisionVerification>,
}
```

Resolution must be scoped by decision ID and version. Hash-scoped resolution uses `DecisionResolutionContext` with `expected_action_hash` and `expected_target_hash`.

---

## 5. Decision Types And Statuses

```rust
enum DecisionType {
    Approval,
    TargetSelection,
    ScopeClarification,
    RecoveryChoice,
    CredentialRequired,
    VerifierConflict,
    UnsafeUncertainty,
}

enum DecisionStatus {
    Pending,
    Resolved,
    Deferred,
    Expired,
    Invalidated,
    Denied,
    Cancelled,
}
```

Only `Pending` decisions can be resolved, denied, cancelled, expired, or invalidated by the transition helpers.

---

## 6. Gate Outcomes

The live pre-tool gate returns:

```rust
enum ExecutionGateOutcome {
    Proceed,
    Block { reason: String },
    PauseForDecision { decision_id: String, decision_type: &'static str, reason: String },
    RequiresApproval { decision: InteractionDecision },
}
```

Resume uses a separate result:

```rust
enum ResumeGateOutcome {
    Ready,
    MissingActionProposal,
    StaleActionProposal { reason: String },
    Block { reason: String },
    RiskIncreased { previous: RiskLevel, current: RiskLevel, reason: String },
    RequiresApproval { risk_level: RiskLevel, reason: String },
}
```

`NeedLease` and `NeedReobserve` exist only in the generic collaborative-decision vocabulary. They are not live `ExecutionGateOutcome` variants. Live resource ownership is represented by `resource_requirements` returned alongside the gate outcome.

---

## 7. Runtime Components

| Component | Contract |
|---|---|
| `ExecutionGate` | Runs readiness, preflight, execution authority, policy, decision creation, and resource declaration. It does not execute tools. |
| `DecisionStore` | Persists and replays decision, execution, continuation, and evidence events. It rejects stale version/hash/status transitions. |
| `ResourceLeaseManager` | Acquires scoped leases and releases them through `ResourceLeaseGuard`. |
| `ResumeExecutor` | Executes exactly one resolved persisted action after context, version, hash, gate, grounding, tool-version, tool-capability, and lease checks. |
| `ContinuationReentryService` | Verifies one previously executed decision-bound action and records action-level progress only. |
| `AuditLogger` | Writes SQLite policy/HITL action decisions with hash-chain verification. |

---

## 8. Serialization Rules

- `DecisionStore` persistence is append-only JSONL and replay-derived state.
- Unknown or missing critical fields must fail closed at transition or resume boundaries.
- `action_hash`, `target_hash`, tool schema version, and tool registry version are resume-critical.
- Decision events include `policy_version` and `runtime_version`.
- SQLite audit rows are hash-chained and queryable by session, risk, and action.
- Tool result stored in decision execution records must be redacted.
