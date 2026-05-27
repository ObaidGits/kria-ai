# KRIA Runtime Contracts

**Document status:** MVP contract specification  
**Purpose:** Define the structs, enums, and narrow traits required for HITL MVP.  
**Rule:** Runtime contracts must be deterministic, serializable, versioned, and testable.

---

## 1. Core Types

```rust
type WorkflowId = String;
type AttemptId = String;
type StageId = String;
type DecisionId = String;
type OptionId = String;
type LeaseId = String;
type EvidenceId = String;
type AuditId = String;
type ActionHash = String;
type TargetHash = String;
```

IDs must be opaque. Do not encode behavior in IDs.

---

## 2. ActionProposal

```rust
struct ActionProposal {
    workflow_id: WorkflowId,
    attempt_id: AttemptId,
    stage_id: StageId,
    tool_name: String,
    parameters: serde_json::Value,
    target: TargetBinding,
    action_hash: ActionHash,
    target_hash: TargetHash,
    requested_by: Actor,
    created_at: DateTime<Utc>,
}
```

`action_hash` and `target_hash` are immutable. Any parameter or target mutation creates a new proposal.

---

## 3. InteractionDecision

```rust
struct InteractionDecision {
    id: DecisionId,
    workflow_id: WorkflowId,
    attempt_id: AttemptId,
    stage_id: StageId,
    action_hash: ActionHash,
    target_hash: TargetHash,
    decision_type: DecisionType,
    risk_class: RiskClass,
    rollbackability: Rollbackability,
    status: DecisionStatus,
    version: u64,
    options: Vec<DecisionOption>,
    evidence_refs: Vec<EvidenceId>,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    invalidation: InvalidationRules,
}
```

Resolution must include `id`, `version`, and `option_id`.

---

## 4. DecisionOption

```rust
struct DecisionOption {
    id: OptionId,
    label: String,
    effect: DecisionEffect,
    risk_delta: RiskDelta,
    requires_revalidation: bool,
}
```

Options must be executable by deterministic backend code. LLM wording may describe options but must not create authority.

---

## 5. GateOutcome

```rust
enum GateOutcome {
    Proceed { lease_requirements: Vec<LeaseRequirement> },
    Block { reason: BlockReason },
    PauseForDecision { decision: InteractionDecision },
    NeedReobserve { reason: String },
    NeedLease { requirements: Vec<LeaseRequirement> },
}
```

No other MVP gate outcomes are allowed.

---

## 6. Narrow Traits

```rust
trait PolicyEngine {
    fn classify(&self, action: &ActionProposal) -> PolicyDecision;
}

trait ExecutionAuthority {
    fn bind_target(&self, action: &ActionProposal) -> TargetAuthorityResult;
}

trait DecisionStore {
    fn create(&self, decision: InteractionDecision) -> Result<DecisionId>;
    fn resolve(&self, id: &DecisionId, version: u64, option: &OptionId) -> Result<ResolvedDecision>;
    fn invalidate(&self, id: &DecisionId, reason: InvalidationReason) -> Result<()>;
}

trait AuditSink {
    fn record(&self, event: AuditRecord) -> Result<AuditId>;
}
```

Do not expose a plugin-style `DecisionProducer` ecosystem in MVP.

---

## 7. Serialization Rules

- All persisted structs must include schema version.
- Unknown enum variants must fail closed.
- Decision resolution must be idempotent.
- Audit append failure must stop side-effecting execution.
- Deserialization must reject missing `action_hash` or `target_hash`.

